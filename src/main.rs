use std::path::PathBuf;
use std::process;

use clap::Parser;
use colored::Colorize;
use tracing_subscriber::EnvFilter;

use fastgit::cli::{Cli, Command};
use fastgit::config::Config;
use fastgit::daemon::ipc::{send_command, IpcCommand, IpcResponse};
use fastgit::daemon::{start_daemon, stop_daemon, DaemonPaths};

fn main() {
    let cli = Cli::parse();

    // Initialise tracing before anything else
    let filter = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)),
        )
        .with_target(false)
        .with_level(true)
        .init();

    let exit_code = run(cli);
    process::exit(exit_code);
}

fn run(cli: Cli) -> i32 {
    let repo_root = resolve_repo_root(cli.repo.as_deref());

    match cli.command {
        // ── Daemon lifecycle ─────────────────────────────────────────────────
        Command::Up(args) => {
            if args.background {
                cmd_up_background(&repo_root, args.config.as_deref())
            } else {
                cmd_up_foreground(&repo_root, args.config.as_deref())
            }
        }

        Command::Down => cmd_down(&repo_root),

        // ── Inspection ───────────────────────────────────────────────────────
        Command::Status => cmd_status(&repo_root),

        Command::Log(args) => cmd_log(&repo_root, args.count),

        // ── Control ──────────────────────────────────────────────────────────
        Command::Pause => cmd_ipc_simple(&repo_root, IpcCommand::Pause, "auto-push paused"),

        Command::Resume => cmd_ipc_simple(&repo_root, IpcCommand::Resume, "auto-push resumed"),

        Command::PushNow => cmd_ipc_simple(&repo_root, IpcCommand::PushNow, "push triggered"),

        // ── Init ─────────────────────────────────────────────────────────────
        Command::Init(args) => cmd_init(&repo_root, args.force),
    }
}

// ============================================================================
// Daemon lifecycle commands
// ============================================================================

fn cmd_up_foreground(repo_root: &PathBuf, config_path: Option<&std::path::Path>) -> i32 {
    let config = match load_config(repo_root, config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            return 3;
        }
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    if let Err(e) = rt.block_on(start_daemon(repo_root.clone(), config)) {
        eprintln!("{} {}", "error:".red().bold(), e);
        return 1;
    }
    0
}

fn cmd_up_background(repo_root: &PathBuf, config_path: Option<&std::path::Path>) -> i32 {
    // Verify config is valid before forking
    if let Err(e) = load_config(repo_root, config_path) {
        eprintln!("{} {}", "error:".red().bold(), e);
        return 3;
    }

    // Re-launch this binary as a detached background process.
    // The child runs `fg up` (without -d) so it goes through cmd_up_foreground.
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{} failed to find current executable: {}", "error:".red().bold(), e);
            return 1;
        }
    };

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("up")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    if let Some(repo) = repo_root.to_str() {
        cmd.args(["--repo", repo]);
    }

    // On Unix, call setsid() in the child to detach from the terminal
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    match cmd.spawn() {
        Ok(child) => {
            println!("daemon started (pid {})", child.id());
            0
        }
        Err(e) => {
            eprintln!("{} failed to start daemon: {}", "error:".red().bold(), e);
            1
        }
    }
}

fn cmd_down(repo_root: &PathBuf) -> i32 {
    let paths = DaemonPaths::new(repo_root);

    if !paths.pid_file.exists() {
        eprintln!(
            "{} no PID file found (daemon may not be running)",
            "error:".red().bold()
        );
        return 2;
    }

    match stop_daemon(&paths.pid_file) {
        Ok(()) => {
            println!("daemon stopped");
            0
        }
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            1
        }
    }
}

// ============================================================================
// Status & log
// ============================================================================

fn cmd_status(repo_root: &PathBuf) -> i32 {
    let paths = DaemonPaths::new(repo_root);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    match rt.block_on(send_command(&paths.socket, IpcCommand::Status)) {
        Ok(IpcResponse::Status(snap)) => {
            println!("{}", snap.render());
            0
        }
        Ok(IpcResponse::Error { message }) => {
            eprintln!("{} {}", "error:".red().bold(), message);
            1
        }
        Ok(_) => {
            eprintln!("{} unexpected response from daemon", "error:".red().bold());
            1
        }
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            2
        }
    }
}

fn cmd_log(repo_root: &PathBuf, count: usize) -> i32 {
    let repo = match git2::Repository::open(repo_root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            return 1;
        }
    };

    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            return 1;
        }
    };

    if revwalk.push_head().is_err() {
        println!("(no commits yet)");
        return 0;
    }

    let mut shown = 0;
    for oid in revwalk.flatten() {
        if shown >= count {
            break;
        }
        if let Ok(commit) = repo.find_commit(oid) {
            let summary = commit.summary().unwrap_or("(no message)");
            // Only show auto-commits created by fg
            if summary.starts_with("auto:") {
                let time = commit.time().seconds();
                let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(time, 0)
                    .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                println!(
                    "{}  {}  {}",
                    &oid.to_string()[..8].yellow(),
                    dt.dimmed(),
                    summary
                );
                shown += 1;
            }
        }
    }

    if shown == 0 {
        println!("no auto-commits found");
    }

    0
}

// ============================================================================
// IPC helpers
// ============================================================================

fn cmd_ipc_simple(repo_root: &PathBuf, cmd: IpcCommand, success_msg: &str) -> i32 {
    let paths = DaemonPaths::new(repo_root);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    match rt.block_on(send_command(&paths.socket, cmd)) {
        Ok(IpcResponse::Ok { .. }) | Ok(IpcResponse::Pong) => {
            println!("{}", success_msg);
            0
        }
        Ok(IpcResponse::Error { message }) => {
            eprintln!("{} {}", "error:".red().bold(), message);
            1
        }
        Ok(_) => {
            eprintln!("{} unexpected response", "error:".red().bold());
            1
        }
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            2
        }
    }
}

// ============================================================================
// Init
// ============================================================================

fn cmd_init(repo_root: &PathBuf, force: bool) -> i32 {
    let config_path = repo_root.join("fg.yml");

    if config_path.exists() && !force {
        eprintln!(
            "{} fg.yml already exists. Use --force to overwrite.",
            "error:".red().bold()
        );
        return 1;
    }

    let content = Config::generate_default();
    if let Err(e) = std::fs::write(&config_path, &content) {
        eprintln!("{} failed to write fg.yml: {}", "error:".red().bold(), e);
        return 1;
    }

    println!("{} created fg.yml", "✓".green().bold());
    println!("  Edit it to customize, then run: {}", "fg up".bold());
    0
}

// ============================================================================
// Helpers
// ============================================================================

fn resolve_repo_root(repo_arg: Option<&std::path::Path>) -> PathBuf {
    repo_arg
        .map(|p| p.to_path_buf())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_config(
    repo_root: &PathBuf,
    config_path: Option<&std::path::Path>,
) -> anyhow::Result<Config> {
    let path = config_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| repo_root.join("fg.yml"));

    Config::load(&path)
}
