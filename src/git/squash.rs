//! `fg squash <n>` — squash the last N auto-commits into one clean commit.
//!
//! Combines the diffs of the last N commits and re-generates a single
//! conventional commit message from the combined changeset.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use tracing::info;

/// Result of a squash operation.
#[derive(Debug, Clone)]
pub struct SquashResult {
    /// New commit OID after squash.
    pub oid: String,
    /// Generated commit message.
    pub message: String,
    /// Number of commits that were squashed.
    pub squashed: usize,
    /// Total files changed in the squashed commit.
    pub files_changed: usize,
}

/// Squash the last `count` commits into one, generating a new message from
/// the combined diff.
pub async fn squash_last(repo_root: PathBuf, count: usize) -> Result<SquashResult> {
    if count < 2 {
        return Err(anyhow!("squash requires at least 2 commits (got {})", count));
    }
    tokio::task::spawn_blocking(move || squash_last_sync(&repo_root, count))
        .await
        .context("spawn_blocking join")?
}

fn squash_last_sync(repo_root: &std::path::Path, count: usize) -> Result<SquashResult> {
    let repo = git2::Repository::open(repo_root).context("failed to open repository")?;

    // Find the base commit (parent of the oldest commit to squash)
    let base_ref = format!("HEAD~{}", count);
    let base_obj = repo
        .revparse_single(&base_ref)
        .with_context(|| format!("not enough commits to squash {} (need > {} commits)", count, count))?;
    let base_commit = base_obj.peel_to_commit().context("failed to peel base to commit")?;
    let base_tree = base_commit.tree().context("failed to get base tree")?;

    // HEAD tree — the state after all the commits we're squashing
    let head_commit = repo
        .head()
        .context("no HEAD")?
        .peel_to_commit()
        .context("failed to peel HEAD to commit")?;
    let head_tree = head_commit.tree().context("failed to get HEAD tree")?;

    // Build the combined diff for message generation
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None)
        .context("failed to diff base..HEAD")?;

    let (deltas, symbols) = extract_deltas_and_symbols(&diff);
    let message = crate::git::commit::build_summary_pub(&deltas, &symbols);
    let files_changed = deltas.len();

    // Soft-reset to base, re-stage everything, commit with new message
    repo.reset(&base_obj, git2::ResetType::Soft, None)
        .context("failed to soft-reset to base")?;

    // Stage all files from the HEAD tree we preserved
    let mut index = repo.index().context("failed to open index")?;
    index
        .read_tree(&head_tree)
        .context("failed to read head tree into index")?;
    index.write().context("failed to write index")?;

    // Create the squash commit
    let sig = repo.signature().context("failed to get signature")?;
    let tree_oid = index.write_tree().context("failed to write tree")?;
    let tree = repo.find_tree(tree_oid).context("failed to find tree")?;

    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&base_commit])
        .context("failed to create squash commit")?;

    info!(oid = %oid, squashed = count, files = files_changed, "squashed commits");

    Ok(SquashResult {
        oid: oid.to_string(),
        message,
        squashed: count,
        files_changed,
    })
}

fn extract_deltas_and_symbols(
    diff: &git2::Diff<'_>,
) -> (Vec<(git2::Delta, std::path::PathBuf)>, Vec<crate::git::commit::DeclaredSymbolPub>) {
    let mut deltas = Vec::new();
    let mut symbols = Vec::new();

    for delta in diff.deltas() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        deltas.push((delta.status(), path));
    }

    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let origin = line.origin();
        if origin == '+' || origin == '-' {
            if let Ok(content) = std::str::from_utf8(line.content()) {
                if let Some(sym) = crate::git::commit::parse_symbol_pub(content, origin == '+') {
                    symbols.push(sym);
                }
            }
        }
        true
    })
    .ok();

    (deltas, symbols)
}
