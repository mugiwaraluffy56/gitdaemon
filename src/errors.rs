use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HookError {
    #[error("pre-commit hook failed with exit code {exit_code}: {stderr}")]
    NonZeroExit { exit_code: i32, stderr: String },
    #[error("failed to execute hook: {0}")]
    ExecutionFailed(#[from] std::io::Error),
    #[error("hook command is empty")]
    EmptyCommand,
}

#[derive(Error, Debug)]
pub enum GitError {
    #[error("git operation failed: {0}")]
    OperationFailed(String),
    #[error("no changes to commit")]
    NoChanges,
    #[error("failed to open repository at {path}: {source}")]
    RepositoryOpenFailed { path: PathBuf, source: git2::Error },
}

impl From<git2::Error> for GitError {
    fn from(err: git2::Error) -> Self {
        GitError::OperationFailed(err.to_string())
    }
}
