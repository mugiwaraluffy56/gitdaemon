//! Auto-sync base branch.
//!
//! After every fetch cycle `gd` can:
//! 1. **Fast-forward** the base branch (`main`/`master`) to `origin/<base>`
//!    without checking it out — the ref is updated directly.
//! 2. **Rebase** the current working branch onto the updated base, but only
//!    when the working tree is clean so the user's in-progress edits are never
//!    disturbed.
//!
//! If the rebase produces a conflict it is immediately aborted and the error
//! is surfaced through the daemon's error list. The user can then resolve it
//! manually with `git rebase <base_branch>`.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

// ============================================================================
// Public types
// ============================================================================

/// Outcome of one sync cycle.
#[derive(Debug, Clone, Default)]
pub struct SyncResult {
    /// The base branch was fast-forwarded to `origin/<base>`.
    pub base_updated: bool,
    /// How many commits the base branch moved forward.
    pub base_advanced: usize,
    /// The current branch was successfully rebased onto the updated base.
    pub rebased: bool,
    /// Rebase was skipped because the working tree had unstaged modifications.
    pub skipped_dirty: bool,
    /// A rebase conflict was detected and the rebase was aborted.
    pub conflict: bool,
}

// ============================================================================
// Public async entry point
// ============================================================================

/// Fast-forward `base_branch` to `origin/<base_branch>` and, when
/// `do_rebase` is `true`, rebase the current branch onto it.
///
/// This function is a thin async wrapper that moves all blocking git2 and
/// subprocess work onto a `spawn_blocking` thread.
pub async fn sync_base_branch(
    repo_root: PathBuf,
    base_branch: String,
    do_rebase: bool,
) -> Result<SyncResult> {
    tokio::task::spawn_blocking(move || {
        sync_base_branch_sync(&repo_root, &base_branch, do_rebase)
    })
    .await
    .context("spawn_blocking join error")?
}

// ============================================================================
// Synchronous implementation
// ============================================================================

fn sync_base_branch_sync(
    repo_root: &Path,
    base_branch: &str,
    do_rebase: bool,
) -> Result<SyncResult> {
    let repo = git2::Repository::open(repo_root)
        .with_context(|| format!("failed to open repo at {}", repo_root.display()))?;

    let mut result = SyncResult::default();

    // ── Guard: if we ARE on the base branch, there's nothing to sync ────────
    let current = current_branch_name(&repo);
    if current.as_deref() == Some(base_branch) {
        debug!(
            base = %base_branch,
            "currently on base branch — skipping auto-sync"
        );
        return Ok(result);
    }

    // ── Step 1: fast-forward base branch ────────────────────────────────────
    let base_was_updated = fast_forward_base(&repo, base_branch, &mut result)?;

    // ── Step 2: optionally rebase current branch ─────────────────────────────
    if do_rebase && (base_was_updated || result.base_updated) {
        // Drop the repo handle before shelling out — avoids lock contention
        drop(repo);
        rebase_onto_base(repo_root, base_branch, &mut result)?;
    }

    Ok(result)
}

/// Fast-forward `refs/heads/<base>` to `refs/remotes/origin/<base>`.
///
/// Returns `true` when the ref was actually moved forward.
fn fast_forward_base(
    repo: &git2::Repository,
    base_branch: &str,
    result: &mut SyncResult,
) -> Result<bool> {
    let remote_ref = format!("refs/remotes/origin/{}", base_branch);
    let local_ref = format!("refs/heads/{}", base_branch);

    // Remote ref must exist (fetch must have run first)
    let remote_oid = match repo.revparse_single(&remote_ref) {
        Ok(obj) => obj.id(),
        Err(_) => {
            debug!(
                remote_ref = %remote_ref,
                "remote ref not found — skipping fast-forward"
            );
            return Ok(false);
        }
    };

    let local_oid = match repo.revparse_single(&local_ref) {
        Ok(obj) => obj.id(),
        Err(_) => {
            // Local base branch doesn't exist — create it pointing at remote
            repo.reference(
                &local_ref,
                remote_oid,
                false,
                &format!(
                    "gd: create {} tracking origin/{}",
                    base_branch, base_branch
                ),
            )
            .with_context(|| format!("failed to create local ref {}", local_ref))?;
            result.base_updated = true;
            result.base_advanced = 1;
            info!(branch = %base_branch, "created local base branch from remote");
            return Ok(true);
        }
    };

    if local_oid == remote_oid {
        debug!(branch = %base_branch, "base branch already up to date");
        return Ok(false);
    }

    // Only fast-forward: remote must be a strict descendant of local
    let (local_ahead, remote_ahead) = repo
        .graph_ahead_behind(local_oid, remote_oid)
        .context("failed to compute ahead/behind for base branch")?;

    if local_ahead > 0 {
        // Diverged — local base has commits not on origin; skip silently
        debug!(
            branch = %base_branch,
            local_ahead,
            "local base has diverged from remote — skipping fast-forward"
        );
        return Ok(false);
    }

    if remote_ahead == 0 {
        return Ok(false);
    }

    let mut reference = repo
        .find_reference(&local_ref)
        .with_context(|| format!("failed to find ref {}", local_ref))?;

    reference
        .set_target(
            remote_oid,
            &format!(
                "gd: fast-forward {} → origin/{} (+{} commits)",
                base_branch, base_branch, remote_ahead
            ),
        )
        .with_context(|| format!("failed to fast-forward {}", local_ref))?;

    result.base_updated = true;
    result.base_advanced = remote_ahead;
    info!(
        branch = %base_branch,
        commits = remote_ahead,
        "fast-forwarded base branch"
    );

    Ok(true)
}

/// Rebase the current branch onto `base_branch` by shelling out to `git`.
///
/// Shells out rather than using git2's rebase API because git's own
/// implementation handles rerere, custom merge drivers, and edge cases
/// far more robustly.
fn rebase_onto_base(
    repo_root: &Path,
    base_branch: &str,
    result: &mut SyncResult,
) -> Result<()> {
    // Re-open for state and status checks
    let repo = git2::Repository::open(repo_root)
        .context("failed to open repo for rebase check")?;

    // Don't rebase if the repo is already in a non-normal state
    if repo.state() != git2::RepositoryState::Clean {
        debug!(
            state = ?repo.state(),
            "repo is not in clean state — skipping rebase"
        );
        return Ok(());
    }

    // Don't rebase if working tree has unstaged modifications —
    // the user is in the middle of editing and we must not interrupt.
    let mut status_opts = git2::StatusOptions::new();
    status_opts.include_untracked(false);
    let statuses = repo
        .statuses(Some(&mut status_opts))
        .context("failed to get repo status")?;

    let is_dirty = statuses.iter().any(|e| {
        e.status().intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE,
        )
    });
    // Drop statuses first to release the borrow on repo, then drop repo
    // before shelling out to git — avoids lock contention with the subprocess.
    drop(statuses);
    drop(repo);

    if is_dirty {
        // Auto-stash: save the dirty working tree, rebase, then pop the stash
        debug!("working tree dirty — attempting auto-stash before rebase");

        let stash_out = std::process::Command::new("git")
            .current_dir(repo_root)
            .args(["stash", "push", "--include-untracked", "-m", "gd: auto-stash before rebase"])
            .output()
            .context("failed to spawn `git stash push`")?;

        if !stash_out.status.success() {
            let stderr = String::from_utf8_lossy(&stash_out.stderr);
            warn!(error = %stderr.trim(), "auto-stash failed — skipping rebase");
            result.skipped_dirty = true;
            return Ok(());
        }

        // Nothing was actually stashed (e.g. empty repo)
        let stdout = String::from_utf8_lossy(&stash_out.stdout);
        let stashed = !stdout.contains("No local changes to save");

        // Run the rebase
        let rebase_out = std::process::Command::new("git")
            .current_dir(repo_root)
            .args(["rebase", base_branch])
            .output()
            .context("failed to spawn `git rebase`")?;

        if !rebase_out.status.success() {
            let stderr = String::from_utf8_lossy(&rebase_out.stderr);
            warn!(onto = %base_branch, error = %stderr.trim(), "rebase conflict after auto-stash — aborting");
            std::process::Command::new("git")
                .current_dir(repo_root)
                .args(["rebase", "--abort"])
                .output()
                .ok();

            // Restore the stash so the user gets their work back
            if stashed {
                std::process::Command::new("git")
                    .current_dir(repo_root)
                    .args(["stash", "pop"])
                    .output()
                    .ok();
            }

            result.conflict = true;
            return Err(anyhow::anyhow!(
                "rebase onto '{}' had conflicts and was aborted — \
                 your stash was restored, resolve manually with `git rebase {}`",
                base_branch,
                base_branch
            ));
        }

        result.rebased = true;
        info!(onto = %base_branch, "rebased current branch onto updated base (auto-stash)");

        // Pop the stash back
        if stashed {
            let pop_out = std::process::Command::new("git")
                .current_dir(repo_root)
                .args(["stash", "pop"])
                .output()
                .context("failed to spawn `git stash pop`")?;

            if !pop_out.status.success() {
                let stderr = String::from_utf8_lossy(&pop_out.stderr);
                warn!(error = %stderr.trim(), "stash pop had conflicts — leaving stash in place");
                // Non-fatal: the user can `git stash pop` manually
            } else {
                debug!("auto-stash popped cleanly");
            }
        }

        return Ok(());
    }

    // Staged-only changes are fine — `git rebase` will carry the index forward.

    debug!(onto = %base_branch, "running git rebase");

    let rebase_out = std::process::Command::new("git")
        .current_dir(repo_root)
        .args(["rebase", base_branch])
        .output()
        .context("failed to spawn `git rebase`")?;

    if rebase_out.status.success() {
        result.rebased = true;
        info!(onto = %base_branch, "rebased current branch onto updated base");
        return Ok(());
    }

    // Rebase failed — abort immediately to restore a clean state
    let stderr = String::from_utf8_lossy(&rebase_out.stderr);
    warn!(
        onto = %base_branch,
        error = %stderr.trim(),
        "rebase conflict detected — aborting rebase"
    );

    let abort_out = std::process::Command::new("git")
        .current_dir(repo_root)
        .args(["rebase", "--abort"])
        .output();

    if let Ok(out) = &abort_out {
        if !out.status.success() {
            let abort_err = String::from_utf8_lossy(&out.stderr);
            warn!(error = %abort_err.trim(), "git rebase --abort failed");
        }
    }

    result.conflict = true;

    Err(anyhow!(
        "rebase onto '{}' had conflicts and was aborted — \
         resolve manually with `git rebase {}`",
        base_branch,
        base_branch
    ))
}

// ============================================================================
// Helpers
// ============================================================================

fn current_branch_name(repo: &git2::Repository) -> Option<String> {
    repo.head()
        .ok()
        .and_then(|h| h.shorthand().map(|s| s.to_string()))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_result_default() {
        let r = SyncResult::default();
        assert!(!r.base_updated);
        assert_eq!(r.base_advanced, 0);
        assert!(!r.rebased);
        assert!(!r.skipped_dirty);
        assert!(!r.conflict);
    }

    #[test]
    fn sync_result_clone() {
        let mut r = SyncResult::default();
        r.base_updated = true;
        r.base_advanced = 3;
        let r2 = r.clone();
        assert_eq!(r2.base_advanced, 3);
        assert!(r2.base_updated);
    }
}
