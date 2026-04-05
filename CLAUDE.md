# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`gitdaemon` (`gd`) is a declarative background Git sync engine written in Rust. It automates the fetch/stage/commit/push loop so developers don't have to manually run Git rituals. The project is currently in the specification phase — the README.md is the authoritative design document.

## Build & Development Commands

Once the Rust project is initialized:

```sh
cargo build              # compile
cargo build --release    # release build
cargo check              # fast type/borrow check without linking
cargo clippy             # lint
cargo fmt                # format
cargo test               # all tests
cargo test <test_name>   # single test
```

## Architecture

The daemon runs a single `tokio::select!` loop over five concurrent channels:

1. **Filesystem watcher** (`src/watcher.rs`) — wraps the `notify` crate, respects `.gitignore` + `fg.yml` ignore list
2. **Commit ticker** — fires every `commit.interval` seconds (strategy: `time` or `change_count`)
3. **Push ticker** — fires every `push.interval` seconds, batching commits
4. **Fetch ticker** — fires every `repo.fetch_interval` seconds (default: 60s)
5. **IPC channel** — unix socket at `.gd/daemon.sock`, handles `status`/`pause`/`push now` commands from CLI

### Module Layout

```
src/
├── main.rs              # CLI entrypoint
├── cli.rs               # clap command definitions
├── config.rs            # fg.yml schema + serde loader
├── daemon/
│   ├── mod.rs           # daemon lifecycle (start/stop/status)
│   ├── pid.rs           # PID file management
│   ├── ipc.rs           # unix socket IPC
│   └── sync_loop.rs     # main tokio::select! orchestration loop
├── git/
│   ├── mod.rs
│   ├── stage.rs         # auto-stage logic
│   ├── commit.rs        # commit batching + message generation
│   ├── fetch.rs         # background fetch
│   ├── push.rs          # batched push queue
│   └── secrets.rs       # secret scanning (API keys, private key headers) before push
├── watcher.rs           # filesystem watcher wrapper
└── status.rs            # status snapshot + renderer
```

### Configuration (`fg.yml`)

Each tracked repo has an `fg.yml` at its root (created with `gd init`). Key fields:
- `commit.strategy`: `time` (every N seconds if changes exist) or `change_count` (once N files accumulated)
- `safety.block_secrets`: scans the *diff* before push for secret patterns — non-zero exit aborts
- `hooks.pre_commit` / `hooks.post_commit`: shell commands; non-zero exit aborts the commit
- Conflicts are never auto-resolved — daemon pauses push on merge conflict detection

### IPC Protocol

CLI commands (`gd pause`, `gd push now`, `gd status`) communicate with the running daemon via the unix socket at `.gd/daemon.sock`. The `src/daemon/ipc.rs` module owns both the server side (daemon) and client side (CLI).
