/// Daemon context
pub struct DaemonContext {
    pub repo_root: std::path::PathBuf,
    pub config: crate::config::Config,
    pub ipc_server: crate::daemon::IpcServer,
}
