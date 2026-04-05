//! Unix-socket IPC: JSON protocol between `fg` CLI and the daemon.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::status::StatusSnapshot;

// ============================================================================
// Protocol types
// ============================================================================

/// Commands the CLI can send to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcCommand {
    Status,
    Pause,
    Resume,
    PushNow,
    Shutdown,
    Ping,
    /// Soft-reset the last `count` auto-commits back to the index.
    Undo { count: usize, force: bool },
    /// Squash the last `count` auto-commits into one clean commit.
    Squash { count: usize },
}

/// Responses from the daemon to the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcResponse {
    Status(StatusSnapshot),
    Ok { message: String },
    Pong,
    Error { message: String },
}

/// A command paired with a channel to send the reply back on.
pub type IpcCommandWithReply = (IpcCommand, oneshot::Sender<IpcResponse>);

// ============================================================================
// Server (daemon side)
// ============================================================================

/// Start the IPC server, listening on `socket_path`.
///
/// Each incoming connection is handled on its own task. Commands are forwarded
/// to `cmd_tx`; replies arrive back on a one-shot channel and are written to
/// the connection.
pub async fn start_ipc_server(
    socket_path: &Path,
    cmd_tx: mpsc::Sender<IpcCommandWithReply>,
) -> Result<()> {
    // Remove stale socket file from a previous run
    if socket_path.exists() {
        std::fs::remove_file(socket_path).ok();
    }

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("failed to bind IPC socket {}", socket_path.display()))?;

    info!(path = %socket_path.display(), "IPC server listening");

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let tx = cmd_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, tx).await {
                        warn!(error = %e, "IPC connection error");
                    }
                });
            }
            Err(e) => {
                error!(error = %e, "IPC accept error");
                // Brief back-off before retrying
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

async fn handle_connection(
    stream: UnixStream,
    cmd_tx: mpsc::Sender<IpcCommandWithReply>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    reader
        .read_line(&mut line)
        .await
        .context("failed to read IPC command")?;

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    debug!(raw = trimmed, "IPC command received");

    let cmd: IpcCommand = match serde_json::from_str(trimmed) {
        Ok(c) => c,
        Err(e) => {
            let resp = IpcResponse::Error {
                message: format!("invalid command: {}", e),
            };
            write_response(&mut write_half, &resp).await?;
            return Ok(());
        }
    };

    let (reply_tx, reply_rx) = oneshot::channel();
    cmd_tx
        .send((cmd, reply_tx))
        .await
        .map_err(|_| anyhow!("daemon command channel closed"))?;

    match reply_rx.await {
        Ok(response) => write_response(&mut write_half, &response).await?,
        Err(_) => {
            let err = IpcResponse::Error {
                message: "daemon did not reply".to_string(),
            };
            write_response(&mut write_half, &err).await?;
        }
    }

    Ok(())
}

async fn write_response(
    stream: &mut tokio::net::unix::OwnedWriteHalf,
    resp: &IpcResponse,
) -> Result<()> {
    let mut json = serde_json::to_string(resp).context("failed to serialise IPC response")?;
    json.push('\n');
    stream
        .write_all(json.as_bytes())
        .await
        .context("failed to write IPC response")?;
    Ok(())
}

// ============================================================================
// Client (CLI side)
// ============================================================================

/// Send a command to the daemon via the IPC socket and return its response.
pub async fn send_command(socket_path: &Path, cmd: IpcCommand) -> Result<IpcResponse> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| {
            format!(
                "failed to connect to daemon socket {}. Is the daemon running? (fg up)",
                socket_path.display()
            )
        })?;

    let (read_half, mut write_half) = stream.into_split();

    // Send command
    let mut json = serde_json::to_string(&cmd).context("failed to serialise command")?;
    json.push('\n');
    write_half
        .write_all(json.as_bytes())
        .await
        .context("failed to write command")?;
    // Signal EOF so the server knows we're done sending
    write_half.shutdown().await.ok();

    // Read response
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .context("failed to read response")?;

    serde_json::from_str(line.trim()).context("failed to parse IPC response")
}
