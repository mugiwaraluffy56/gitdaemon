//! CLI command definitions using `clap`.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// fastgit — declarative background Git sync engine.
#[derive(Parser, Debug)]
#[command(
    name = "gd",
    version,
    about = "Declarative background Git sync engine",
    long_about = "gd keeps your repos continuously in sync — fetching, staging, committing,\nand pushing without interrupting your flow."
)]
pub struct Cli {
    /// Path to the repository root (defaults to the current directory).
    #[arg(short, long, global = true, value_name = "PATH")]
    pub repo: Option<PathBuf>,

    /// Increase logging verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the sync daemon.
    Up(UpArgs),

    /// Stop the running daemon.
    Down,

    /// Show current sync state.
    Status,

    /// Show recent auto-commit history.
    Log(LogArgs),

    /// Pause auto-push (staging and committing continue).
    Pause,

    /// Resume auto-push after a pause.
    Resume,

    /// Force an immediate push.
    #[command(name = "push")]
    PushNow,

    /// Create a default fg.yml in the current repository.
    Init(InitArgs),

    /// Undo the last N auto-commits (soft reset back to index).
    Undo(UndoArgs),

    /// Squash the last N auto-commits into one clean commit.
    Squash(SquashArgs),

    /// List files being tracked / watched by the daemon.
    Ls,
}

#[derive(Parser, Debug)]
pub struct UndoArgs {
    /// Number of commits to undo (default: 1).
    #[arg(default_value = "1")]
    pub count: usize,

    /// Undo even if the commit doesn't look like an fg auto-commit.
    #[arg(long)]
    pub force: bool,
}

#[derive(Parser, Debug)]
pub struct SquashArgs {
    /// Number of recent commits to squash together.
    pub count: usize,
}

#[derive(Parser, Debug)]
pub struct UpArgs {
    /// Start the daemon in the background (detached).
    #[arg(short = 'd', long)]
    pub background: bool,

    /// Path to fg.yml (defaults to `<repo>/fg.yml`).
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct LogArgs {
    /// Number of commits to show.
    #[arg(short = 'n', long, default_value = "10")]
    pub count: usize,

    /// Follow mode: watch for new auto-commits in real time (like `tail -f`).
    #[arg(short = 'f', long)]
    pub follow: bool,

    /// Show all commits, not just fg auto-commits.
    #[arg(long)]
    pub all: bool,
}

#[derive(Parser, Debug)]
pub struct InitArgs {
    /// Overwrite an existing fg.yml.
    #[arg(long)]
    pub force: bool,
}
