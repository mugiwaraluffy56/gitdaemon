//! Batched push queue with secret scanning and conflict detection.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::git::secrets;

const MAX_CONSECUTIVE_FAILURES: u32 = 3;

// ============================================================================
// Types
// ============================================================================

/// Result of a push attempt.
#[derive(Debug, Clone)]
pub struct PushResult {
    pub pushed_commits: usize,
    pub pushed_at: DateTime<Utc>,
    pub success: bool,
    pub error: Option<String>,
    pub blocked_by_secrets: bool,
    pub secret_hits: Vec<secrets::SecretHit>,
}

/// Reason a push was skipped without being attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushSkipReason {
    Paused,
    NothingToPush,
    ConflictDetected,
    SecretBlocked,
}

#[derive(Debug, Clone)]
pub struct PushState {
    pub paused: bool,
    pub conflict_detected: bool,
    pub last_push: Option<DateTime<Utc>>,
    pub queued_commits: usize,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

/// Thread-safe push queue used by the sync loop.
#[derive(Debug)]
pub struct PushQueue {
    repo_root: PathBuf,
    state: Arc<Mutex<PushState>>,
    force_notify: Arc<Notify>,
}

impl PushQueue {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            state: Arc::new(Mutex::new(PushState {
                paused: false,
                conflict_detected: false,
                last_push: None,
                queued_commits: 0,
                last_error: None,
                consecutive_failures: 0,
            })),
            force_notify: Arc::new(Notify::new()),
        }
    }

    /// Record that new commits are pending push.
    pub fn record_commits(&self, n: usize) {
        self.state.lock().unwrap().queued_commits += n;
    }

    pub fn pause(&self) {
        self.state.lock().unwrap().paused = true;
    }

    pub fn resume(&self) {
        let mut s = self.state.lock().unwrap();
        s.paused = false;
        s.consecutive_failures = 0;
    }

    /// Trigger an immediate push attempt regardless of the timer.
    pub fn push_now(&self) {
        self.force_notify.notify_one();
    }

    /// Returns a clone of the current push state for status reporting.
    pub fn state_snapshot(&self) -> PushState {
        self.state.lock().unwrap().clone()
    }

    /// Reference to the force-push notification handle (for `tokio::select!`).
    pub fn force_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.force_notify)
    }

    // ──────────────────────────────────────────────────────────────────────────
    // Core push logic
    // ──────────────────────────────────────────────────────────────────────────

    /// Attempt a push if conditions allow it.
    ///
    /// Returns `None` when skipped (paused, nothing to push, or conflict).
    /// Returns `Some(PushResult)` on both success and failure.
    pub async fn try_push(&self, cfg: &Config) -> Result<Option<PushResult>> {
        let snap = self.state_snapshot();

        if snap.paused {
            return Ok(None);
        }
        if snap.conflict_detected {
            return Ok(None);
        }

        // Branch-push guard: refuse to auto-push to protected branches
        let current_branch = {
            let root = self.repo_root.clone();
            tokio::task::spawn_blocking(move || {
                git2::Repository::open(&root)
                    .ok()
                    .and_then(|r| {
                        r.head()
                            .ok()
                            .and_then(|h| h.shorthand().map(|s| s.to_string()))
                    })
            })
            .await
            .unwrap_or(None)
        };
        if let Some(ref cb) = current_branch {
            if cfg.push.protected_branches.iter().any(|p| p == cb) {
                warn!(
                    branch = %cb,
                    "auto-push skipped: '{}' is a protected branch — use `fg push` to push manually",
                    cb
                );
                return Ok(None);
            }
        }

        // Check if there are commits to push (queued or ahead of remote)
        let branch = cfg.push.branch.clone();
        let repo_root = self.repo_root.clone();
        let has_work = if snap.queued_commits > 0 {
            true
        } else {
            let branch_c = branch.clone();
            let root_c = repo_root.clone();
            tokio::task::spawn_blocking(move || check_ahead_count(&root_c, &branch_c))
                .await
                .context("spawn_blocking join error")??
                > 0
        };

        if !has_work {
            return Ok(None);
        }

        // Secret scanning on the staged diff
        if cfg.safety.block_secrets {
            let root_c = repo_root.clone();
            let branch_c = branch.clone();
            let hits = tokio::task::spawn_blocking(move || scan_diff_for_secrets(&root_c, &branch_c))
                .await
                .context("spawn_blocking join error")??;

            if !hits.is_empty() {
                error!(?hits, "push blocked — secrets detected in diff");
                return Ok(Some(PushResult {
                    pushed_commits: 0,
                    pushed_at: Utc::now(),
                    success: false,
                    error: Some("secrets detected in diff".to_string()),
                    blocked_by_secrets: true,
                    secret_hits: hits,
                }));
            }
        }

        // Perform the push
        let root_c = repo_root.clone();
        let branch_c = branch.clone();
        match tokio::task::spawn_blocking(move || push_to_remote(&root_c, &branch_c))
            .await
            .context("spawn_blocking join error")?
        {
            Ok(()) => {
                let now = Utc::now();
                let queued = {
                    let mut s = self.state.lock().unwrap();
                    let q = s.queued_commits;
                    s.queued_commits = 0;
                    s.last_push = Some(now);
                    s.last_error = None;
                    s.consecutive_failures = 0;
                    s.conflict_detected = false;
                    q
                };
                info!(branch = %branch, commits = queued, "pushed");
                Ok(Some(PushResult {
                    pushed_commits: queued,
                    pushed_at: now,
                    success: true,
                    error: None,
                    blocked_by_secrets: false,
                    secret_hits: vec![],
                }))
            }
            Err(e) => {
                warn!(error = %e, "push failed");
                let err_str = e.to_string();
                let mut s = self.state.lock().unwrap();
                s.last_error = Some(err_str.clone());
                s.consecutive_failures += 1;
                if s.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    error!(
                        consecutive_failures = s.consecutive_failures,
                        "max push failures reached — pausing"
                    );
                    s.paused = true;
                }
                Ok(Some(PushResult {
                    pushed_commits: 0,
                    pushed_at: Utc::now(),
                    success: false,
                    error: Some(err_str),
                    blocked_by_secrets: false,
                    secret_hits: vec![],
                }))
            }
        }
    }

    /// Check for merge conflicts and update the state accordingly.
    ///
    /// Returns `true` when a conflict is detected.
    pub async fn check_and_record_conflict(&self) -> bool {
        let root = self.repo_root.clone();
        let has_conflict = tokio::task::spawn_blocking(move || {
            git2::Repository::open(&root)
                .map(|r| r.state() == git2::RepositoryState::Merge)
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false);

        let mut s = self.state.lock().unwrap();
        if has_conflict && !s.conflict_detected {
            warn!("merge conflict detected — push paused");
            s.conflict_detected = true;
        } else if !has_conflict && s.conflict_detected {
            info!("conflict resolved — push resumed");
            s.conflict_detected = false;
        }
        has_conflict
    }
}

// ============================================================================
// Synchronous git helpers (run inside spawn_blocking)
// ============================================================================

fn push_to_remote(repo_root: &std::path::Path, branch: &str) -> Result<()> {
    let repo = git2::Repository::open(repo_root).context("failed to open repository")?;
    let mut remote = repo.find_remote("origin").context("remote 'origin' not found")?;

    let refspec = format!("refs/heads/{}:refs/heads/{}", branch, branch);

    let mut callbacks = git2::RemoteCallbacks::new();
    let mut tried_ssh_agent = false;
    let mut key_idx: usize = 0;
    let key_names = ["id_ed25519", "id_rsa", "id_ecdsa"];

    callbacks.credentials(move |url, username, allowed| {
        if allowed.contains(git2::CredentialType::USERNAME) {
            return git2::Cred::username(username.unwrap_or("git"));
        }
        if allowed.contains(git2::CredentialType::SSH_KEY) {
            if !tried_ssh_agent {
                tried_ssh_agent = true;
                if let Ok(c) = git2::Cred::ssh_key_from_agent(username.unwrap_or("git")) {
                    return Ok(c);
                }
            }
            let home = dirs::home_dir().unwrap_or_default();
            while key_idx < key_names.len() {
                let key = home.join(".ssh").join(key_names[key_idx]);
                key_idx += 1;
                if key.exists() {
                    if let Ok(c) = git2::Cred::ssh_key(username.unwrap_or("git"), None, &key, None) {
                        return Ok(c);
                    }
                }
            }
        }
        if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
            if let Ok(cfg) = git2::Config::open_default() {
                if let Ok(c) = git2::Cred::credential_helper(&cfg, url, username) {
                    return Ok(c);
                }
            }
        }
        Err(git2::Error::from_str("no credentials available"))
    });

    let mut push_opts = git2::PushOptions::new();
    push_opts.remote_callbacks(callbacks);

    remote
        .push(&[refspec.as_str()], Some(&mut push_opts))
        .with_context(|| format!("push to origin/{} failed", branch))
}

fn check_ahead_count(repo_root: &std::path::Path, branch: &str) -> Result<usize> {
    let repo = git2::Repository::open(repo_root).context("failed to open repository")?;
    let local_ref = format!("refs/heads/{}", branch);
    let remote_ref = format!("refs/remotes/origin/{}", branch);

    let local_oid = match repo.revparse_single(&local_ref) {
        Ok(o) => o.id(),
        Err(_) => return Ok(0),
    };
    let remote_oid = match repo.revparse_single(&remote_ref) {
        Ok(o) => o.id(),
        Err(_) => return Ok(0),
    };

    let (ahead, _) = repo
        .graph_ahead_behind(local_oid, remote_oid)
        .context("failed to compute ahead/behind")?;
    Ok(ahead)
}

fn scan_diff_for_secrets(
    repo_root: &std::path::Path,
    branch: &str,
) -> Result<Vec<secrets::SecretHit>> {
    let repo = git2::Repository::open(repo_root).context("failed to open repository")?;

    // Build diff between remote tracking branch and local HEAD
    let local_ref = format!("refs/heads/{}", branch);
    let remote_ref = format!("refs/remotes/origin/{}", branch);

    let local_oid = match repo.revparse_single(&local_ref) {
        Ok(o) => o.id(),
        Err(_) => return Ok(vec![]),
    };
    let remote_oid = match repo.revparse_single(&remote_ref) {
        Ok(o) => o.id(),
        Err(_) => {
            // No remote yet — scan the full tree vs empty
            return scan_head_diff_for_secrets(&repo);
        }
    };

    let local_tree = repo
        .find_commit(local_oid)
        .and_then(|c| c.tree())
        .context("failed to get local tree")?;
    let remote_tree = repo
        .find_commit(remote_oid)
        .and_then(|c| c.tree())
        .context("failed to get remote tree")?;

    let diff = repo
        .diff_tree_to_tree(Some(&remote_tree), Some(&local_tree), None)
        .context("failed to diff remote..local")?;

    collect_secret_hits_from_diff(&diff)
}

fn scan_head_diff_for_secrets(repo: &git2::Repository) -> Result<Vec<secrets::SecretHit>> {
    let index = repo.index().context("failed to open index")?;
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(vec![]),
    };
    let head_tree = head
        .peel_to_commit()
        .and_then(|c| c.tree())
        .context("failed to peel HEAD to tree")?;
    let diff = repo
        .diff_tree_to_index(Some(&head_tree), Some(&index), None)
        .context("failed to diff HEAD vs index")?;
    collect_secret_hits_from_diff(&diff)
}

fn collect_secret_hits_from_diff(diff: &git2::Diff<'_>) -> Result<Vec<secrets::SecretHit>> {
    let mut hits: Vec<secrets::SecretHit> = Vec::new();

    diff.print(git2::DiffFormat::Patch, |delta, _hunk, line| {
        if line.origin() == '+' {
            if let Ok(content) = std::str::from_utf8(line.content()) {
                if let Some(mut hit) = secrets::scan_line(content) {
                    hit.file = delta
                        .new_file()
                        .path()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    hit.line = line.new_lineno().unwrap_or(0);
                    hits.push(hit);
                }
            }
        }
        true
    })
    .ok(); // Non-fatal — scan what we can

    Ok(hits)
}
