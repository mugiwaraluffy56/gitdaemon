# Architecture

This document describes the internal design of `gitdaemon` — how the modules
fit together, how data flows between them, and the rationale behind key design
decisions.

---

## High-level overview

```
┌─────────────────────────────────────────────────────────┐
│                        gd (binary)                      │
│                                                         │
│   CLI parsing (cli.rs)  ──►  Command dispatch (main.rs) │
│                                    │                    │
│              ┌─────────────────────┴──────────────┐     │
│              │                                    │     │
│      Daemon lifecycle                      IPC client   │
│      (daemon/mod.rs)                  (daemon/ipc.rs)   │
│              │                                          │
│              ▼                                          │
│      ┌───────────────┐                                  │
│      │  sync_loop.rs │  tokio::select! over 6 channels  │
│      └──────┬────────┘                                  │
│             │                                           │
│    ┌────────┼────────────────────┐                      │
│    │        │                   │                       │
│  git/     watcher.rs        daemon/ipc.rs               │
│  stage.rs   (notify)         (Unix socket)              │
│  commit.rs                                              │
│  fetch.rs                                               │
│  push.rs                                                │
│  secrets.rs                                             │
│  ai_commit.rs                                           │
└─────────────────────────────────────────────────────────┘
```

---

## Module reference

### `main.rs` — CLI entry point

Parses the `clap` command line, initialises `tracing`, and dispatches to either:
- `daemon::start` for `gd up`
- `daemon::stop` for `gd down`
- `ipc::send_command` for all other commands

### `cli.rs` — command definitions

`clap` structs only. No logic lives here; it is purely the shape of the
command line interface.

### `config.rs` — configuration schema

Deserialises `gd.yml` with `serde_yaml`. Every field has a typed default.
`Config::validate()` runs semantic checks after deserialisation. The module
is standalone — it has no dependencies beyond `serde` and the standard library
(plus `dotenvy` for `.env` loading in `AiConfig::resolve_api_key`).

### `daemon/mod.rs` — lifecycle

`start_daemon` forks a `tokio` runtime and runs the sync loop.
`stop_daemon` reads the PID file and sends `SIGTERM`.

### `daemon/pid.rs` — PID file

Writes and reads `.gd/daemon.pid`. `pid_is_running` checks `/proc/{pid}` on
Linux and uses `kill(pid, 0)` on macOS.

### `daemon/ipc.rs` — Unix socket IPC

The server side runs inside the daemon; the client side is used by the CLI.
The protocol is newline-delimited JSON:

```json
// request
{"cmd": "status"}

// response
{"type": "Status", "data": { ... }}
```

`IpcCommand` and `IpcResponse` are the canonical protocol types. Both sides
share the same file, which keeps the protocol in one place.

### `daemon/context.rs` — daemon context

`DaemonContext` bundles together everything the sync loop needs:
config, repo root, IPC receiver, and shutdown receiver. Passing a single
struct instead of individual fields keeps `sync_loop::run` readable.

### `daemon/sync_loop.rs` — orchestration

The heart of the daemon. A single `tokio::select!` loop with six branches:

| Branch | What it does |
|---|---|
| `file_rx` | File system event → auto-stage |
| `commit_ticker` | Periodic commit tick |
| `push_ticker` | Periodic push tick |
| `force_push_notify` | Immediate push from `gd push` IPC |
| `fetch_ticker` | Fetch + base-branch sync |
| `ipc_rx` | Handle IPC commands from CLI |

Errors in individual branches are logged and recovered — the loop never exits
on a recoverable git error.

### `watcher.rs` — filesystem watcher

Wraps the `notify` crate. Applies a 150 ms debounce to coalesce rapid saves.
Filters events through `.gitignore` (via the `ignore` crate) and the
`gd.yml` ignore list. The `.gd/` directory itself is always excluded.

### `git/stage.rs` — auto-staging

Calls `git add -A` equivalent using `git2`. Respects the ignore patterns from
config. Returns the list of newly staged paths.

### `git/commit.rs` — commit generation

Three responsibilities:
1. **Diff analysis** — walks the staged diff, extracts file deltas and declared
   symbols (structs, enums, traits, functions) using language-specific regexes.
2. **Message generation** — `build_summary` uses the extracted data to produce
   a conventional commit message. When `ai.enabled = true`, `generate_ai_commit_message`
   is called first; `build_summary` is the fallback.
3. **Commit creation** — writes the index tree and creates a `git2` commit
   object, running pre/post hooks around it.

### `git/ai_commit.rs` — AI message generation

Makes an HTTP POST to the Anthropic Messages API with the staged diff.
The API key is resolved from `ai.api_key`, `env:VAR`, or `ANTHROPIC_API_KEY`
in the environment or `.env` file. Any error causes the caller to fall back
to the heuristic generator — AI is always opt-in and never blocks commits.

### `git/fetch.rs` — background fetch

Fetches all configured remotes using `git2`. Returns a summary of how many
refs were updated.

### `git/push.rs` — batched push

`PushQueue` accumulates commit counts between push ticks. The actual push uses
`git2` credential callbacks for SSH key authentication. Detects push rejection
(non-fast-forward) and pauses the queue.

### `git/secrets.rs` — pre-push secret scanning

Scans the full diff text against a set of compiled `Regex` patterns before
every push. Patterns cover AWS keys, GitHub tokens, Stripe keys, Google API
keys, Slack tokens, private key headers, and generic `password=` / `api_key=`
assignments.

### `git/sync.rs` — base branch sync

After a successful fetch, fast-forwards the base branch (e.g. `main`) without
checking it out, then rebases the current branch onto it. Auto-stashes a dirty
working tree before rebasing and pops the stash after.

### `git/undo.rs` — commit undo

Soft-resets HEAD by N commits, returning staged changes to the index. Checks
that the commits look like `gd` auto-commits unless `--force` is passed.

### `git/squash.rs` — commit squash

Combines the last N auto-commits into one. Regenerates the commit message from
the combined diff using the same heuristic as `commit.rs`.

### `status.rs` — status snapshot

`StatusSnapshot` is a point-in-time view of the repository state: branch name,
ahead/behind counts, staged files, last commit, daemon PID, and health enum.
`render()` formats it for the terminal.

### `errors.rs` — error types

Custom `thiserror`-derived error types for cases where structured error
matching is needed beyond `anyhow`.

### `ls.rs` — file listing

Iterates the git index and working tree to produce a categorised list
(staged / modified / untracked / deleted / renamed / conflict) for `gd ls`.

---

## Data flow: auto-commit cycle

```
File saved on disk
       │
       ▼
ChangeWatcher (notify + debounce 150ms)
       │  FileEvent { path, kind }
       ▼
sync_loop: file_rx branch
       │
       ▼  (if auto_stage)
git/stage::stage_changes()
       │  Vec<PathBuf> staged
       ▼
CommitAccumulator::add(n)
       │
       ▼  (on commit_ticker)
git/commit::commit_if_ready()
       │
       ├─► (if ai.enabled)  git/ai_commit::generate_ai_commit_message(diff)
       │         │ Ok(message)  │ Err(_) → fallback
       │         └──────────────┘
       ├─► git/commit::build_summary(deltas, symbols)   ← heuristic
       │
       ▼
git2::Repository::commit()
       │
       ▼
PushQueue::record_commits(1)
       │
       ▼  (on push_ticker or force_push)
git/secrets::scan_for_secrets(diff)
       │  Ok(no secrets)
       ▼
git2::Remote::push()
```

---

## Concurrency model

The daemon is single-threaded from a logic perspective — all state lives in
`sync_loop.rs` and is mutated sequentially inside the `select!` loop. There
is no shared mutable state across tasks; no `Arc<Mutex<…>>` needed.

`git2` calls (which are synchronous and can block) are offloaded to
`tokio::task::spawn_blocking` so they do not stall the event loop.

HTTP calls to the Anthropic API (`ai_commit.rs`) are fully async via `reqwest`
and run directly on the tokio runtime without blocking.

---

## IPC protocol

```
Client                        Daemon
  │                              │
  │──── connect .gd/daemon.sock ─►│
  │──── {"cmd":"status"}\n ───────►│
  │◄─── {"type":"Status","data":{…}}\n ─│
  │                              │
  │──── {"cmd":"push_now"}\n ────►│
  │◄─── {"type":"Ok","message":"queued"}\n ─│
```

The socket is a `tokio::net::UnixListener`. Each connection is served by a
short-lived task. The connection is closed after one request/response pair.

---

## State files

```
<repo-root>/
└── .gd/
    ├── daemon.pid    ← PID of running daemon (absent when stopped)
    └── daemon.sock   ← Unix domain socket (absent when stopped)
```

Both files are created on daemon start and removed on clean shutdown.
`gd up` checks for a stale PID file and warns if the previous daemon did not
exit cleanly.
