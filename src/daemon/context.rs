//! Shared state passed to the daemon's sync loop.

use std::path::PathBuf;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::daemon::ipc::IpcCommandWithReply;

/// Everything the sync loop needs to orchestrate git operations and IPC.
pub struct DaemonContext {
    /// Absolute path to the repo being managed.
    pub repo_root: PathBuf,
    /// Loaded and validated configuration.
    pub config: Config,
    /// Receives commands from the IPC server (with reply channels).
    pub ipc_rx: mpsc::Receiver<IpcCommandWithReply>,
    /// Receives the shutdown signal.
    pub shutdown_rx: tokio::sync::watch::Receiver<()>,
}
