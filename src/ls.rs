//! `fg ls` — list files the daemon is currently tracking / watching.
//!
//! Shows the working-tree state in a git-status style layout, coloured by
//! whether a file is staged, modified but unstaged, or untracked.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;

/// Category of a tracked file for display purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    /// In the index, not yet committed.
    Staged,
    /// Modified in working tree but not staged.
    Modified,
    /// New file not yet staged.
    Untracked,
    /// Deleted in working tree.
    Deleted,
    /// Renamed.
    Renamed { from: String },
    /// In a merge conflict.
    Conflicted,
}

/// One entry in the `fg ls` output.
#[derive(Debug, Clone)]
pub struct TrackedFile {
    pub path: String,
    pub state: FileState,
}

/// Collect the current working-tree / index state of `repo_root`.
pub fn list_tracked_files(repo_root: &Path) -> Result<Vec<TrackedFile>> {
    let repo = git2::Repository::open(repo_root).context("failed to open repository")?;

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);

    let statuses = repo.statuses(Some(&mut opts)).context("failed to read status")?;

    let mut files = Vec::new();

    for entry in statuses.iter() {
        let s = entry.status();

        // Skip .fg/ internal state
        if entry.path().map(|p| p.starts_with(".fg")).unwrap_or(false) {
            continue;
        }

        let path = entry.path().unwrap_or("?").to_string();

        // Conflict states
        if s.intersects(
            git2::Status::CONFLICTED,
        ) {
            files.push(TrackedFile { path, state: FileState::Conflicted });
            continue;
        }

        // Index (staged) states
        if s.contains(git2::Status::INDEX_RENAMED) {
            let from = entry
                .head_to_index()
                .and_then(|d| d.old_file().path().map(|p| p.to_string_lossy().into_owned()))
                .unwrap_or_default();
            files.push(TrackedFile { path, state: FileState::Renamed { from } });
            continue;
        }
        if s.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED,
        ) {
            let state = if s.contains(git2::Status::INDEX_DELETED) {
                FileState::Deleted
            } else {
                FileState::Staged
            };
            files.push(TrackedFile { path, state });
            continue;
        }

        // Working-tree (unstaged) states
        if s.contains(git2::Status::WT_DELETED) {
            files.push(TrackedFile { path, state: FileState::Deleted });
        } else if s.contains(git2::Status::WT_RENAMED) {
            let from = entry
                .index_to_workdir()
                .and_then(|d| d.old_file().path().map(|p| p.to_string_lossy().into_owned()))
                .unwrap_or_default();
            files.push(TrackedFile { path, state: FileState::Renamed { from } });
        } else if s.contains(git2::Status::WT_MODIFIED) {
            files.push(TrackedFile { path, state: FileState::Modified });
        } else if s.contains(git2::Status::WT_NEW) {
            files.push(TrackedFile { path, state: FileState::Untracked });
        }
    }

    // Sort: staged first, then modified, then untracked, alphabetical within group
    files.sort_by(|a, b| {
        state_sort_key(&a.state)
            .cmp(&state_sort_key(&b.state))
            .then(a.path.cmp(&b.path))
    });

    Ok(files)
}

fn state_sort_key(s: &FileState) -> u8 {
    match s {
        FileState::Conflicted  => 0,
        FileState::Staged      => 1,
        FileState::Renamed { .. } => 2,
        FileState::Deleted     => 3,
        FileState::Modified    => 4,
        FileState::Untracked   => 5,
    }
}

/// Render a list of tracked files to a human-readable string.
pub fn render(files: &[TrackedFile], repo_root: &Path) -> String {
    if files.is_empty() {
        return format!(
            "  {} nothing to show — working tree is clean",
            "✓".green()
        );
    }

    let mut lines = Vec::new();

    lines.push(format!(
        "{} {} files",
        "tracking".dimmed(),
        files.len().to_string().bold()
    ));

    let mut last_group: Option<u8> = None;

    for f in files {
        let group = state_sort_key(&f.state);
        if last_group != Some(group) {
            let header = match &f.state {
                FileState::Conflicted         => "conflicts:".red().bold().to_string(),
                FileState::Staged             => "staged:".green().bold().to_string(),
                FileState::Renamed { .. }     => "renamed:".cyan().bold().to_string(),
                FileState::Deleted            => "deleted:".red().to_string(),
                FileState::Modified           => "modified:".yellow().bold().to_string(),
                FileState::Untracked          => "untracked:".dimmed().to_string(),
            };
            lines.push(format!("\n  {}", header));
            last_group = Some(group);
        }

        let path_display = if let Some(stripped) =
            Path::new(&f.path).strip_prefix(repo_root).ok()
        {
            stripped.to_string_lossy().to_string()
        } else {
            f.path.clone()
        };

        let line = match &f.state {
            FileState::Staged             => format!("    {} {}", "S".green().bold(), path_display.green()),
            FileState::Modified           => format!("    {} {}", "M".yellow().bold(), path_display.yellow()),
            FileState::Untracked          => format!("    {} {}", "?".dimmed(), path_display.dimmed()),
            FileState::Deleted            => format!("    {} {}", "D".red().bold(), path_display.red()),
            FileState::Conflicted         => format!("    {} {}", "!".red().bold(), path_display.red().bold()),
            FileState::Renamed { from }   => format!(
                "    {} {} → {}",
                "R".cyan().bold(),
                from.dimmed(),
                path_display.cyan()
            ),
        };
        lines.push(line);
    }

    lines.join("\n")
}
