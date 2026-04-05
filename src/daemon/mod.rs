//! Daemon lifecycle — start, stop, status.

pub mod context;
pub mod ipc;
pub mod pid;
pub mod sync_loop;

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, watch};
use tracing::info;

use crate::config::Config;
use crate::daemon::context::DaemonContext;
use crate::daemon::ipc::start_ipc_server;
use crate::daemon::pid::{remove_pid, running_daemon_pid, write_pid};

/// Paths used by the daemon inside a `.fg/` directory.
pub struct DaemonPaths {
    pub pid_file: PathBuf,
    pub socket: PathBuf,
    pub log_file: PathBuf,
}

impl DaemonPaths {
    /// Resolve paths relative to `repo_root`.
    pub fn new(repo_root: &Path) -> Self {
        let fg_dir = repo_root.join(".fg");
        Self {
            pid_file: fg_dir.join("daemon.pid"),
            socket: fg_dir.join("daemon.sock"),
            log_file: fg_dir.join("daemon.log"),
        }
    }
}

/// Start the daemon (foreground — blocks until shutdown).
///
/// Writes the PID file, opens the IPC socket, and runs the sync loop.
/// On return (or panic), cleans up PID file and socket.
pub async fn start_daemon(repo_root: PathBuf, config: Config) -> Result<()> {
    let paths = DaemonPaths::new(&repo_root);

    // Ensure .fg/ exists
    if let Some(parent) = paths.pid_file.parent() {
        std::fs::create_dir_all(parent).context("failed to create .fg/ directory")?;
    }

    // Check for existing daemon
    if let Some(pid) = running_daemon_pid(&paths.pid_file) {
        return Err(anyhow!(
            "daemon already running (pid {}). Use `fg down` to stop it.",
            pid
        ));
    }

    // Write PID
    let my_pid = std::process::id();
    write_pid(my_pid, &paths.pid_file)?;
    info!(pid = my_pid, repo = %repo_root.display(), "daemon started");

    // Build communication channels
    let (ipc_cmd_tx, ipc_cmd_rx) = mpsc::channel(64);
    let (shutdown_tx, shutdown_rx) = watch::channel(());

    // Set up signal handlers
    let shutdown_tx_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        info!("signal received — initiating shutdown");
        shutdown_tx_signal.send(()).ok();
    });

    // Start IPC server in background
    let socket_path = paths.socket.clone();
    let ipc_tx = ipc_cmd_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = start_ipc_server(&socket_path, ipc_tx).await {
            tracing::error!(error = %e, "IPC server error");
        }
    });

    let ctx = DaemonContext {
        repo_root,
        config,
        ipc_rx: ipc_cmd_rx,
        shutdown_rx,
    };

    // Run the main sync loop (blocks until shutdown)
    let result = sync_loop::run(ctx).await;

    // Cleanup
    remove_pid(&paths.pid_file);
    if paths.socket.exists() {
        std::fs::remove_file(&paths.socket).ok();
    }
    info!("daemon stopped");

    result
}

/// Stop the daemon by sending SIGTERM to the process recorded in the PID file.
pub fn stop_daemon(pid_path: &Path) -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let pid = crate::daemon::pid::read_pid(pid_path)
        .with_context(|| format!("failed to read PID file {}", pid_path.display()))?;

    if !crate::daemon::pid::pid_is_running(pid) {
        remove_pid(pid_path);
        return Err(anyhow!("no process with PID {} found", pid));
    }

    kill(Pid::from_raw(pid as i32), Signal::SIGTERM)
        .with_context(|| format!("failed to send SIGTERM to pid {}", pid))?;

    info!(pid, "sent SIGTERM to daemon");
    Ok(())
}

// ============================================================================
// Signal handling
// ============================================================================

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    tokio::signal::ctrl_c().await.ok();
}
