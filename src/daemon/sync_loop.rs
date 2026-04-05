//! Main orchestration loop: `tokio::select!` over five concurrent channels.

use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::daemon::context::DaemonContext;
use crate::daemon::ipc::{IpcCommand, IpcResponse};
use crate::git::commit::{commit_if_ready, CommitAccumulator};
use crate::git::fetch::fetch_all_remotes;
use crate::git::push::PushQueue;
use crate::git::stage::stage_changes;
use crate::git::sync::sync_base_branch;
use crate::git::GitRepo;
use crate::status::StatusSnapshot;
use crate::watcher::{ChangeWatcher, FileEvent};

/// Run the daemon until the shutdown signal fires.
pub async fn run(ctx: DaemonContext) -> anyhow::Result<()> {
    let DaemonContext {
        repo_root,
        config,
        mut ipc_rx,
        mut shutdown_rx,
    } = ctx;

    let git_repo = GitRepo::open(&repo_root)?;
    let branch = config.push.branch.clone();
    let push_queue = PushQueue::new(repo_root.clone());
    let mut commit_acc = CommitAccumulator::new();

    // ── Watcher ─────────────────────────────────────────────────────────────
    let (file_tx, mut file_rx) = mpsc::channel::<FileEvent>(256);
    let _watcher = ChangeWatcher::start(&repo_root, config.ignore.clone(), file_tx)
        .map_err(|e| {
            error!(error = %e, "failed to start filesystem watcher");
            e
        })?;

    // ── Tickers ─────────────────────────────────────────────────────────────
    let mut commit_ticker = interval(Duration::from_secs(config.commit.interval));
    commit_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut push_ticker = interval(Duration::from_secs(config.push.interval));
    push_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut fetch_ticker = interval(Duration::from_secs(config.repo.fetch_interval));
    fetch_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let force_push_notify = push_queue.force_notify();

    // Track watching count
    let mut watching_count: usize = 0;
    // Track recent errors (capped at 5)
    let mut recent_errors: Vec<String> = Vec::new();

    // Track daemon PID
    let daemon_pid = Some(std::process::id());

    info!(
        repo = %repo_root.display(),
        branch = %branch,
        commit_interval = config.commit.interval,
        push_interval = config.push.interval,
        "sync loop started"
    );

    loop {
        tokio::select! {
            // ── 1. File system events ────────────────────────────────────────
            Some(event) = file_rx.recv() => {
                watching_count = watching_count.saturating_add(1);
                debug!(path = %event.path.display(), kind = ?event.kind, "file change");

                // If auto-stage is disabled, just track counts
                if !config.repo.auto_stage {
                    continue;
                }

                match stage_changes(&repo_root, &config.ignore) {
                    Ok(staged) if !staged.is_empty() => {
                        commit_acc.add(staged.len());
                        debug!(count = staged.len(), "staged changes");
                    }
                    Ok(_) => { /* nothing to stage */ }
                    Err(e) => {
                        warn!(error = %e, "staging failed");
                        push_error(&mut recent_errors, e.to_string());
                    }
                }
                let _ = event; // suppress unused warning
            }

            // ── 2. Commit ticker ─────────────────────────────────────────────
            _ = commit_ticker.tick() => {
                let pending = commit_acc.current();
                if pending == 0 && !git_repo.is_dirty().unwrap_or(false) {
                    continue;
                }

                debug!(pending, "commit ticker fired");
                let repo_root_c = repo_root.clone();
                let cfg_c = config.commit.clone();
                let hooks_c = config.hooks.clone();
                let acc_pending = pending;

                match commit_if_ready(repo_root_c, cfg_c, hooks_c, &mut commit_acc, acc_pending).await {
                    Ok(result) if !result.skipped => {
                        info!(oid = ?result.oid, files = result.files_changed, "auto-committed");
                        push_queue.record_commits(1);
                    }
                    Ok(_) => {} // skipped
                    Err(e) => {
                        warn!(error = %e, "commit failed");
                        push_error(&mut recent_errors, e.to_string());
                    }
                }
            }

            // ── 3. Push ticker ───────────────────────────────────────────────
            _ = push_ticker.tick() => {
                do_push(&push_queue, &config, &mut recent_errors).await;
            }

            // ── 4. Force-push notification ───────────────────────────────────
            _ = force_push_notify.notified() => {
                info!("force-push triggered");
                do_push(&push_queue, &config, &mut recent_errors).await;
            }

            // ── 5. Fetch ticker ──────────────────────────────────────────────
            _ = fetch_ticker.tick() => {
                if !config.repo.auto_fetch {
                    continue;
                }
                let root_c = repo_root.clone();
                let fetch_ok = match fetch_all_remotes(root_c).await {
                    Ok(summary) if summary.refs_updated > 0 => {
                        info!(refs = summary.refs_updated, "fetched remote");
                        true
                    }
                    Ok(_) => true,
                    Err(e) => {
                        debug!(error = %e, "fetch failed (non-fatal)");
                        false
                    }
                };

                // Check for conflicts after fetch
                push_queue.check_and_record_conflict().await;

                // After a successful fetch, fast-forward base branch and
                // optionally rebase the current working branch onto it.
                if fetch_ok && config.repo.auto_sync_base {
                    let root_c = repo_root.clone();
                    let base = config.repo.base_branch.clone();
                    let rebase = config.repo.rebase_on_sync;
                    match sync_base_branch(root_c, base.clone(), rebase).await {
                        Ok(r) if r.base_updated => {
                            if r.rebased {
                                info!(
                                    base = %base,
                                    commits = r.base_advanced,
                                    "base updated and current branch rebased"
                                );
                            } else if r.skipped_dirty {
                                info!(
                                    base = %base,
                                    commits = r.base_advanced,
                                    "base updated — rebase skipped (working tree dirty)"
                                );
                            } else {
                                info!(
                                    base = %base,
                                    commits = r.base_advanced,
                                    "fast-forwarded base branch"
                                );
                            }
                        }
                        Ok(_) => {} // nothing moved, no-op
                        Err(e) => {
                            warn!(error = %e, "base sync failed");
                            push_error(&mut recent_errors, format!("base sync: {}", e));
                            // Pause push on rebase conflict so user notices
                            push_queue.pause();
                        }
                    }
                }
            }

            // ── 6. IPC commands ──────────────────────────────────────────────
            Some((cmd, reply)) = ipc_rx.recv() => {
                handle_ipc(
                    cmd,
                    reply,
                    &git_repo,
                    &push_queue,
                    &branch,
                    watching_count,
                    daemon_pid,
                    &recent_errors,
                    &config,
                )
                .await;
            }

            // ── 7. Shutdown signal ───────────────────────────────────────────
            _ = shutdown_rx.changed() => {
                info!("shutdown signal received");
                break;
            }
        }
    }

    info!("sync loop exited cleanly");
    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

async fn do_push(
    push_queue: &PushQueue,
    config: &Config,
    recent_errors: &mut Vec<String>,
) {
    match push_queue.try_push(config).await {
        Ok(Some(result)) if result.success => {
            info!(commits = result.pushed_commits, "pushed");
        }
        Ok(Some(result)) => {
            if result.blocked_by_secrets {
                error!("push blocked by secret scan");
                push_error(
                    recent_errors,
                    "push blocked: secrets detected in diff".to_string(),
                );
            } else if let Some(err) = result.error {
                warn!(error = %err, "push failed");
                push_error(recent_errors, format!("push failed: {}", err));
            }
        }
        Ok(None) => { /* skipped */ }
        Err(e) => {
            warn!(error = %e, "push error");
            push_error(recent_errors, e.to_string());
        }
    }
}

async fn handle_ipc(
    cmd: IpcCommand,
    reply: tokio::sync::oneshot::Sender<IpcResponse>,
    git_repo: &GitRepo,
    push_queue: &PushQueue,
    branch: &str,
    watching_count: usize,
    daemon_pid: Option<u32>,
    recent_errors: &[String],
    _config: &Config,
) {
    let response = match cmd {
        IpcCommand::Ping => IpcResponse::Pong,

        IpcCommand::Status => {
            let push_state = push_queue.state_snapshot();
            match StatusSnapshot::from_repo(
                git_repo,
                branch,
                &push_state,
                watching_count,
                daemon_pid,
                recent_errors.to_vec(),
            ) {
                Ok(snap) => IpcResponse::Status(snap),
                Err(e) => IpcResponse::Error {
                    message: e.to_string(),
                },
            }
        }

        IpcCommand::Pause => {
            push_queue.pause();
            info!("auto-push paused via IPC");
            IpcResponse::Ok {
                message: "auto-push paused".to_string(),
            }
        }

        IpcCommand::Resume => {
            push_queue.resume();
            info!("auto-push resumed via IPC");
            IpcResponse::Ok {
                message: "auto-push resumed".to_string(),
            }
        }

        IpcCommand::PushNow => {
            push_queue.push_now();
            IpcResponse::Ok {
                message: "push queued".to_string(),
            }
        }

        IpcCommand::Shutdown => {
            info!("shutdown requested via IPC");
            IpcResponse::Ok {
                message: "shutting down".to_string(),
            }
            // The shutdown will be handled separately by the caller sending to shutdown_tx
        }
    };

    reply.send(response).ok();
}

fn push_error(errors: &mut Vec<String>, msg: String) {
    errors.push(msg);
    if errors.len() > 5 {
        errors.remove(0);
    }
}
