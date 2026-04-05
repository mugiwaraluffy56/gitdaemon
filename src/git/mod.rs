//! Git operations module — high-level async wrappers over git2.

pub mod commit;
pub mod fetch;
pub mod push;
pub mod secrets;
pub mod stage;
pub mod sync;

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// Lightweight handle to a Git repository.
///
/// Stores only the path; opens a fresh `git2::Repository` on each
/// operation so the handle is `Clone + Send + Sync`.
#[derive(Debug, Clone)]
pub struct GitRepo {
    path: PathBuf,
}

impl GitRepo {
    /// Verify the path is a valid Git repository and return a handle.
    pub fn open(path: &Path) -> Result<Self> {
        git2::Repository::open(path)
            .map_err(|e| anyhow!("not a git repository at {}: {}", path.display(), e))?;
        Ok(Self { path: path.to_path_buf() })
    }

    /// Path to the working directory root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Open a fresh `git2::Repository` from the stored path.
    ///
    /// Use this inside `spawn_blocking` closures.
    pub fn repo(&self) -> Result<git2::Repository> {
        git2::Repository::open(&self.path)
            .map_err(|e| anyhow!("failed to open repository: {}", e))
    }

    /// Return the short name of HEAD (branch name), or `None` when in detached-HEAD state.
    pub fn current_branch(&self) -> Result<Option<String>> {
        let repo = self.repo()?;
        let head = match repo.head() {
            Ok(h) => h,
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(head.shorthand().map(|s| s.to_string()))
    }

    /// Return `true` when the working tree or index has any changes.
    pub fn is_dirty(&self) -> Result<bool> {
        let repo = self.repo()?;
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true);
        let statuses = repo.statuses(Some(&mut opts))?;
        Ok(!statuses.is_empty())
    }

    /// Return the list of configured remote names (e.g. `["origin"]`).
    pub fn remote_names(&self) -> Result<Vec<String>> {
        let repo = self.repo()?;
        let arr = repo.remotes()?;
        let names: Vec<String> = arr.iter().flatten().map(|s| s.to_string()).collect();
        Ok(names)
    }

    /// Return `(ahead, behind)` relative to `refs/remotes/origin/<branch>`.
    ///
    /// Returns `(0, 0)` when the remote ref does not exist yet.
    pub fn ahead_behind(&self, branch: &str) -> Result<(usize, usize)> {
        let repo = self.repo()?;
        let local_ref = format!("refs/heads/{}", branch);
        let remote_ref = format!("refs/remotes/origin/{}", branch);

        let local_oid = match repo.revparse_single(&local_ref) {
            Ok(obj) => obj.id(),
            Err(_) => return Ok((0, 0)),
        };
        let remote_oid = match repo.revparse_single(&remote_ref) {
            Ok(obj) => obj.id(),
            Err(_) => return Ok((0, 0)),
        };

        Ok(repo.graph_ahead_behind(local_oid, remote_oid)?)
    }

    /// Return staged file paths relative to the repo root.
    pub fn staged_files(&self) -> Result<Vec<String>> {
        let repo = self.repo()?;
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(false);
        let statuses = repo.statuses(Some(&mut opts))?;
        let mut files = Vec::new();
        for entry in statuses.iter() {
            let s = entry.status();
            if s.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED,
            ) {
                if let Some(p) = entry.path() {
                    files.push(p.to_string());
                }
            }
        }
        Ok(files)
    }

    /// Return `true` when the repository is currently in a merge-conflict state.
    pub fn has_conflict(&self) -> Result<bool> {
        let repo = self.repo()?;
        Ok(repo.state() == git2::RepositoryState::Merge)
    }
}
