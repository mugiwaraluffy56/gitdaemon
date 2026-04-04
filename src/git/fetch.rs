//! Background fetch from all configured remotes.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Summary returned after a fetch cycle.
#[derive(Debug, Clone, Default)]
pub struct FetchSummary {
    /// Total number of refs updated across all remotes.
    pub refs_updated: usize,
    /// Names of remotes that were successfully fetched.
    pub remotes_fetched: Vec<String>,
    /// Names of remotes that failed, with the error message.
    pub errors: Vec<(String, String)>,
}

/// Fetch from all remotes of the repository at `repo_root`.
///
/// Errors from individual remotes are collected but do not abort the overall
/// operation — the caller sees per-remote failures in `FetchSummary::errors`.
pub async fn fetch_all_remotes(repo_root: PathBuf) -> Result<FetchSummary> {
    tokio::task::spawn_blocking(move || fetch_all_remotes_sync(&repo_root))
        .await
        .context("spawn_blocking join error")?
}

fn fetch_all_remotes_sync(repo_root: &Path) -> Result<FetchSummary> {
    let repo = git2::Repository::open(repo_root)
        .with_context(|| format!("failed to open repository at {}", repo_root.display()))?;

    let remote_arr = repo.remotes().context("failed to list remotes")?;
    let remote_names: Vec<String> = remote_arr.iter().flatten().map(|s| s.to_string()).collect();

    if remote_names.is_empty() {
        debug!("no remotes configured — skipping fetch");
        return Ok(FetchSummary::default());
    }

    let mut summary = FetchSummary::default();

    for name in &remote_names {
        match fetch_one_remote(&repo, name) {
            Ok(n) => {
                info!(remote = %name, refs_updated = n, "fetched");
                summary.refs_updated += n;
                summary.remotes_fetched.push(name.clone());
            }
            Err(e) => {
                warn!(remote = %name, error = %e, "fetch failed");
                summary.errors.push((name.clone(), e.to_string()));
            }
        }
    }

    Ok(summary)
}

fn fetch_one_remote(repo: &git2::Repository, remote_name: &str) -> Result<usize> {
    let mut remote = repo
        .find_remote(remote_name)
        .with_context(|| format!("remote '{}' not found", remote_name))?;

    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(make_credential_cb());
    callbacks.transfer_progress(|stats| {
        debug!(
            received = stats.received_objects(),
            total = stats.total_objects(),
            "fetch progress"
        );
        true
    });

    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts
        .remote_callbacks(callbacks)
        .download_tags(git2::AutotagOption::Unspecified)
        .prune(git2::FetchPrune::Unspecified);

    let before_count = count_remote_refs(repo, remote_name);

    remote
        .fetch(&[] as &[&str], Some(&mut fetch_opts), None)
        .with_context(|| format!("fetch from '{}' failed", remote_name))?;

    let after_count = count_remote_refs(repo, remote_name);
    Ok(after_count.saturating_sub(before_count))
}

fn count_remote_refs(repo: &git2::Repository, remote_name: &str) -> usize {
    repo.references()
        .map(|refs| {
            refs.filter_map(|r| r.ok())
                .filter(|r| {
                    r.name()
                        .map(|n| n.starts_with(&format!("refs/remotes/{}/", remote_name)))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

/// Build a reusable credential callback that tries:
/// 1. SSH agent
/// 2. Common key files (~/.ssh/id_ed25519, id_rsa, id_ecdsa)
/// 3. Git credential helper (for HTTPS)
fn make_credential_cb(
) -> impl FnMut(&str, Option<&str>, git2::CredentialType) -> Result<git2::Cred, git2::Error> {
    let mut tried_ssh_agent = false;
    let mut ssh_key_idx: usize = 0;
    let ssh_key_names = ["id_ed25519", "id_rsa", "id_ecdsa"];

    move |url, username, allowed| {
        // Username challenge — return "git"
        if allowed.contains(git2::CredentialType::USERNAME) {
            return git2::Cred::username(username.unwrap_or("git"));
        }

        if allowed.contains(git2::CredentialType::SSH_KEY) {
            // Try ssh-agent once
            if !tried_ssh_agent {
                tried_ssh_agent = true;
                if let Ok(cred) =
                    git2::Cred::ssh_key_from_agent(username.unwrap_or("git"))
                {
                    return Ok(cred);
                }
            }
            // Try common key files in turn
            let home = dirs::home_dir().unwrap_or_default();
            while ssh_key_idx < ssh_key_names.len() {
                let key_path = home.join(".ssh").join(ssh_key_names[ssh_key_idx]);
                ssh_key_idx += 1;
                if key_path.exists() {
                    if let Ok(cred) = git2::Cred::ssh_key(
                        username.unwrap_or("git"),
                        None,
                        &key_path,
                        None,
                    ) {
                        return Ok(cred);
                    }
                }
            }
        }

        if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
            if let Ok(cfg) = git2::Config::open_default() {
                if let Ok(cred) = git2::Cred::credential_helper(&cfg, url, username) {
                    return Ok(cred);
                }
            }
        }

        Err(git2::Error::from_str("no suitable credentials found"))
    }
}

