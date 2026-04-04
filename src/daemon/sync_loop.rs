/// Sync loop for the daemon
pub async fn run(_ctx: crate::daemon::DaemonContext) -> anyhow::Result<()> {
    // Placeholder - would contain the actual tokio::select! loop
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}