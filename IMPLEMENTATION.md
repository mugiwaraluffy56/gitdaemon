# Implementation Plan: fastgit (fg)

## Project Overview

Build a declarative background Git sync engine in Rust that automates the fetch/stage/commit/push loop. The daemon will run a `tokio::select!` loop coordinating 5 concurrent channels with proper error handling, secret scanning, and IPC control.

---

## Phase 0: Project Initialization

### Steps
1. **Initialize Rust project**
   ```sh
   cargo init --bin
   ```
   Creates `Cargo.toml` and `src/main.rs`

2. **Set edition to 2021** in `Cargo.toml`
   ```toml
   [package]
   name = "fastgit"
   version = "0.1.0"
   edition = "2021"
   ```

3. **Add core dependencies** to `Cargo.toml`:

```toml
[dependencies]
tokio = { version = "1.36", features = ["full"] }
tokio-stream = "0.1"
anyhow = "1.0"           # error handling
thiserror = "1.0"        # derive Error
clap = { version = "4.5", features = ["derive"] }
serde = { version = "1.0", features = ["derive"] }
serde_yaml = "0.9"
config = "0.13"          # config loading with defaults
notify = "6.1"
walkdir = "2.5"          # recursive file walking
git2 = "0.19"            # libgit2 bindings (higher-level than git2-commit)
regex = "1.10"           # secret scanning
chrono = { version = "0.4", features = ["serde"] }
dirs = "5.0"            # standard directories
nix = { version = "0.28", features = ["user"] }  # unix socket, PID management
tempfile = "3.10"        # testing utilities
uuid = { version = "1.6", features = ["v4"] }

[dev-dependencies]
assert_cmd = "2.0"       # integration testing
predicates = "3.1"       # test assertions
tempfile = "3.10"
```

---

## Phase 1: Core Types & Configuration

### Files to create (in order):

#### 1. `src/config.rs` — fg.yml schema + loader
**Contains:**
- `struct Config` with all config fields (version, repo, commit, push, ignore, safety, hooks)
- `struct RepoConfig`, `CommitConfig`, `PushConfig`, `SafetyConfig`, `HookConfig`
- `impl Config` with `load(path: &Path) -> anyhow::Result<Self>`
- Default implementations (e.g., `commit.interval: 120`, `push.interval: 300`, `fetch_interval: 60`)
- Validation logic (strategy must be "time" or "change_count")
- `fn validate(&self) -> anyhow::Result<()>`

**Why now:** All modules depend on configuration; this is the foundation.

#### 2. `src/status.rs` — state snapshot + renderer
**Contains:**
- `struct StatusSnapshot` (ahead: usize, behind: usize, staged_files: Vec<String>, last_commit: Option<CommitInfo>, daemon_pid: Option<u32>, etc.)
- `enum SyncHealth` { Healthy, Behind, Stuck, Error }
- `impl StatusSnapshot { fn from_repo(repo_path: &Path) -> anyhow::Result<Self> }`
- `struct StatusRenderer` with `fn render(snapshot: &StatusSnapshot) -> String`
- Terminal formatting (colors, emojis)

**Why now:** Used by IPC status command and daemon diagnostics.

---

## Phase 2: Git Operations Layer

#### 3. `src/git/mod.rs` — git module entry
**Exports:** stage, commit, fetch, push, secrets submodules
**Contains:**
- `struct GitRepo { path: PathBuf }`
- `impl GitRepo { fn open(path: &Path) -> anyhow::Result<Self> }`
- Common utilities (current_branch, remote_name, is_dirty, etc.)

#### 4. `src/git/stage.rs` — auto-stage logic
**Contains:**
- `fn stage_changes(repo: &GitRepo, ignore_patterns: &[String]) -> anyhow::Result<Vec<String>>`
- Uses `walkdir` to find modified files
- Respects `.gitignore` and fg.yml ignore list
- Returns list of staged files
- Handles edge cases (binary files, submodules)

#### 5. `src/git/commit.rs` — commit batching + message generation
**Contains:**
- `struct CommitBatcher { pending_changes: Vec<String>, max_batch_size: usize }`
- `impl CommitBatcher { fn add_changes(&mut self, files: Vec<String>); fn should_commit(&self, strategy: &CommitStrategy) -> bool }`
- `fn generate_commit_message(changes: &[String]) -> String` (e.g., "auto: 5 files changed" or custom template)
- `fn create_commit(repo: &GitRepo, message: &str) -> anyhow::Result<()>`
- Respects `hooks.pre_commit` and `hooks.post_commit` (run as shell commands)

#### 6. `src/git/fetch.rs` — background fetch
**Contains:**
- `async fn fetch_remote(repo: &GitRepo) -> anyhow::Result<()>`
- Fetches all remotes with `git2::fetch`
- Updates local refs
- Returns summary (new commits fetched)

#### 7. `src/git/push.rs` — batched push queue
**Contains:**
- `struct PushQueue { commits: Vec<Commit>, branch: String }`
- `impl PushQueue { fn push(&mut self, repo: &GitRepo) -> anyhow::Result<()> }`
- Push batching logic (push all accumulated commits together)
- Conflict detection: if push fails with MERGING/REJECTED, daemon pauses push
- Respects `push.branch` config

#### 8. `src/git/secrets.rs` — secret scanning
**Contains:**
- `fn scan_for_secrets(diff: &str) -> Vec<SecretMatch>`
- Regex patterns for:
  - AWS keys (`AKIA[0-9A-Z]{16}`)
  - GitHub tokens (`ghp_[0-9a-zA-Z]{36}`)
  - Private key headers (`-----BEGIN (RSA )?PRIVATE KEY-----`)
  - Generic patterns (long hex/base64 strings, `password=`, `api_key=`)
- `struct SecretMatch { line: usize, pattern: String, snippet: String }`
- If `safety.block_secrets = true`, returns error on match, blocking push

---

## Phase 3: Daemon Core

#### 9. `src/daemon/mod.rs` — daemon lifecycle
**Contains:**
- `struct Daemon { config: Config, repo_path: PathBuf, pid_file: PathBuf, ipc_socket: PathBuf }`
- `impl Daemon { async fn start(config_path: &Path, background: bool) -> anyhow::Result<()> }`
- `fn stop(pid_file: &Path) -> anyhow::Result<()>`
- `fn status(pid_file: &Path) -> anyhow::Result<DaemonStatus>`
- PID file management (`.fg/daemon.pid`)
- Background daemonization (fork to background if `-d` flag)
- Socket cleanup on shutdown

#### 10. `src/daemon/pid.rs` — PID file management
**Contains:**
- `fn write_pid(pid: u32, path: &Path) -> anyhow::Result<()>`
- `fn read_pid(path: &Path) -> anyhow::Result<u32>`
- `fn pid_is_running(pid: u32) -> bool` (check `/proc/{pid}` on Unix)
- Cleanup on shutdown

#### 11. `src/daemon/ipc.rs` — unix socket IPC server + client
**Contains:**
- `enum IpcCommand { Status, Pause, PushNow, Shutdown, Ping }`
- `enum IpcResponse { Status(StatusSnapshot), Ok, Error(String) }`
- `async fn start_ipc_server(socket_path: &Path, tx: mpsc::Sender<IpcCommand>) -> anyhow::Result<()>`
  - Listens on `.fg/daemon.sock`
  - Parses commands (JSON protocol: `{"cmd": "status"}`)
  - Sends responses
- `async fn send_command(socket_path: &Path, cmd: IpcCommand) -> anyhow::Result<IpcResponse>`
  - Client-side used by CLI
- Socket path resolution (XDG_RUNTIME_DIR fallback to `.fg/`)

#### 12. `src/daemon/sync_loop.rs` — main orchestration loop
**Contains:**
- `struct SyncLoop { config: Config, repo: GitRepo, ipc_rx: mpsc::Receiver<IpcCommand>, ... }`
- `impl SyncLoop { async fn run(mut self) -> anyhow::Result<()> }`
- The big `tokio::select!` with 5 branches:

```rust
loop {
    tokio::select! {
        // 1. File system events from watcher
        Some(event) = watcher_rx.recv() => {
            handle_file_change(event).await?;
        }

        // 2. Commit ticker
        _ = commit_ticker.tick() => {
            if commit_batcher.should_commit() {
                stage_and_commit().await?;
            }
        }

        // 3. Push ticker
        _ = push_ticker.tick() => {
            if !push_paused && !push_queue.is_empty() {
                push_queue.push().await?;
            }
        }

        // 4. Fetch ticker
        _ = fetch_ticker.tick() => {
            fetch_remote().await?;
        }

        // 5. IPC commands
        Some(cmd) = ipc_rx.recv() => {
            handle_ipc_command(cmd).await?;
        }

        // 6. Shutdown signal
        _ = shutdown_rx.recv() => {
            break;
        }
    }
}
```

- State tracking: `push_paused: bool`, `push_queue: PushQueue`, `commit_batcher: CommitBatcher`
- Error handling: log errors, some are fatal (config parse fail), others recoverable (git op fail)

---

## Phase 4: Supporting Components

#### 13. `src/watcher.rs` — filesystem watcher wrapper
**Contains:**
- `struct ChangeWatcher { watcher: RecommendedWatcher, ignore_patterns: Vec<glob::Pattern>, tx: mpsc::Sender<FileEvent> }`
- `impl ChangeWatcher { fn start(path: &Path, tx: mpsc::Sender<FileEvent>) -> anyhow::Result<Self> }`
- Uses `notify` crate with `RecommendedWatcher`
- Filters events:
  - Respects `.gitignore` and fg.yml ignore patterns
  - Only tracks file modifications (ignore directory moves, metadata)
  - Debounces rapid events (e.g., editors writing temp files)
- Converts events to `FileEvent { path: PathBuf, event_type: FileChangeType }`

**Note:** `.fg/` directory should be ignored (daemon's own state)

#### 14. `src/cli.rs` — clap command definitions
**Contains:**
- `#[derive(Parser)] struct Cli { command: Command, ... }`
- `enum Command { Up, Down, Status, Log, Pause, PushNow, Init }`
- Subcommand parsing with flags:
  - `Up { background: bool, config: Option<PathBuf> }`
  - `Status { format: OutputFormat }`
  - `PushNow { force: bool }`
  - `Init { force: bool }`
- `fn parse() -> Cli`

#### 15. `src/main.rs` — CLI entrypoint
**Contains:**
- `#[tokio::main] async fn main() -> anyhow::Result<()>`
- Command dispatch:
  - `up` → `Daemon::start().await`
  - `down` → `Daemon::stop()`
  - `status` → `send_command(IpcCommand::Status).await` → render output
  - `push now` → `send_command(IpcCommand::PushNow).await`
  - `pause` → `send_command(IpcCommand::Pause).await`
  - `init` → create default `fg.yml` in current repo
- Error handling: print user-friendly messages to stderr, exit codes:
  - 0: success
  - 1: general error
  - 2: daemon not running
  - 3: config error

---

## Phase 5: Utilities & Testing

#### 16. Test files
- `tests/integration.rs` — full daemon lifecycle tests:
  - `test_daemon_start_stop()`
  - `test_auto_commit_and_push()`
  - `test_secret_scanning_blocks_push()`
  - `test_hooks_executed()`
  - `test_ipc_status_command()`
  - `test_ignore_patterns()`
- `tests/common.rs` — test fixtures:
  - `fn setup_test_repo() -> TempDir` (creates temp git repo with files)
  - `fn create_modified_file(repo: &Path)`
  - `fn wait_for_daemon_state(condition: impl Fn(&StatusSnapshot) -> bool)`
- Uses `assert_cmd::Command` to spawn `fg` binary
- Uses `git2::init_repository` for test repos

#### 17. `src/errors.rs` (optional enhancement)
If error types become complex, define:
- `enum FastgitError { ConfigError(String), GitError(git2::Error), IoError(std::io::Error), SecretDetected(SecretMatch) }`
- `impl std::error::Error for FastgitError`
- `impl From<git2::Error> for FastgitError`, etc.

---

## Implementation Order (Recommended)

**Day 1: Foundation**
1. Create `Cargo.toml` with all dependencies
2. `src/main.rs` + `src/cli.rs` (CLI scaffolding, parse commands, print "not implemented" stubs)
3. `src/config.rs` (full implementation with defaults and validation)
4. `tests/common.rs` (test repo fixture)
5. Test: `fg init` creates valid `fg.yml`

**Day 2: Git Ops Layer**
6. `src/git/mod.rs` (repo wrapper)
7. `src/git/stage.rs` (staging with ignore support)
8. `src/git/commit.rs` (commit creation + hooks)
9. `src/git/secrets.rs` (regex scanning)
10. Test: staging, committing, secret detection

**Day 3: Fetch & Push**
11. `src/git/fetch.rs` (background fetch)
12. `src/git/push.rs` (batched push + conflict handling)
13. Test: fetch updates, push batching, conflict pause

**Day 4: Daemon Core**
14. `src/daemon/mod.rs` (daemon lifecycle, PID)
15. `src/daemon/pid.rs`
16. `src/watcher.rs` (filesystem events)
17. `src/daemon/sync_loop.rs` (orchestration)
18. Test: daemon start/stop, event loop

**Day 5: IPC & CLI Integration**
19. `src/daemon/ipc.rs` (unix socket server + client)
20. Wire IPC to `main.rs` commands
21. `src/status.rs` (snapshot + renderer)
22. Test: `fg status`, `fg push now`, `fg pause`

**Day 6: Polish & Edge Cases**
23. Background daemonization (`-d` flag)
24. Signal handling (SIGTERM/SIGINT cleanup)
25. Robust error recovery (git failures, network timeouts)
26. Logging (tracing/log crate for structured logs)
27. Documentation (help text, README updates)

---

## Testing Strategy

### Unit Tests
- Each module has `#[cfg(test)] mod tests` with:
  - Config parser: invalid YAML, missing fields, defaults
  - Secret scanner: positive/negative cases for each pattern
  - Stage logic: .gitignore parsing, custom ignore patterns
  - Commit batcher: strategy evaluation, message generation

### Integration Tests (tests/integration.rs)
Run against real temporary Git repos:
1. **Basic flow**: daemon starts, file change → staged → committed → pushed
2. **Secret blocking**: create file with AWS key, verify push blocked, daemon logs error
3. **Hooks**: pre_commit returns non-zero → commit aborted
4. **IPC**: send `status` → receives valid snapshot; `push now` → immediate push
5. **Ignore patterns**: files matching ignore list are not staged
6. **Daemon lifecycle**: `fg up -d` background start, `fg down` stops cleanly, PID removed
7. **Conflict handling**: simulate remote change, verify daemon pauses push

### Test Fixtures
- `setup_test_repo()`: tempdir with `git2::init`, creates 3 files
- Mock time via `tokio::time::pause()` for ticker tests
- Use `tempfile::TempDir` auto-cleanup

---

## Build & Run Instructions

### Prerequisites
```sh
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Verify
rustc --version  # rustc 1.75+
cargo --version
```

### Building
```sh
cd /Users/puneethadityamyakam/projects/fg

# Development build (debug)
cargo build

# Release build (optimized)
cargo build --release
./target/release/fg --help

# Run tests
cargo test
cargo test -- --nocapture  # see logs
```

### Running in a Repository

```sh
cd /path/to/your/repo

# 1. Initialize config
./target/debug/fg init

# 2. Start daemon (foreground)
./target/debug/fg up

# 3. In another terminal, check status
./target/debug/fg status

# 4. Make a file change, watch it auto-commit/push
echo "test" >> README.md

# 5. Stop daemon
./target/debug/fg down
```

### Background Daemon
```sh
./target/debug/fg up -d  # starts in background
fg status               # CLI communicates via IPC
fg down                 # stops daemon
```

### Debugging
```sh
# Run with RUST_LOG=debug
RUST_LOG=debug cargo run -- up

# Check daemon state
ps aux | grep fastgit
ls -la .fg/
cat .fg/daemon.log  # if we add file logging
```

---

## Core Data Structures Summary

```rust
// config.rs
pub struct Config {
    pub version: u32,
    pub repo: RepoConfig,
    pub commit: CommitConfig,
    pub push: PushConfig,
    pub ignore: Vec<String>,
    pub safety: SafetyConfig,
    pub hooks: HookConfig,
}
pub struct CommitConfig { pub strategy: String, pub interval: u64, pub message: String }
pub struct PushConfig { pub interval: u64, pub branch: String }
pub struct SafetyConfig { pub confirm_first: bool, pub block_secrets: bool }
pub struct HookConfig { pub pre_commit: String, pub post_commit: String }

// git/stage.rs
pub fn stage_changes(repo: &GitRepo, ignore_patterns: &[String]) -> anyhow::Result<Vec<PathBuf>>

// git/commit.rs
pub struct CommitBatcher { pending_files: Vec<PathBuf>, strategy: CommitStrategy, max_age: Duration }
pub enum CommitStrategy { Time(u64), ChangeCount(usize) }

// git/secrets.rs
pub struct SecretMatch { pub line: usize, pub pattern: &'static str, pub context: String }

// watcher.rs
pub struct ChangeWatcher { tx: mpsc::Sender<FileEvent>, /* ... */ }
pub struct FileEvent { pub path: PathBuf, pub change_type: FileChangeType }

// daemon/sync_loop.rs
pub struct SyncLoopState {
    pub config: Config,
    pub repo: GitRepo,
    pub commit_batcher: CommitBatcher,
    pub push_queue: PushQueue,
    pub push_paused: bool,
    pub ipc_rx: mpsc::Receiver<IpcCommand>,
    pub shutdown_rx: watch::Receiver<()>,
}

// daemon/ipc.rs
pub enum IpcCommand { Status, Pause, PushNow, Shutdown }
pub enum IpcResponse { Status(StatusSnapshot), Ok, Error(String) }

// status.rs
pub struct StatusSnapshot {
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub staged_files: Vec<String>,
    pub last_commit: Option<CommitInfo>,
    pub daemon_pid: Option<u32>,
    pub health: SyncHealth,
}
```

---

## Trait Boundaries & Module Responsibilities

```
┌─────────────┐
│   main.rs   │ CLI parsing → dispatches to daemon or IPC client
└──────┬──────┘
       │
       ├─────────────┐
       │   cli.rs    │ clap structs only, no logic
       └─────────────┘
       │
       ├─────────────┐
       │  config.rs  │ Load/validate fg.yml, provide defaults
       └─────────────┘
       │
       ▼
┌──────────────────────────────────────────┐
│           Daemon (sync_loop.rs)          │
│  orchestrates: watcher, tickers, IPC     │
└─────┬────────┬────────┬────────┬────────┘
      │        │        │        │
      ▼        ▼        ▼        ▼
  watcher  commit   push    fetch
      │        │        │        │
      └────────┴────────┴────────┘
             git/ module
           ┌─────────────┐
           │  stage.rs   │
           │  commit.rs  │
           │  push.rs    │
           │  fetch.rs   │
           │ secrets.rs  │
           └─────────────┘
```

### Module Boundaries

- **config.rs**: standalone, no deps beyond serde
- **status.rs**: depends on config, git2
- **git/\***: pure Git operations, no IPC/daemon deps
- **watcher.rs**: depends on config (ignore patterns), sends events to daemon
- **daemon/\***: depends on config, git, watcher, status
- **ipc.rs**: shared protocol, used by daemon (server) and CLI (client)

---

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| `git2` API complexity | Start with simple `git add/commit/push` commands, add robust error handling, wrapper functions |
| Filesystem watch storms | Debounce events (100ms), coalesce batches, respect existing `.gitignore` |
| Unix socket conflicts | Use unique socket in `.fg/`, clean up on daemon start |
| Daemon becomes unresponsive | Add health check endpoint in IPC, auto-restart logic |
| Secret scanning false positives | Use conservative patterns, allow false positive via config escape hatch |

---

## Success Criteria

- [ ] `fg init` creates valid `fg.yml`
- [ ] `fg up` starts daemon, writes PID, opens IPC socket
- [ ] File changes detected → auto-staged within 5s
- [ ] Commit ticker fires → creates commit with correct message
- [ ] Push ticker fires → pushes to remote (batched)
- [ ] `fg status` shows accurate sync state
- [ ] `fg pause` stops auto-push (staging/committing continues)
- [ ] `fg push now` forces immediate push
- [ ] Secret scanning blocks push when API key detected in diff
- [ ] Daemon can be stopped with `fg down`, PID/socket cleaned
- [ ] All integration tests pass

---

## Extensibility (Post-MVP)

- Multiple remote support (push to all remotes)
- Commit strategy templates (conventional commits, emoji)
- Push strategy: per-branch, time windows, CI gate
- Webhooks (notify on push)
- TUI dashboard (crossterm/ratatui)
- Windows support (named pipes instead of unix socket)
