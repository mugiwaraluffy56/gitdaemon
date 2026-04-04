//! Commit batching, message generation, and hook execution.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use git2::Delta;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{debug, info};

use crate::config::{CommitConfig, CommitStrategy, HooksConfig};

// ============================================================================
// Public types
// ============================================================================

/// Result of a commit attempt.
#[derive(Debug, Clone)]
pub struct CommitResult {
    /// Hex OID of the created commit, or `None` when skipped.
    pub oid: Option<String>,
    pub message: String,
    pub files_changed: usize,
    pub committed_at: DateTime<Utc>,
    pub skipped: bool,
    pub skip_reason: Option<SkipReason>,
}

/// Reason a commit was skipped instead of created.
#[derive(Debug, Clone)]
pub enum SkipReason {
    NoChanges,
    PreCommitHookFailed { exit_code: i32, stderr: String },
    ThresholdNotReached { current: usize, required: usize },
}

/// Change accumulator for the `change_count` commit strategy.
#[derive(Debug, Clone, Default)]
pub struct CommitAccumulator {
    pending: usize,
}

impl CommitAccumulator {
    pub fn new() -> Self { Self::default() }
    pub fn add(&mut self, n: usize) { self.pending += n; }
    pub fn reset(&mut self) { self.pending = 0; }
    pub fn current(&self) -> usize { self.pending }
}

// ============================================================================
// Hook execution
// ============================================================================

async fn run_hook(repo_root: &Path, command: &str) -> Result<()> {
    if command.trim().is_empty() {
        return Ok(());
    }
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(repo_root)
        .output()
        .await
        .with_context(|| format!("failed to spawn hook: {}", command))?;

    if !output.status.success() {
        let exit_code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        debug!(cmd = command, exit_code, stderr = %stderr, "hook failed");
        return Err(anyhow!(
            "pre-commit hook exited with code {}: {}",
            exit_code,
            stderr.trim()
        ));
    }
    Ok(())
}

// ============================================================================
// Core synchronous git helpers (run inside spawn_blocking)
// ============================================================================

/// Collect staged deltas from the index vs HEAD (or initial commit).
fn collect_staged_deltas(repo: &git2::Repository) -> Result<Vec<(Delta, PathBuf)>> {
    let index = repo.index().context("failed to open index")?;
    let mut deltas: Vec<(Delta, PathBuf)> = Vec::new();

    match repo.head() {
        Ok(head_ref) => {
            let head_tree = head_ref
                .peel_to_commit()
                .and_then(|c| c.tree())
                .context("failed to peel HEAD to tree")?;

            let diff = repo
                .diff_tree_to_index(Some(&head_tree), Some(&index), None)
                .context("failed to diff HEAD vs index")?;

            for delta in diff.deltas() {
                let path = delta
                    .new_file()
                    .path()
                    .or_else(|| delta.old_file().path())
                    .map(|p| p.to_path_buf())
                    .unwrap_or_default();
                deltas.push((delta.status(), path));
            }
        }
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
            // Initial commit — list every entry in the index
            for i in 0..index.len() {
                if let Some(entry) = index.get(i) {
                    let path = match std::str::from_utf8(&entry.path) {
                        Ok(s) => PathBuf::from(s),
                        Err(_) => continue,
                    };
                    deltas.push((Delta::Added, path));
                }
            }
        }
        Err(e) => return Err(e.into()),
    }

    Ok(deltas)
}

/// Write tree and create a commit from whatever is currently staged.
fn create_git_commit(repo: &git2::Repository, message: &str) -> Result<String> {
    let mut index = repo.index().context("failed to open index")?;
    let tree_oid = index.write_tree().context("failed to write tree")?;
    let tree = repo.find_tree(tree_oid).context("failed to find tree")?;

    let sig = repo.signature().context("failed to get signature")?;

    let parent_commit = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok());

    let parents: Vec<&git2::Commit<'_>> = parent_commit.as_ref().map(|c| vec![c]).unwrap_or_default();

    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .context("failed to create commit")?;

    Ok(oid.to_string())
}

/// Get the current branch name, falling back to "HEAD".
fn get_branch_name(repo: &git2::Repository) -> String {
    repo.head()
        .ok()
        .and_then(|h| h.shorthand().map(|s| s.to_string()))
        .unwrap_or_else(|| "HEAD".to_string())
}

// ============================================================================
// Public async API
// ============================================================================

/// Attempt to create a commit from whatever is currently staged.
///
/// - Runs the pre-commit hook first (abort on non-zero exit).
/// - Runs the post-commit hook after (non-fatal on failure).
/// - Returns a `CommitResult` describing success or the skip reason.
pub async fn commit(
    repo_root: PathBuf,
    commit_cfg: CommitConfig,
    hook_cfg: HooksConfig,
) -> Result<CommitResult> {
    // ── Phase 1: collect staged deltas ──────────────────────────────────────
    let repo_root_c = repo_root.clone();
    let (deltas, branch) = tokio::task::spawn_blocking(move || -> Result<_> {
        let repo = git2::Repository::open(&repo_root_c)
            .context("failed to open repository")?;
        let deltas = collect_staged_deltas(&repo)?;
        let branch = get_branch_name(&repo);
        Ok((deltas, branch))
    })
    .await
    .context("spawn_blocking join error")??;

    if deltas.is_empty() {
        debug!("no staged changes — skipping commit");
        return Ok(CommitResult {
            oid: None,
            message: String::new(),
            files_changed: 0,
            committed_at: Utc::now(),
            skipped: true,
            skip_reason: Some(SkipReason::NoChanges),
        });
    }

    // ── Phase 2: pre-commit hook ─────────────────────────────────────────────
    if let Err(e) = run_hook(&repo_root, &hook_cfg.pre_commit).await {
        let (exit_code, stderr) = if let Some(msg) = e.to_string().strip_prefix("pre-commit hook exited with code ") {
            let parts: Vec<&str> = msg.splitn(2, ": ").collect();
            let code = parts[0].parse::<i32>().unwrap_or(-1);
            let err = parts.get(1).unwrap_or(&"").to_string();
            (code, err)
        } else {
            (-1, e.to_string())
        };
        return Ok(CommitResult {
            oid: None,
            message: String::new(),
            files_changed: deltas.len(),
            committed_at: Utc::now(),
            skipped: true,
            skip_reason: Some(SkipReason::PreCommitHookFailed { exit_code, stderr }),
        });
    }

    // ── Phase 3: build message and create commit ─────────────────────────────
    let summary = build_summary(&deltas);
    let file_count = deltas.len();
    let message = commit_cfg.format_message(&summary, file_count, &branch);

    let repo_root_c = repo_root.clone();
    let msg_c = message.clone();
    let oid = tokio::task::spawn_blocking(move || {
        let repo = git2::Repository::open(&repo_root_c)
            .context("failed to open repository")?;
        create_git_commit(&repo, &msg_c)
    })
    .await
    .context("spawn_blocking join error")??;

    info!(oid = %oid, files = file_count, message = %message, "committed");

    // ── Phase 4: post-commit hook (non-fatal) ────────────────────────────────
    if !hook_cfg.post_commit.is_empty() {
        if let Err(e) = run_hook(&repo_root, &hook_cfg.post_commit).await {
            debug!(error = %e, "post-commit hook failed (non-fatal)");
        }
    }

    Ok(CommitResult {
        oid: Some(oid),
        message,
        files_changed: file_count,
        committed_at: Utc::now(),
        skipped: false,
        skip_reason: None,
    })
}

/// Commit if the configured strategy determines it's time.
///
/// For `time`: delegates directly to `commit()` (the ticker handles timing).
/// For `change_count`: accumulates `new_changes` and only commits once the
/// threshold is reached.
pub async fn commit_if_ready(
    repo_root: PathBuf,
    commit_cfg: CommitConfig,
    hook_cfg: HooksConfig,
    acc: &mut CommitAccumulator,
    new_changes: usize,
) -> Result<CommitResult> {
    if matches!(commit_cfg.strategy, CommitStrategy::ChangeCount) {
        acc.add(new_changes);
        let current = acc.current();
        let required = commit_cfg.change_threshold;
        debug!(current, required, "change_count strategy check");
        if current < required {
            return Ok(CommitResult {
                oid: None,
                message: String::new(),
                files_changed: current,
                committed_at: Utc::now(),
                skipped: true,
                skip_reason: Some(SkipReason::ThresholdNotReached { current, required }),
            });
        }
    } else {
        acc.add(new_changes);
    }

    let result = commit(repo_root, commit_cfg, hook_cfg).await;
    if let Ok(ref r) = result {
        if !r.skipped {
            acc.reset();
        }
    }
    result
}

// ============================================================================
// Summary builder
// ============================================================================

/// Build a short human-readable summary of the changes.
///
/// Shows up to 3 filenames; appends `+N more` if there are more.
fn build_summary(deltas: &[(Delta, PathBuf)]) -> String {
    if deltas.is_empty() {
        return "no changes".to_string();
    }

    let all_added = deltas.iter().all(|(d, _)| matches!(d, Delta::Added | Delta::Copied));
    let all_deleted = deltas.iter().all(|(d, _)| matches!(d, Delta::Deleted));
    let prefix = if all_added {
        "added:"
    } else if all_deleted {
        "deleted:"
    } else {
        "changed:"
    };

    let total = deltas.len();
    let max_show = 3;
    let mut names: Vec<String> = deltas
        .iter()
        .take(max_show)
        .map(|(_, path)| {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| path.to_string_lossy().into_owned())
        })
        .collect();

    if total > max_show {
        names.push(format!("+{} more", total - max_show));
    }

    format!("{} {}", prefix, names.join(", "))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a git repo with an initial commit containing README.md.
    fn create_test_repo() -> Result<(TempDir, PathBuf)> {
        let temp_dir = TempDir::new()?;
        let path = temp_dir.path().to_path_buf();
        let repo = git2::Repository::init(&path)?;

        fs::write(path.join("README.md"), "# Test Repo\n")?;
        let mut index = repo.index()?;
        index.add_path(Path::new("README.md"))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])?;

        Ok((temp_dir, path))
    }

    /// Stage a file in the repository.
    fn stage_file(repo_path: &Path, name: &str, content: &str) -> Result<()> {
        let repo = git2::Repository::open(repo_path)?;
        fs::write(repo_path.join(name), content)?;
        let mut index = repo.index()?;
        index.add_path(Path::new(name))?;
        index.write()?;
        Ok(())
    }

    #[tokio::test]
    async fn test_commit_creates_commit() -> Result<()> {
        let (dir, path) = create_test_repo()?;
        stage_file(&path, "test.txt", "hello\n")?;

        let result = commit(
            path.clone(),
            CommitConfig::default(),
            HooksConfig::default(),
        )
        .await?;

        assert!(!result.skipped);
        assert!(result.oid.is_some());
        assert!(result.message.contains("auto:"));
        assert_eq!(result.files_changed, 1);
        drop(dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_commit_no_changes_skipped() -> Result<()> {
        let (dir, path) = create_test_repo()?;

        let result = commit(path, CommitConfig::default(), HooksConfig::default()).await?;

        assert!(result.skipped);
        assert!(matches!(result.skip_reason, Some(SkipReason::NoChanges)));
        drop(dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_commit_message_format() -> Result<()> {
        let (dir, path) = create_test_repo()?;
        for name in &["a.txt", "b.txt", "c.txt", "d.txt"] {
            stage_file(&path, name, "x\n")?;
        }

        let cfg = CommitConfig {
            message: "auto: {summary} [{branch}]".to_string(),
            ..Default::default()
        };
        let result = commit(path, cfg, HooksConfig::default()).await?;

        assert!(result.message.contains("auto:"));
        assert!(result.message.contains("changed:"));
        assert!(result.message.contains("a.txt"));
        assert!(result.message.contains("+1 more"));
        drop(dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_pre_commit_hook_aborts() -> Result<()> {
        let (dir, path) = create_test_repo()?;
        stage_file(&path, "test.txt", "x\n")?;

        let hooks = HooksConfig {
            pre_commit: "exit 1".to_string(),
            post_commit: String::new(),
        };
        let result = commit(path, CommitConfig::default(), hooks).await?;

        assert!(result.skipped);
        assert!(matches!(
            result.skip_reason,
            Some(SkipReason::PreCommitHookFailed { exit_code: 1, .. })
        ));
        drop(dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_post_commit_hook_failure_non_fatal() -> Result<()> {
        let (dir, path) = create_test_repo()?;
        stage_file(&path, "test.txt", "x\n")?;

        let hooks = HooksConfig {
            pre_commit: String::new(),
            post_commit: "exit 1".to_string(),
        };
        let result = commit(path, CommitConfig::default(), hooks).await?;

        // Commit should succeed even when post-commit hook fails
        assert!(!result.skipped);
        assert!(result.oid.is_some());
        drop(dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_change_count_below_threshold() -> Result<()> {
        let (dir, path) = create_test_repo()?;
        stage_file(&path, "test.txt", "x\n")?;

        let cfg = CommitConfig {
            strategy: CommitStrategy::ChangeCount,
            change_threshold: 5,
            ..Default::default()
        };
        let mut acc = CommitAccumulator::new();
        let result = commit_if_ready(path, cfg, HooksConfig::default(), &mut acc, 1).await?;

        assert!(result.skipped);
        assert!(matches!(
            result.skip_reason,
            Some(SkipReason::ThresholdNotReached { current: 1, required: 5 })
        ));
        drop(dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_change_count_reaches_threshold() -> Result<()> {
        let (dir, path) = create_test_repo()?;
        for i in 0..5 {
            stage_file(&path, &format!("file{}.txt", i), "x\n")?;
        }

        let cfg = CommitConfig {
            strategy: CommitStrategy::ChangeCount,
            change_threshold: 5,
            ..Default::default()
        };
        let mut acc = CommitAccumulator::new();
        let result = commit_if_ready(path, cfg, HooksConfig::default(), &mut acc, 5).await?;

        assert!(!result.skipped);
        assert!(result.oid.is_some());
        assert_eq!(acc.current(), 0);
        drop(dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_initial_commit() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let path = temp_dir.path().to_path_buf();
        git2::Repository::init(&path)?;
        stage_file(&path, "new.txt", "content\n")?;

        let result = commit(path, CommitConfig::default(), HooksConfig::default()).await?;

        assert!(!result.skipped);
        assert!(result.oid.is_some());
        drop(temp_dir);
        Ok(())
    }

    #[test]
    fn test_build_summary_single_file() {
        let deltas = vec![(Delta::Modified, PathBuf::from("src/main.rs"))];
        let s = build_summary(&deltas);
        assert!(s.contains("changed:"));
        assert!(s.contains("main.rs"));
    }

    #[test]
    fn test_build_summary_truncation() {
        let deltas: Vec<_> = (0..5)
            .map(|i| (Delta::Added, PathBuf::from(format!("file{}.txt", i))))
            .collect();
        let s = build_summary(&deltas);
        assert!(s.contains("added:"));
        assert!(s.contains("file0.txt"));
        assert!(s.contains("+2 more"));
    }
}
