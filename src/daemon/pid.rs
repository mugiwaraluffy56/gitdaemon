//! PID file management for the daemon.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use tracing::debug;

/// Write `pid` to the PID file at `path`, creating parent directories as needed.
pub fn write_pid(pid: u32, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    fs::write(path, pid.to_string())
        .with_context(|| format!("failed to write PID file {}", path.display()))?;
    debug!(pid, path = %path.display(), "PID file written");
    Ok(())
}

/// Read the PID from the file at `path`.
pub fn read_pid(path: &Path) -> Result<u32> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read PID file {}", path.display()))?;
    content
        .trim()
        .parse::<u32>()
        .with_context(|| format!("invalid PID in {}: {:?}", path.display(), content.trim()))
}

/// Return `true` if a process with the given PID is currently running.
///
/// On Unix, sends signal 0 to the process (no-op, just checks existence).
pub fn pid_is_running(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

/// Remove the PID file if it exists. Silently ignores missing files.
pub fn remove_pid(path: &Path) {
    if path.exists() {
        fs::remove_file(path).ok();
        debug!(path = %path.display(), "PID file removed");
    }
}

/// Returns the daemon's PID if it is currently running, or `None` otherwise.
pub fn running_daemon_pid(pid_path: &Path) -> Option<u32> {
    match read_pid(pid_path) {
        Ok(pid) if pid_is_running(pid) => Some(pid),
        _ => None,
    }
}
