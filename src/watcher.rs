//! Filesystem watcher — wraps `notify` and delivers debounced `FileEvent`s.

use anyhow::{Context, Result};
use glob::Pattern;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

/// The type of filesystem change detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
    Renamed,
    Other,
}

/// A single file-change event, filtered and debounced.
#[derive(Debug, Clone)]
pub struct FileEvent {
    pub path: PathBuf,
    pub kind: FileChangeKind,
}

/// Holds the watcher alive for the duration of the daemon.
pub struct ChangeWatcher {
    /// Keep the underlying watcher alive; dropping it stops watching.
    _watcher: RecommendedWatcher,
}

impl ChangeWatcher {
    /// Start watching `repo_root` recursively, filtering by `ignore_patterns`.
    ///
    /// Events are debounced over a 150 ms window and delivered via `tx`.
    /// The `.gd/` directory is always ignored.
    pub fn start(
        repo_root: &Path,
        ignore_patterns: Vec<String>,
        tx: mpsc::Sender<FileEvent>,
    ) -> Result<Self> {
        let (raw_tx, raw_rx) = std_mpsc::channel::<notify::Result<notify::Event>>();

        let mut watcher = notify::recommended_watcher(move |res| {
            raw_tx.send(res).ok();
        })
        .context("failed to create filesystem watcher")?;

        watcher
            .watch(repo_root, RecursiveMode::Recursive)
            .with_context(|| {
                format!("failed to watch directory {}", repo_root.display())
            })?;

        let compiled: Vec<Pattern> = ignore_patterns
            .iter()
            .filter_map(|p| Pattern::new(p).ok())
            .collect();

        // Bridge the std channel to a tokio channel via a dedicated thread.
        // The thread also implements a simple per-path debounce.
        let repo_root_owned = repo_root.to_path_buf();
        std::thread::spawn(move || {
            debounce_and_forward(raw_rx, tx, compiled, repo_root_owned);
        });

        Ok(Self { _watcher: watcher })
    }
}

// ============================================================================
// Debounce thread
// ============================================================================

const DEBOUNCE_MS: u64 = 150;

fn debounce_and_forward(
    raw_rx: std_mpsc::Receiver<notify::Result<notify::Event>>,
    tx: mpsc::Sender<FileEvent>,
    ignore_patterns: Vec<Pattern>,
    repo_root: PathBuf,
) {
    // pending[path] = (FileChangeKind, last_event_time)
    let mut pending: HashMap<PathBuf, (FileChangeKind, Instant)> = HashMap::new();

    loop {
        // Drain all immediately available events, then wait up to DEBOUNCE_MS
        let recv_result = raw_rx.recv_timeout(Duration::from_millis(DEBOUNCE_MS));

        match recv_result {
            Ok(Ok(event)) => {
                for path in &event.paths {
                    if should_skip(path, &repo_root, &ignore_patterns) {
                        trace!(path = %path.display(), "watcher: skipping");
                        continue;
                    }
                    let kind = event_kind(&event.kind);
                    trace!(path = %path.display(), kind = ?kind, "watcher: queued");
                    pending.insert(path.clone(), (kind, Instant::now()));
                }
            }
            Ok(Err(e)) => {
                warn!(error = %e, "watcher error");
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                // Flush events whose debounce window has expired
                let now = Instant::now();
                let threshold = Duration::from_millis(DEBOUNCE_MS);
                let ready: Vec<(PathBuf, FileChangeKind)> = pending
                    .iter()
                    .filter(|(_, (_, t))| now.duration_since(*t) >= threshold)
                    .map(|(p, (k, _))| (p.clone(), k.clone()))
                    .collect();

                for (path, kind) in ready {
                    pending.remove(&path);
                    debug!(path = %path.display(), kind = ?kind, "watcher: event");
                    let ev = FileEvent { path, kind };
                    if tx.blocking_send(ev).is_err() {
                        // Receiver dropped — daemon is shutting down
                        return;
                    }
                }
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                // Watcher dropped — we're done
                return;
            }
        }
    }
}

fn should_skip(path: &Path, repo_root: &Path, patterns: &[Pattern]) -> bool {
    // Compute path relative to repo_root
    let rel = match path.strip_prefix(repo_root) {
        Ok(r) => r,
        Err(_) => path,
    };
    let rel_str = rel.to_string_lossy();

    // Always skip .gd/ daemon state and .git/ internal files
    if rel_str.starts_with(".gd") || rel_str.starts_with(".git") {
        return true;
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    patterns.iter().any(|p| p.matches(&rel_str) || p.matches(file_name))
}

fn event_kind(kind: &EventKind) -> FileChangeKind {
    match kind {
        EventKind::Create(_) => FileChangeKind::Created,
        EventKind::Modify(_) => FileChangeKind::Modified,
        EventKind::Remove(_) => FileChangeKind::Deleted,
        EventKind::Access(_) => FileChangeKind::Other,
        EventKind::Other => FileChangeKind::Other,
        _ => FileChangeKind::Modified,
    }
}
