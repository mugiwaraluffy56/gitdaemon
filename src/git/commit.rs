//! Commit batching, message generation, and hook execution.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use git2::Delta;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{debug, info};

use crate::config::{AiConfig, CommitConfig, CommitStrategy, HooksConfig};
use crate::git::ai_commit::generate_ai_commit_message;

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

// ============================================================================
// Symbol extraction from diff
// ============================================================================

/// A symbol declaration found on a `+` or `-` diff line.
#[derive(Debug, Clone)]
pub struct DeclaredSymbol {
    pub kind: SymbolKind,
    pub name: String,
    /// `true` = appeared on a `+` line (added/changed), `false` = `-` line (removed).
    pub added: bool,
    /// `true` = was explicitly marked public (`pub`, `export`, capital Go name, etc.)
    pub public: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    /// struct, class, dataclass, record
    Struct,
    /// enum, union
    Enum,
    /// trait, interface
    Trait,
    /// type alias
    TypeAlias,
    /// const / static / let binding
    Const,
    /// synchronous function / method
    Function,
    /// async function / method
    AsyncFn,
}

impl SymbolKind {
    /// Priority for display: types before functions, functions before constants.
    fn priority(&self) -> u8 {
        match self {
            SymbolKind::Trait     => 0,
            SymbolKind::Enum      => 1,
            SymbolKind::Struct    => 2,
            SymbolKind::TypeAlias => 3,
            SymbolKind::AsyncFn   => 4,
            SymbolKind::Function  => 5,
            SymbolKind::Const     => 6,
        }
    }
}

// Patterns ordered highest-priority-first; first match wins per line.
static SYMBOL_PATTERNS: Lazy<Vec<(Regex, SymbolKind, bool)>> = Lazy::new(|| {
    // (pattern, kind, is_public)
    vec![
        // ── Rust ──────────────────────────────────────────────────────────
        (Regex::new(r"^\s*pub\s+struct\s+(\w+)").unwrap(),          SymbolKind::Struct,    true),
        (Regex::new(r"^\s*pub\s+enum\s+(\w+)").unwrap(),            SymbolKind::Enum,      true),
        (Regex::new(r"^\s*pub\s+trait\s+(\w+)").unwrap(),           SymbolKind::Trait,     true),
        (Regex::new(r"^\s*pub\s+type\s+(\w+)").unwrap(),            SymbolKind::TypeAlias, true),
        (Regex::new(r"^\s*pub\s+const\s+(\w+)").unwrap(),           SymbolKind::Const,     true),
        (Regex::new(r"^\s*pub\s+static\s+\w*\s*(\w+)").unwrap(),    SymbolKind::Const,     true),
        (Regex::new(r"^\s*pub\s+async\s+fn\s+(\w+)").unwrap(),      SymbolKind::AsyncFn,   true),
        (Regex::new(r"^\s*pub\s+fn\s+(\w+)").unwrap(),              SymbolKind::Function,  true),
        (Regex::new(r"^\s*async\s+fn\s+(\w+)").unwrap(),            SymbolKind::AsyncFn,   false),
        (Regex::new(r"^\s*fn\s+(\w+)").unwrap(),                    SymbolKind::Function,  false),
        (Regex::new(r"^\s*struct\s+(\w+)").unwrap(),                 SymbolKind::Struct,    false),
        (Regex::new(r"^\s*enum\s+(\w+)").unwrap(),                   SymbolKind::Enum,      false),
        (Regex::new(r"^\s*trait\s+(\w+)").unwrap(),                  SymbolKind::Trait,     false),
        // ── TypeScript / JavaScript ────────────────────────────────────────
        (Regex::new(r"^\s*export\s+(?:default\s+)?class\s+(\w+)").unwrap(),     SymbolKind::Struct,    true),
        (Regex::new(r"^\s*export\s+interface\s+(\w+)").unwrap(),                SymbolKind::Trait,     true),
        (Regex::new(r"^\s*export\s+type\s+(\w+)").unwrap(),                     SymbolKind::TypeAlias, true),
        (Regex::new(r"^\s*export\s+async\s+function\s+(\w+)").unwrap(),         SymbolKind::AsyncFn,   true),
        (Regex::new(r"^\s*export\s+function\s+(\w+)").unwrap(),                 SymbolKind::Function,  true),
        (Regex::new(r"^\s*export\s+const\s+(\w+)").unwrap(),                    SymbolKind::Const,     true),
        // ── Python ────────────────────────────────────────────────────────
        (Regex::new(r"^\s*class\s+(\w+)").unwrap(),                 SymbolKind::Struct,    false),
        (Regex::new(r"^\s*async\s+def\s+(\w+)").unwrap(),           SymbolKind::AsyncFn,   false),
        (Regex::new(r"^\s*def\s+(\w+)").unwrap(),                    SymbolKind::Function,  false),
        // ── Go ────────────────────────────────────────────────────────────
        (Regex::new(r"^\s*type\s+(\w+)\s+struct").unwrap(),         SymbolKind::Struct,    false),
        (Regex::new(r"^\s*type\s+(\w+)\s+interface").unwrap(),       SymbolKind::Trait,     false),
        (Regex::new(r"^\s*func\s+(?:\([^)]+\)\s+)?([A-Z]\w*)").unwrap(), SymbolKind::Function, true),
        (Regex::new(r"^\s*func\s+(?:\([^)]+\)\s+)?([a-z]\w*)").unwrap(), SymbolKind::Function, false),
    ]
});

fn parse_symbol_from_line(line: &str, added: bool) -> Option<DeclaredSymbol> {
    let trimmed = line.trim_start_matches('+').trim_start_matches('-').trim();
    // Skip comment lines
    if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with('*') {
        return None;
    }
    for (re, kind, is_public) in SYMBOL_PATTERNS.iter() {
        if let Some(cap) = re.captures(trimmed) {
            if let Some(m) = cap.get(1) {
                let name = m.as_str().to_string();
                // Skip common noise names
                if matches!(name.as_str(), "main" | "new" | "default" | "fmt" | "init" | "test" | "setup" | "teardown") {
                    continue;
                }
                return Some(DeclaredSymbol { kind: kind.clone(), name, added, public: *is_public });
            }
        }
    }
    None
}

// ============================================================================
// Staged info collector — deltas + symbols in one pass
// ============================================================================

struct StagedInfo {
    deltas: Vec<(Delta, PathBuf)>,
    symbols: Vec<DeclaredSymbol>,
    /// Raw unified diff text (for AI message generation).
    raw_diff: String,
}

fn collect_staged_info(repo: &git2::Repository) -> Result<StagedInfo> {
    let index = repo.index().context("failed to open index")?;
    let mut deltas: Vec<(Delta, PathBuf)> = Vec::new();
    let mut symbols: Vec<DeclaredSymbol> = Vec::new();
    let mut raw_diff = String::new();

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

            // Walk every diff line: extract symbols and collect raw text
            diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
                let origin = line.origin();
                if let Ok(content) = std::str::from_utf8(line.content()) {
                    raw_diff.push_str(content);
                    if origin == '+' || origin == '-' {
                        if let Some(sym) = parse_symbol_from_line(content, origin == '+') {
                            symbols.push(sym);
                        }
                    }
                }
                true
            })
            .ok(); // best-effort; commit proceeds regardless
        }
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
            // Initial commit — list every index entry; no diff to parse
            for i in 0..index.len() {
                if let Some(entry) = index.get(i) {
                    if let Ok(s) = std::str::from_utf8(&entry.path) {
                        deltas.push((Delta::Added, PathBuf::from(s)));
                    }
                }
            }
        }
        Err(e) => return Err(e.into()),
    }

    Ok(StagedInfo { deltas, symbols, raw_diff })
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
/// - If `ai_cfg.enabled`, calls the Claude API to generate the commit message;
///   falls back to the heuristic generator on any API error.
/// - Runs the post-commit hook after (non-fatal on failure).
/// - Returns a `CommitResult` describing success or the skip reason.
pub async fn commit(
    repo_root: PathBuf,
    commit_cfg: CommitConfig,
    hook_cfg: HooksConfig,
    ai_cfg: AiConfig,
) -> Result<CommitResult> {
    // ── Phase 1: collect staged deltas + diff symbols ───────────────────────
    let repo_root_c = repo_root.clone();
    let (staged, branch) = tokio::task::spawn_blocking(move || -> Result<_> {
        let repo = git2::Repository::open(&repo_root_c)
            .context("failed to open repository")?;
        let staged = collect_staged_info(&repo)?;
        let branch = get_branch_name(&repo);
        Ok((staged, branch))
    })
    .await
    .context("spawn_blocking join error")??;
    let deltas = staged.deltas;
    let symbols = staged.symbols;
    let raw_diff = staged.raw_diff;

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
    // Use AI generation when enabled; fall back to the heuristic on any error.
    let summary = if ai_cfg.enabled {
        match generate_ai_commit_message(&raw_diff, &ai_cfg, &repo_root).await {
            Ok(msg) => {
                info!("AI commit message generated");
                msg
            }
            Err(e) => {
                debug!(error = %e, "AI commit message failed — using heuristic fallback");
                build_summary(&deltas, &symbols)
            }
        }
    } else {
        build_summary(&deltas, &symbols)
    };
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
    ai_cfg: AiConfig,
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

    let result = commit(repo_root, commit_cfg, hook_cfg, ai_cfg).await;
    if let Ok(ref r) = result {
        if !r.skipped {
            acc.reset();
        }
    }
    result
}

// ============================================================================
// Conventional commit message generator
// ============================================================================

/// Generate a conventional commit message by analysing both the file set
/// and the actual symbol declarations found in the diff.
///
/// Quality targets — messages like:
/// - `feat(git): introduce PushQueue and implement try_push`
/// - `fix(config): rework validate to reject zero-value intervals`
/// - `refactor(daemon): extract IPC server into dedicated module`
/// - `feat(ipc): introduce IpcCommand and IpcResponse, implement send_command`
/// - `chore: drop stale lock files`
/// - `test(secrets): add coverage for AWS key and GitHub token patterns`
/// - `docs: update README with installation guide`
/// - `build: add serde_json, tighten nix feature flags`
///
/// Produces messages like:
/// - `feat(git): add push queue and credential callback`
/// - `fix(config): update branch validation and interval defaults`
/// - `refactor(daemon): reorganise IPC server, context, and 2 others`
/// - `chore: remove stale lock files`
/// - `test(secrets): add coverage for AWS key and GitHub token patterns`
/// - `docs: update README with installation guide`
///
/// The result is used as the `{summary}` token inside the config message template.
/// With the default template of `"{summary}"` this becomes the full commit message.
/// Public alias for use by `squash.rs` and other callers outside this module.
pub type DeclaredSymbolPub = DeclaredSymbol;

/// Public entry point into the message generator — used by `squash.rs`.
pub fn build_summary_pub(deltas: &[(Delta, PathBuf)], symbols: &[DeclaredSymbolPub]) -> String {
    build_summary(deltas, symbols)
}

/// Public entry point into the symbol parser — used by `squash.rs`.
pub fn parse_symbol_pub(line: &str, added: bool) -> Option<DeclaredSymbolPub> {
    parse_symbol_from_line(line, added)
}

fn build_summary(deltas: &[(Delta, PathBuf)], symbols: &[DeclaredSymbol]) -> String {
    if deltas.is_empty() {
        return "chore: sync working tree".to_string();
    }

    let commit_type = infer_commit_type(deltas, symbols);
    let scope       = infer_scope(deltas);
    let subject     = build_subject(deltas, symbols);

    let type_scope = match scope {
        Some(s) => format!("{}({})", commit_type, s),
        None    => commit_type.to_string(),
    };

    // Large changesets get a body that enumerates every symbol / file.
    if deltas.len() > 5 {
        let body = build_body(deltas, symbols);
        format!("{}: {}\n\n{}", type_scope, subject, body)
    } else {
        format!("{}: {}", type_scope, subject)
    }
}

// ─── commit type ─────────────────────────────────────────────────────────────

fn infer_commit_type(deltas: &[(Delta, PathBuf)], symbols: &[DeclaredSymbol]) -> &'static str {
    let mut test_count  = 0usize;
    let mut doc_count   = 0usize;
    let mut build_count = 0usize;
    let mut src_count   = 0usize;
    let mut added       = 0usize;
    let mut deleted     = 0usize;
    let mut renamed     = 0usize;

    for (delta, path) in deltas {
        match delta {
            Delta::Added | Delta::Copied => added   += 1,
            Delta::Deleted               => deleted += 1,
            Delta::Renamed               => renamed += 1,
            _                            => {}
        }
        if is_test_file(path)   { test_count  += 1; }
        else if is_doc_file(path)   { doc_count   += 1; }
        else if is_build_file(path) { build_count += 1; }
        else                        { src_count   += 1; }
    }

    let total = deltas.len();

    if test_count  == total { return "test"; }
    if doc_count   == total { return "docs"; }
    if build_count == total { return "build"; }
    if deleted     == total { return "chore"; }
    if renamed     == total { return "refactor"; }

    // New public types in source → feat
    let has_new_types = symbols.iter().any(|s| {
        s.added && s.public
            && matches!(s.kind, SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait)
    });
    if src_count > 0 && has_new_types { return "feat"; }

    // Dominant add without new types → feat if new files, fix if modifications
    if src_count > 0 && added * 2 >= total { return "feat"; }
    if added > 0 && deleted > 0 && src_count > 0 { return "refactor"; }

    "fix"
}

fn is_test_file(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/tests/") || s.contains("/test/")
        || s.starts_with("tests/") || s.starts_with("test/")
        || path.file_stem()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("test_") || n.ends_with("_test") || n.ends_with("_spec"))
            .unwrap_or(false)
}

fn is_doc_file(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/docs/") || s.contains("/doc/") || s.starts_with("docs/")
        || matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md" | "mdx" | "rst" | "txt" | "adoc")
        )
        || path.file_name().and_then(|n| n.to_str())
            .map(|n| matches!(n, "README" | "CHANGELOG" | "LICENSE" | "CONTRIBUTING" | "AUTHORS"))
            .unwrap_or(false)
}

fn is_build_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let s    = path.to_string_lossy();
    matches!(
        name,
        "Cargo.toml" | "Cargo.lock"
            | "package.json" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml"
            | "go.mod" | "go.sum"
            | "Makefile" | "CMakeLists.txt"
            | "Dockerfile" | ".dockerignore"
            | ".gitignore" | ".gitattributes"
            | ".editorconfig" | ".rustfmt.toml" | "clippy.toml"
            | "build.rs" | "setup.py" | "pyproject.toml"
    ) || s.contains("/.github/")
        || s.contains("/ci/")
        || (path.extension().and_then(|e| e.to_str()) == Some("toml") && !s.contains("/src/"))
}

// ─── scope ───────────────────────────────────────────────────────────────────

fn infer_scope(deltas: &[(Delta, PathBuf)]) -> Option<String> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (_, path) in deltas {
        if let Some(scope) = extract_scope(path) {
            *counts.entry(scope).or_insert(0) += 1;
        }
    }
    if counts.is_empty() { return None; }
    if counts.len() == 1 { return counts.into_keys().next(); }

    // Use scope only when it covers > 60 % of files
    let total = deltas.len();
    counts
        .into_iter()
        .filter(|(_, n)| n * 10 > total * 6)
        .max_by_key(|(_, n)| *n)
        .map(|(s, _)| s)
}

fn extract_scope(path: &Path) -> Option<String> {
    let components: Vec<_> = path.components().collect();
    let src_idx = components.iter().position(|c| c.as_os_str() == "src")?;
    let next    = components.get(src_idx + 1)?;
    let name    = next.as_os_str().to_string_lossy();

    if name.ends_with(".rs") {
        let stem = name.trim_end_matches(".rs");
        return match stem {
            "main" | "lib" | "errors" => None,
            other => Some(other.to_string()),
        };
    }
    Some(name.into_owned())
}

// ─── subject line (symbol-aware) ─────────────────────────────────────────────

fn build_subject(deltas: &[(Delta, PathBuf)], symbols: &[DeclaredSymbol]) -> String {
    // ── Step 1: partition symbols into four buckets ─────────────────────────
    // A symbol that appears in both +/- lines was modified, not added/removed.
    let added_names: HashSet<&str>   = symbols.iter().filter(|s|  s.added).map(|s| s.name.as_str()).collect();
    let removed_names: HashSet<&str> = symbols.iter().filter(|s| !s.added).map(|s| s.name.as_str()).collect();

    // Prefer public symbols; fall back to private when nothing public is found.
    let prefer_public = symbols.iter().any(|s| s.public && s.added);

    let mut new_types: Vec<&DeclaredSymbol> = Vec::new();   // struct/enum/trait added for the first time
    let mut new_fns:   Vec<&DeclaredSymbol> = Vec::new();   // fn/async fn added for the first time
    let mut reworked:  Vec<&DeclaredSymbol> = Vec::new();   // same name in + and - → modified
    let mut dropped:   Vec<&DeclaredSymbol> = Vec::new();   // removed only

    // Deduplicate by name within each bucket
    let mut seen_new:  HashSet<&str> = HashSet::new();
    let mut seen_rw:   HashSet<&str> = HashSet::new();
    let mut seen_drop: HashSet<&str> = HashSet::new();

    for sym in symbols {
        if prefer_public && !sym.public { continue; }
        let name = sym.name.as_str();

        if sym.added && removed_names.contains(name) {
            // modified
            if seen_rw.insert(name) { reworked.push(sym); }
        } else if sym.added {
            if seen_new.insert(name) {
                match sym.kind {
                    SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait
                    | SymbolKind::TypeAlias => new_types.push(sym),
                    _ => new_fns.push(sym),
                }
            }
        } else if !added_names.contains(name) && seen_drop.insert(name) {
            dropped.push(sym);
        }
    }

    // Sort by priority so the most important symbols appear first
    new_types.sort_by_key(|s| s.kind.priority());
    new_fns.sort_by_key(|s| s.kind.priority());

    // ── Step 2: build clauses ───────────────────────────────────────────────
    let mut clauses: Vec<String> = Vec::new();

    if !new_types.is_empty() {
        clauses.push(format!("introduce {}", sym_list(&new_types)));
    }
    if !new_fns.is_empty() {
        clauses.push(format!("implement {}", sym_list(&new_fns)));
    }
    if !reworked.is_empty() && new_types.is_empty() && new_fns.is_empty() {
        // All modifications, no new symbols — use "rework"
        clauses.push(format!("rework {}", sym_list(&reworked)));
    } else if !reworked.is_empty() {
        // There are also new things — mention the rework at a lower priority
        clauses.push(format!("update {}", sym_list(&reworked)));
    }
    if !dropped.is_empty() {
        clauses.push(format!("drop {}", sym_list(&dropped)));
    }

    if !clauses.is_empty() {
        return join_clauses(clauses);
    }

    // ── Step 3: symbol-less fallback — describe the file changes ───────────
    file_based_subject(deltas)
}

// ─── symbol display ──────────────────────────────────────────────────────────

/// `[sym1, sym2, sym3, and N others]` — at most three names shown.
fn sym_list(syms: &[&DeclaredSymbol]) -> String {
    // Deduplicate while preserving order
    let mut seen = HashSet::new();
    let unique: Vec<&&DeclaredSymbol> = syms.iter().filter(|s| seen.insert(s.name.as_str())).collect();
    let names: Vec<String> = unique.iter().take(3).map(|s| sym_display(&s)).collect();
    let total = unique.len();
    match total {
        0 => unreachable!(),
        1 => names[0].clone(),
        2 => format!("{} and {}", names[0], names[1]),
        3 => format!("{}, {}, and {}", names[0], names[1], names[2]),
        n => format!("{}, {}, and {} others", names[0], names[1], n - 2),
    }
}

/// Display a symbol name naturally:
/// - PascalCase types (`PushQueue`, `IpcCommand`) → kept as-is (they're proper nouns)
/// - snake_case functions (`try_push`, `fetch_all_remotes`) → spaced lowercase
/// - SCREAMING_SNAKE consts → lowercase spaced
fn sym_display(sym: &DeclaredSymbol) -> String {
    match sym.kind {
        SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait | SymbolKind::TypeAlias => {
            // PascalCase → keep as-is
            sym.name.clone()
        }
        SymbolKind::Function | SymbolKind::AsyncFn => {
            // snake_case → spaced, with known abbreviation expansions
            expand_snake(&sym.name)
        }
        SymbolKind::Const => {
            sym.name.to_lowercase().replace('_', " ")
        }
    }
}

/// Expand a `snake_case` identifier into readable English, replacing
/// well-known abbreviations.
fn expand_snake(s: &str) -> String {
    s.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| match w {
            "ipc"   => "IPC",
            "cli"   => "CLI",
            "api"   => "API",
            "url"   => "URL",
            "id"    => "ID",
            "pid"   => "PID",
            "db"    => "DB",
            "io"    => "IO",
            "cfg"   => "config",
            "msg"   => "message",
            "err"   => "error",
            "mgr"   => "manager",
            "ctx"   => "context",
            "srv"   => "server",
            "impl"  => "implementation",
            other   => other,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ─── file-based fallback subject ─────────────────────────────────────────────

fn file_based_subject(deltas: &[(Delta, PathBuf)]) -> String {
    let added:    Vec<&PathBuf> = deltas.iter().filter(|(d,_)| matches!(d, Delta::Added|Delta::Copied)).map(|(_,p)| p).collect();
    let deleted:  Vec<&PathBuf> = deltas.iter().filter(|(d,_)| matches!(d, Delta::Deleted)).map(|(_,p)| p).collect();
    let modified: Vec<&PathBuf> = deltas.iter().filter(|(d,_)| matches!(d, Delta::Modified)).map(|(_,p)| p).collect();
    let renamed:  Vec<&PathBuf> = deltas.iter().filter(|(d,_)| matches!(d, Delta::Renamed)).map(|(_,p)| p).collect();
    let total = deltas.len();

    if total == 1 {
        let (delta, path) = &deltas[0];
        let name = module_name(path);
        return match delta {
            Delta::Added  | Delta::Copied  => format!("add {}", name),
            Delta::Deleted                 => format!("remove {}", name),
            Delta::Modified                => format!("update {}", name),
            Delta::Renamed                 => format!("rename {}", name),
            _                              => format!("update {}", name),
        };
    }

    let mut clauses = Vec::new();
    if !added.is_empty()    { clauses.push(name_list("add",    &added)); }
    if !modified.is_empty() { clauses.push(name_list("update", &modified)); }
    if !renamed.is_empty()  { clauses.push(format!("rename {} files", renamed.len())); }
    if !deleted.is_empty()  {
        let n = deleted.len();
        clauses.push(format!("remove {} file{}", n, if n == 1 { "" } else { "s" }));
    }
    join_clauses(clauses)
}

fn name_list(verb: &str, paths: &[&PathBuf]) -> String {
    let names: Vec<String> = paths.iter().map(|p| module_name(p)).collect();
    let total = names.len();
    let joined = match total {
        1 => names[0].clone(),
        2 => format!("{} and {}", names[0], names[1]),
        3 => format!("{}, {}, and {}", names[0], names[1], names[2]),
        _ => format!("{}, {}, and {} others", names[0], names[1], total - 2),
    };
    format!("{} {}", verb, joined)
}

fn join_clauses(mut clauses: Vec<String>) -> String {
    match clauses.len() {
        0 => "sync changes".to_string(),
        1 => clauses.remove(0),
        2 => format!("{} and {}", clauses[0], clauses[1]),
        _ => {
            let last = clauses.pop().unwrap();
            format!("{}, and {}", clauses.join(", "), last)
        }
    }
}

/// Short, readable module name from a file path — used in fallback subjects.
fn module_name(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    if matches!(stem, "Cargo" | "Makefile" | "Dockerfile" | "README" | "CHANGELOG") {
        return stem.to_string();
    }
    expand_snake(stem)
}

// ─── body for large changesets ───────────────────────────────────────────────

fn build_body(deltas: &[(Delta, PathBuf)], symbols: &[DeclaredSymbol]) -> String {
    // Group added public symbols by file scope
    use std::collections::BTreeMap;
    let mut by_scope: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let added_syms: Vec<&DeclaredSymbol> = symbols.iter()
        .filter(|s| s.added && s.public)
        .collect();

    // Build scope → symbol list
    let mut seen_sym = HashSet::new();
    for sym in &added_syms {
        if seen_sym.insert(sym.name.as_str()) {
            let scope = deltas.iter()
                .find_map(|(_, p)| extract_scope(p))
                .unwrap_or_else(|| "core".to_string());
            by_scope.entry(scope).or_default().push(sym_display(sym));
        }
    }

    let mut lines = Vec::new();

    if by_scope.is_empty() {
        // Fall back to per-file bullet points
        for (delta, path) in deltas {
            let verb = match delta {
                Delta::Added | Delta::Copied => "add",
                Delta::Deleted               => "remove",
                Delta::Modified              => "update",
                Delta::Renamed               => "rename",
                _                            => "modify",
            };
            lines.push(format!("- {} {}", verb, path.to_string_lossy()));
        }
    } else {
        for (scope, names) in &by_scope {
            lines.push(format!("- {}({}): {}", "add", scope, names.join(", ")));
        }
        // Any files not covered by symbol extraction
        for (delta, path) in deltas {
            if extract_scope(path).map_or(true, |s| !by_scope.contains_key(&s)) {
                let verb = match delta {
                    Delta::Added | Delta::Copied => "add",
                    Delta::Deleted               => "remove",
                    Delta::Modified              => "update",
                    Delta::Renamed               => "rename",
                    _                            => "modify",
                };
                lines.push(format!("- {} {}", verb, path.to_string_lossy()));
            }
        }
    }

    lines.join("\n")
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
            AiConfig::default(),
        )
        .await?;

        assert!(!result.skipped);
        assert!(result.oid.is_some());
        // Conventional commit format: "type[(scope)]: subject"
        assert!(result.message.contains(':'), "expected conventional commit: {}", result.message);
        assert_eq!(result.files_changed, 1);
        drop(dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_commit_no_changes_skipped() -> Result<()> {
        let (dir, path) = create_test_repo()?;

        let result = commit(path, CommitConfig::default(), HooksConfig::default(), AiConfig::default()).await?;

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
            message: "[{branch}] {summary}".to_string(),
            ..Default::default()
        };
        let result = commit(path, cfg, HooksConfig::default(), AiConfig::default()).await?;

        // Template prefix is preserved
        assert!(result.message.starts_with('['), "expected branch prefix: {}", result.message);
        // Conventional commit summary is embedded
        assert!(result.message.contains(':'), "expected conventional commit in summary: {}", result.message);
        // Friendly names drop the extension — check for the stem
        assert!(
            result.message.contains(" a") || result.message.contains(" b") || result.message.contains(" c"),
            "expected file name stem in message: {}",
            result.message
        );
        drop(dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_pre_commit_hook_aborts() -> Result<()> {
        let (dir, path) = create_test_repo()?;
        stage_file(&path, "test.txt", "x\n")?;

        let hooks = HooksConfig {
            pre_commit: "exit 1".to_string(),
            ..HooksConfig::default()
        };
        let result = commit(path, CommitConfig::default(), hooks, AiConfig::default()).await?;

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
            post_commit: "exit 1".to_string(),
            ..HooksConfig::default()
        };
        let result = commit(path, CommitConfig::default(), hooks, AiConfig::default()).await?;

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
        let result = commit_if_ready(path, cfg, HooksConfig::default(), AiConfig::default(), &mut acc, 1).await?;

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
        let result = commit_if_ready(path, cfg, HooksConfig::default(), AiConfig::default(), &mut acc, 5).await?;

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

        let result = commit(path, CommitConfig::default(), HooksConfig::default(), AiConfig::default()).await?;

        assert!(!result.skipped);
        assert!(result.oid.is_some());
        drop(temp_dir);
        Ok(())
    }

    // Helper: build a symbol slice easily
    fn sym(kind: SymbolKind, name: &str, added: bool, public: bool) -> DeclaredSymbol {
        DeclaredSymbol { kind, name: name.to_string(), added, public }
    }

    // ── build_summary unit tests ──────────────────────────────────────────────

    #[test]
    fn test_summary_single_modified_file_no_symbols() {
        let deltas = vec![(Delta::Modified, PathBuf::from("src/config.rs"))];
        let s = build_summary(&deltas, &[]);
        assert!(s.contains(':'), "expected conventional commit: {}", s);
        assert!(s.contains("config"), "expected module name: {}", s);
    }

    #[test]
    fn test_summary_new_struct_uses_introduce() {
        let deltas  = vec![(Delta::Added, PathBuf::from("src/git/push.rs"))];
        let symbols = vec![sym(SymbolKind::Struct, "PushQueue", true, true)];
        let s = build_summary(&deltas, &symbols);
        assert!(s.starts_with("feat"), "expected feat: {}", s);
        assert!(s.contains("introduce"), "expected 'introduce': {}", s);
        assert!(s.contains("PushQueue"), "expected symbol name: {}", s);
    }

    #[test]
    fn test_summary_new_async_fn_uses_implement() {
        let deltas  = vec![(Delta::Added, PathBuf::from("src/git/fetch.rs"))];
        let symbols = vec![sym(SymbolKind::AsyncFn, "fetch_all_remotes", true, true)];
        let s = build_summary(&deltas, &symbols);
        assert!(s.contains("implement"), "expected 'implement': {}", s);
        assert!(s.contains("fetch all remotes"), "expected expanded name: {}", s);
    }

    #[test]
    fn test_summary_modified_fn_uses_rework() {
        let deltas = vec![(Delta::Modified, PathBuf::from("src/config.rs"))];
        let symbols = vec![
            sym(SymbolKind::Function, "validate", false, true), // removed version
            sym(SymbolKind::Function, "validate", true,  true), // added version
        ];
        let s = build_summary(&deltas, &symbols);
        assert!(s.contains("rework") || s.contains("validate"),
            "expected rework or symbol name: {}", s);
    }

    #[test]
    fn test_summary_dropped_symbol_uses_drop() {
        let deltas  = vec![(Delta::Deleted, PathBuf::from("src/git/old.rs"))];
        let symbols = vec![sym(SymbolKind::Function, "legacy_fetch", false, true)];
        let s = build_summary(&deltas, &symbols);
        assert!(s.contains("drop"), "expected 'drop': {}", s);
        assert!(s.contains("legacy fetch"), "expected expanded name: {}", s);
    }

    #[test]
    fn test_summary_multiple_types_listed() {
        let deltas = vec![
            (Delta::Added, PathBuf::from("src/daemon/ipc.rs")),
        ];
        let symbols = vec![
            sym(SymbolKind::Enum,     "IpcCommand",  true, true),
            sym(SymbolKind::Enum,     "IpcResponse", true, true),
            sym(SymbolKind::AsyncFn,  "send_command", true, true),
        ];
        let s = build_summary(&deltas, &symbols);
        assert!(s.contains("IpcCommand"),  "expected IpcCommand: {}", s);
        assert!(s.contains("IpcResponse"), "expected IpcResponse: {}", s);
        assert!(s.contains("implement"),   "expected 'implement': {}", s);
    }

    #[test]
    fn test_summary_four_plus_symbols_truncated() {
        let deltas  = vec![(Delta::Added, PathBuf::from("src/git/mod.rs"))];
        let symbols = vec![
            sym(SymbolKind::Struct, "FooA", true, true),
            sym(SymbolKind::Struct, "FooB", true, true),
            sym(SymbolKind::Struct, "FooC", true, true),
            sym(SymbolKind::Struct, "FooD", true, true),
        ];
        let s = build_summary(&deltas, &symbols);
        assert!(s.contains("others"), "expected 'others' truncation: {}", s);
    }

    #[test]
    fn test_summary_scope_from_directory() {
        let deltas = vec![
            (Delta::Added, PathBuf::from("src/git/push.rs")),
            (Delta::Added, PathBuf::from("src/git/fetch.rs")),
        ];
        let s = build_summary(&deltas, &[]);
        assert!(s.contains("(git)"), "expected git scope: {}", s);
    }

    #[test]
    fn test_summary_all_deleted_is_chore() {
        let deltas = vec![
            (Delta::Deleted, PathBuf::from("old.log")),
            (Delta::Deleted, PathBuf::from("tmp.cache")),
        ];
        let s = build_summary(&deltas, &[]);
        assert!(s.starts_with("chore:"), "expected chore: {}", s);
    }

    #[test]
    fn test_summary_test_files() {
        let deltas = vec![
            (Delta::Added, PathBuf::from("tests/integration.rs")),
            (Delta::Modified, PathBuf::from("tests/common.rs")),
        ];
        let s = build_summary(&deltas, &[]);
        assert!(s.starts_with("test"), "expected test type: {}", s);
    }

    #[test]
    fn test_summary_docs_file() {
        let deltas = vec![(Delta::Modified, PathBuf::from("README.md"))];
        let s = build_summary(&deltas, &[]);
        assert!(s.starts_with("docs:"), "expected docs type: {}", s);
    }

    #[test]
    fn test_summary_build_file() {
        let deltas = vec![(Delta::Modified, PathBuf::from("Cargo.toml"))];
        let s = build_summary(&deltas, &[]);
        assert!(s.starts_with("build:"), "expected build type: {}", s);
    }

    #[test]
    fn test_summary_large_changeset_has_body() {
        let deltas: Vec<_> = (0..7)
            .map(|i| (Delta::Added, PathBuf::from(format!("src/module/file{}.rs", i))))
            .collect();
        let s = build_summary(&deltas, &[]);
        assert!(s.contains('\n'), "expected multi-line for large changeset: {}", s);
    }

    // ── parse_symbol_from_line unit tests ────────────────────────────────────

    #[test]
    fn test_parse_pub_struct() {
        let sym = parse_symbol_from_line("+pub struct PushQueue {", true).unwrap();
        assert_eq!(sym.name, "PushQueue");
        assert!(sym.public);
        assert!(matches!(sym.kind, SymbolKind::Struct));
    }

    #[test]
    fn test_parse_pub_async_fn() {
        let sym = parse_symbol_from_line("+pub async fn fetch_all_remotes(", true).unwrap();
        assert_eq!(sym.name, "fetch_all_remotes");
        assert!(sym.public);
        assert!(matches!(sym.kind, SymbolKind::AsyncFn));
    }

    #[test]
    fn test_parse_private_fn() {
        let sym = parse_symbol_from_line("+fn build_summary(deltas:", true).unwrap();
        assert_eq!(sym.name, "build_summary");
        assert!(!sym.public);
        assert!(matches!(sym.kind, SymbolKind::Function));
    }

    #[test]
    fn test_parse_pub_enum() {
        let sym = parse_symbol_from_line("+pub enum IpcCommand {", true).unwrap();
        assert_eq!(sym.name, "IpcCommand");
        assert!(matches!(sym.kind, SymbolKind::Enum));
    }

    #[test]
    fn test_skip_comment_line() {
        assert!(parse_symbol_from_line("+// pub struct Foo", true).is_none());
    }

    #[test]
    fn test_skip_noise_name_new() {
        // "new" is in the skip list
        assert!(parse_symbol_from_line("+    pub fn new() -> Self {", true).is_none());
    }

    // ── sym_display / expand_snake ────────────────────────────────────────────

    #[test]
    fn test_sym_display_pascal_kept() {
        let s = sym(SymbolKind::Struct, "PushQueue", true, true);
        assert_eq!(sym_display(&s), "PushQueue");
    }

    #[test]
    fn test_sym_display_snake_expanded() {
        let s = sym(SymbolKind::Function, "fetch_all_remotes", true, true);
        assert_eq!(sym_display(&s), "fetch all remotes");
    }

    #[test]
    fn test_expand_snake_ipc() {
        assert_eq!(expand_snake("start_ipc_server"), "start IPC server");
    }

    #[test]
    fn test_expand_snake_ctx() {
        assert_eq!(expand_snake("daemon_ctx"), "daemon context");
    }
}
