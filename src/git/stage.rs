//! Auto-staging: detect working-tree changes and add them to the index.

use anyhow::{Context, Result};
use glob::Pattern;
use std::path::{Path, PathBuf};
use tracing::{debug, trace};

/// Stage all working-tree changes, skipping files that match any of the
/// provided ignore patterns or the `.fg/` daemon state directory.
///
/// Returns the list of files that were staged (paths relative to repo root).
pub fn stage_changes(repo_root: &Path, ignore_patterns: &[String]) -> Result<Vec<PathBuf>> {
    let repo = git2::Repository::open(repo_root)
        .with_context(|| format!("failed to open repository at {}", repo_root.display()))?;

    let compiled: Vec<Pattern> = ignore_patterns
        .iter()
        .filter_map(|p| Pattern::new(p).ok())
        .collect();

    let mut status_opts = git2::StatusOptions::new();
    status_opts
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);

    let statuses = repo
        .statuses(Some(&mut status_opts))
        .context("failed to read repository status")?;

    let mut index = repo.index().context("failed to open index")?;
    let mut staged: Vec<PathBuf> = Vec::new();

    for entry in statuses.iter() {
        let status = entry.status();

        // Skip files that are already clean or explicitly ignored by git
        if status == git2::Status::CURRENT || status.contains(git2::Status::IGNORED) {
            continue;
        }

        let path_str = match entry.path() {
            Some(p) => p,
            None => continue,
        };
        let rel_path = Path::new(path_str);

        // Never stage anything inside .fg/ (daemon state directory)
        if path_str.starts_with(".fg/") || path_str == ".fg" {
            trace!(path = path_str, "skipping .fg/ path");
            continue;
        }

        // Apply custom ignore patterns against the full relative path and basename
        let file_name = rel_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path_str);

        if compiled.iter().any(|pat| {
            pat.matches(path_str) || pat.matches(file_name)
        }) {
            trace!(path = path_str, "skipping (ignore pattern)");
            continue;
        }

        // Stage: deletions get removed from the index, everything else gets added
        let needs_delete = status.intersects(
            git2::Status::WT_DELETED | git2::Status::INDEX_DELETED,
        );
        let needs_add = status.intersects(
            git2::Status::WT_NEW
                | git2::Status::WT_MODIFIED
                | git2::Status::WT_RENAMED
                | git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_RENAMED,
        );

        if needs_delete {
            index.remove_path(rel_path).ok(); // Removal is best-effort
            staged.push(rel_path.to_path_buf());
            debug!(path = path_str, "staged removal");
        } else if needs_add {
            index
                .add_path(rel_path)
                .with_context(|| format!("failed to stage {}", path_str))?;
            staged.push(rel_path.to_path_buf());
            debug!(path = path_str, "staged");
        }
    }

    if !staged.is_empty() {
        index.write().context("failed to write index")?;
        debug!(count = staged.len(), "wrote index");
    }

    Ok(staged)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn init_repo_with_commit(dir: &Path) -> Result<(), anyhow::Error> {
        let repo = git2::Repository::init(dir)?;
        let readme = dir.join("README.md");
        fs::write(&readme, "# test\n")?;
        let mut index = repo.index()?;
        index.add_path(Path::new("README.md"))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let sig = git2::Signature::now("test", "test@test.com")?;
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])?;
        Ok(())
    }

    #[test]
    fn test_stage_new_file() -> Result<(), anyhow::Error> {
        let dir = TempDir::new()?;
        init_repo_with_commit(dir.path())?;
        fs::write(dir.path().join("new.txt"), "hello")?;
        let staged = stage_changes(dir.path(), &[])?;
        assert!(staged.iter().any(|p| p == Path::new("new.txt")));
        Ok(())
    }

    #[test]
    fn test_stage_ignores_fg_dir() -> Result<(), anyhow::Error> {
        let dir = TempDir::new()?;
        init_repo_with_commit(dir.path())?;
        let fg_dir = dir.path().join(".fg");
        fs::create_dir_all(&fg_dir)?;
        fs::write(fg_dir.join("daemon.pid"), "12345")?;
        let staged = stage_changes(dir.path(), &[])?;
        assert!(staged.iter().all(|p| !p.starts_with(".fg")));
        Ok(())
    }

    #[test]
    fn test_stage_respects_custom_ignore() -> Result<(), anyhow::Error> {
        let dir = TempDir::new()?;
        init_repo_with_commit(dir.path())?;
        fs::write(dir.path().join("debug.log"), "logs")?;
        fs::write(dir.path().join("keep.txt"), "keep")?;
        let staged = stage_changes(dir.path(), &["*.log".to_string()])?;
        assert!(staged.iter().all(|p| p != Path::new("debug.log")));
        assert!(staged.iter().any(|p| p == Path::new("keep.txt")));
        Ok(())
    }
}
