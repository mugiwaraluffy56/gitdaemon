//! Daemon status snapshot and terminal renderer.

use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};

/// A point-in-time snapshot of the daemon's sync state, returned by
/// `fg status` and the IPC `Status` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusSnapshot {
    /// Absolute path to the repo root.
    pub repo_root: String,
    /// Current branch name (e.g. "main").
    pub branch: String,
    /// Commits ahead of `origin/<branch>`.
    pub ahead: usize,
    /// Commits behind `origin/<branch>`.
    pub behind: usize,
    /// Files currently in the index (staged but not yet pushed).
    pub staged_files: Vec<String>,
    /// Number of files the watcher is tracking.
    pub watching_count: usize,
    /// Whether auto-push is paused.
    pub is_paused: bool,
    /// Commits created but not yet pushed.
    pub pending_commits: usize,
    /// Time of last successful fetch, if any.
    pub last_fetch: Option<DateTime<Utc>>,
    /// Time of last successful push, if any.
    pub last_push: Option<DateTime<Utc>>,
    /// PID of the running daemon, if known.
    pub daemon_pid: Option<u32>,
    /// Recent non-fatal error messages (capped at 5).
    pub recent_errors: Vec<String>,
    /// Overall sync health.
    pub health: SyncHealth,
}

/// High-level sync health classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SyncHealth {
    /// Everything is in sync.
    Healthy,
    /// Behind the remote — fetch needed.
    Behind,
    /// Push paused or blocked.
    Paused,
    /// An error condition is preventing normal operation.
    Error,
}

impl StatusSnapshot {
    /// Build a snapshot from a live `GitRepo`.
    pub fn from_repo(
        repo: &crate::git::GitRepo,
        branch: &str,
        push_state: &crate::git::push::PushState,
        watching_count: usize,
        daemon_pid: Option<u32>,
        recent_errors: Vec<String>,
    ) -> anyhow::Result<Self> {
        let (ahead, behind) = repo.ahead_behind(branch).unwrap_or((0, 0));
        let staged_files = repo.staged_files().unwrap_or_default();

        let health = if push_state.conflict_detected || !recent_errors.is_empty() {
            SyncHealth::Error
        } else if push_state.paused {
            SyncHealth::Paused
        } else if behind > 0 {
            SyncHealth::Behind
        } else {
            SyncHealth::Healthy
        };

        Ok(Self {
            repo_root: repo.path().to_string_lossy().into_owned(),
            branch: branch.to_string(),
            ahead,
            behind,
            staged_files,
            watching_count,
            is_paused: push_state.paused,
            pending_commits: push_state.queued_commits,
            last_fetch: None,
            last_push: push_state.last_push,
            daemon_pid,
            recent_errors,
            health,
        })
    }

    /// Render the snapshot to a human-readable terminal string.
    pub fn render(&self) -> String {
        let mut lines = Vec::new();

        // Header
        let health_icon = match self.health {
            SyncHealth::Healthy => "⚡".green().to_string(),
            SyncHealth::Behind => "⬇".yellow().to_string(),
            SyncHealth::Paused => "⏸".yellow().to_string(),
            SyncHealth::Error => "✗".red().to_string(),
        };
        lines.push(format!(
            "{} {} — {}",
            health_icon,
            "fastgit".bold(),
            self.repo_root.dimmed()
        ));

        // Branch
        lines.push(format!(
            "  {}   {} → origin/{}",
            "branch".dimmed(),
            self.branch.cyan().bold(),
            self.branch.cyan()
        ));

        // Ahead / behind
        let ahead_str = if self.ahead > 0 {
            format!(
                "{} commits (queued to push{})",
                self.ahead,
                self.last_push
                    .map(|t| {
                        let secs = (Utc::now() - t).num_seconds();
                        format!(", last push {}s ago", secs)
                    })
                    .unwrap_or_default()
            )
            .yellow()
            .to_string()
        } else {
            "up to date".green().to_string()
        };
        lines.push(format!("  {}    {}", "ahead".dimmed(), ahead_str));

        let behind_str = if self.behind > 0 {
            self.behind.to_string().yellow().to_string()
        } else {
            "0".green().to_string()
        };
        lines.push(format!("  {}   {}", "behind".dimmed(), behind_str));

        // Staged files
        let staged_str = if self.staged_files.is_empty() {
            "none".dimmed().to_string()
        } else {
            format!("{} files", self.staged_files.len()).yellow().to_string()
        };
        lines.push(format!("  {}   {}", "staged".dimmed(), staged_str));

        // Watching
        lines.push(format!(
            "  {} {}",
            "watching".dimmed(),
            format!("{} files", self.watching_count).normal()
        ));

        // Daemon
        let daemon_str = match self.daemon_pid {
            Some(pid) => format!("running (pid {})", pid).green().to_string(),
            None => "not running".red().to_string(),
        };
        lines.push(format!("  {}   {}", "daemon".dimmed(), daemon_str));

        // Paused notice
        if self.is_paused {
            lines.push(format!(
                "\n  {} auto-push is paused — run {} to resume",
                "⏸".yellow(),
                "fg resume".bold()
            ));
        }

        // Recent errors
        for err in &self.recent_errors {
            lines.push(format!("  {} {}", "error:".red().bold(), err));
        }

        lines.join("\n")
    }
}

impl Default for StatusSnapshot {
    fn default() -> Self {
        Self {
            repo_root: String::new(),
            branch: "main".to_string(),
            ahead: 0,
            behind: 0,
            staged_files: Vec::new(),
            watching_count: 0,
            is_paused: false,
            pending_commits: 0,
            last_fetch: None,
            last_push: None,
            daemon_pid: None,
            recent_errors: Vec::new(),
            health: SyncHealth::Healthy,
        }
    }
}
