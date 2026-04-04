# fastgit (fg)

> **Declarative Git orchestration engine with background synchronization and intelligent commit batching**

A background Git sync engine that keeps your repos continuously in sync — fetching, staging, committing, and pushing without interrupting your flow.

## What it does

- **Constant fetch** — polls remotes in the background so your local refs are always fresh
- **Intelligent auto-stage + commit** — watches for changes and commits them with meaningful messages, respecting `.gitignore` and configurable rules
- **Batched push** — queues commits and pushes in intervals to avoid hammering the remote
- **Instant repo state** — fast status display showing local/remote divergence, staged state, and sync health at a glance

## Why

`git pull`, `git push`, `git status` are manual rituals. `fg` eliminates the ceremony — you write code, it handles the sync loop.

## Design

```
fg/
├── src/
│   ├── main.rs           # CLI entrypoint
│   ├── cli.rs            # clap command definitions
│   ├── config.rs         # fg.yml schema + loader
│   ├── daemon/
│   │   ├── mod.rs        # daemon lifecycle (start/stop/status)
│   │   ├── pid.rs        # PID file management
│   │   ├── ipc.rs        # unix socket IPC
│   │   └── sync_loop.rs  # main orchestration loop
│   ├── git/
│   │   ├── mod.rs
│   │   ├── stage.rs      # auto-stage logic
│   │   ├── commit.rs     # commit batching + message gen
│   │   ├── fetch.rs      # background fetch
│   │   ├── push.rs       # batched push queue
│   │   └── secrets.rs    # secret scanning before push
│   ├── watcher.rs        # filesystem watcher wrapper
│   └── status.rs         # status snapshot + renderer
└── fg.yml                # per-repo config
```

## fg.yml — declarative config

`fg` is driven by a `fg.yml` file at the root of your repo (created with `fg init`):

```yaml
version: 1

repo:
  auto_stage: true
  auto_fetch: true

commit:
  strategy: time       # or: change_count
  interval: 120        # seconds between auto-commits
  message: "auto: {summary}"

push:
  strategy: batch
  interval: 300        # seconds between auto-pushes
  branch: main

ignore:
  - "*.log"
  - "node_modules/"
  - ".env"

safety:
  confirm_first: false
  block_secrets: true  # scans diff for API keys / private keys before push

hooks:
  pre_commit: ""       # e.g. "cargo fmt"
  post_commit: ""      # e.g. "cargo test"
```

## CLI

### Daemon lifecycle

```sh
fg up          # start daemon (foreground)
fg up -d       # start daemon (background, detached)
fg down        # stop daemon
```

### Inspection

```sh
fg status      # show current sync state
fg log         # show auto-commit history
```

### Control

```sh
fg pause       # pause auto-push (staging/committing continues)
fg push now    # force an immediate push
fg init        # create fg.yml in current repo
```

## Status output

```
fg status

⚡ fastgit — /path/to/repo
  branch   main → origin/main
  ahead    3 commits (queued to push in 42s)
  behind   0
  staged   2 files
  watching 1,204 files
  last push 12s ago
  daemon   running (pid 12345)
```

## Commit strategies

| Strategy | Behaviour |
|---|---|
| `time` | commits every N seconds if there are pending changes |
| `change_count` | commits once N files have accumulated changes |

## Daemon internals

The daemon runs a single `tokio::select!` loop over four concurrent channels:

1. **Filesystem watcher** — `notify` crate, respects `.gitignore` + `fg.yml` ignore list
2. **Commit ticker** — fires every `commit.interval` seconds
3. **Push ticker** — fires every `push.interval` seconds
4. **Fetch ticker** — fires every `repo.fetch_interval` seconds (default: 60s)
5. **IPC channel** — unix socket at `.fg/daemon.sock`, handles `status` / `pause` / `push now` from CLI

## Safety

- `block_secrets: true` scans the *diff* of what is about to be committed for API keys, private key headers, and common secret patterns
- Hooks (`pre_commit`, `post_commit`) run as shell commands; a non-zero exit aborts the commit
- `confirm_first: true` prompts before first auto-push in a session
- Conflicts are never auto-resolved — the daemon pauses push if a merge conflict is detected

## Non-goals

- Not a replacement for intentional commits during feature work
- Does not resolve merge conflicts automatically
- Not for monorepos with CI gating every push
