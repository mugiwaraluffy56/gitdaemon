# fastgit User Guide

This guide walks through every feature of `fg`, from getting started to advanced daily use.

---

## Table of contents

1. [Installation](#1-installation)
2. [Initialising a repository](#2-initialising-a-repository)
3. [Starting the daemon](#3-starting-the-daemon)
4. [How auto-staging works](#4-how-auto-staging-works)
5. [How auto-committing works](#5-how-auto-committing-works)
6. [Commit message generation](#6-commit-message-generation)
7. [How auto-pushing works](#7-how-auto-pushing-works)
8. [Background fetch](#8-background-fetch)
9. [Secret scanning](#9-secret-scanning)
10. [Hooks](#10-hooks)
11. [Controlling the daemon at runtime](#11-controlling-the-daemon-at-runtime)
12. [Reading the status display](#12-reading-the-status-display)
13. [Viewing the auto-commit log](#13-viewing-the-auto-commit-log)
14. [Stopping the daemon](#14-stopping-the-daemon)
15. [Conflict handling](#15-conflict-handling)
16. [Background mode (-d)](#16-background-mode--d)
17. [Logging and debugging](#17-logging-and-debugging)
18. [Auto-sync base branch](#18-auto-sync-base-branch)

---

## 1. Installation

### Build from source

```sh
git clone https://github.com/fastgit/fg
cd fg
cargo build --release
cp target/release/fg ~/.local/bin/
```

### Requirements

| Requirement | Notes |
|---|---|
| Rust 1.75+ | `rustup update stable` to upgrade |
| libgit2 | Bundled by the `git2` crate — no separate install required |
| SSH keys | `~/.ssh/id_ed25519`, `id_rsa`, or `id_ecdsa` for SSH remotes |
| Git user config | `git config --global user.name` and `user.email` must be set — `fg` uses them for commits |

---

## 2. Initialising a repository

Run `fg init` inside any Git repository:

```sh
cd ~/projects/my-project
fg init
```

This writes a `fg.yml` config file at the repo root with sensible defaults. If one already exists, `fg init` will refuse to overwrite it unless you pass `--force`:

```sh
fg init --force   # overwrite an existing fg.yml
```

The generated file includes full comments — open it and read through before starting the daemon.

---

## 3. Starting the daemon

```sh
fg up            # foreground (Ctrl-C to stop)
fg up -d         # detached background process
```

On startup the daemon:
1. Reads and validates `fg.yml`
2. Writes its PID to `.fg/daemon.pid`
3. Opens the IPC socket at `.fg/daemon.sock`
4. Starts the filesystem watcher
5. Enters the main `tokio::select!` loop

> **Note:** `fg up` looks for `fg.yml` in the current directory by default.  
> Use `--repo /path/to/repo` to point it at a different directory:
> ```sh
> fg up --repo ~/projects/other-project
> ```

---

## 4. How auto-staging works

When a file in the working tree changes, the filesystem watcher fires within ~150 ms (after debouncing). `fg` then runs the equivalent of `git add` on all modified, new, and deleted files — **but skips**:

- Anything matching patterns in `fg.yml`'s `ignore` list  
- Anything already excluded by `.gitignore`  
- Everything inside `.fg/` (daemon state) and `.git/` (git internals)

```yaml
# fg.yml
ignore:
  - "*.log"       # any .log file in the repo
  - "node_modules/"
  - ".env"
  - "dist/"
  - "target/"     # Rust build output
```

You can disable auto-staging entirely if you prefer to stage manually:

```yaml
repo:
  auto_stage: false
```

With `auto_stage: false`, the daemon still auto-commits and auto-pushes **what is already staged**, but will not call `git add` on your behalf.

---

## 5. How auto-committing works

After staging, `fg` waits for a **commit trigger** — either a timer or a file-count threshold, depending on your configured strategy.

### Strategy: `time` (default)

```yaml
commit:
  strategy: time
  interval: 120    # commit every 2 minutes if there are staged changes
```

Every `interval` seconds, if there is anything staged, `fg` creates a commit. If nothing is staged, the tick is a no-op.

### Strategy: `change_count`

```yaml
commit:
  strategy: change_count
  change_threshold: 10   # commit once 10 or more files have accumulated
```

`fg` counts files as they are staged. Once the count reaches `change_threshold`, it commits everything — regardless of how much time has elapsed.

> **Which should I use?**  
> Use `time` (the default) when you want regular checkpoints regardless of velocity.  
> Use `change_count` when you're on a slow machine or doing bulk edits and don't want commits on every small save.

---

## 6. Commit message generation

`fg` generates **conventional commit** messages by reading the actual diff — not just file names.

### What it reads

For every file in the staged diff, `fg` scans `+` and `-` lines for public symbol declarations in any of these languages: **Rust, TypeScript, JavaScript, Python, Go**.

Patterns recognised:
- `pub struct Foo`, `pub enum Bar`, `pub trait Baz` → type declarations
- `pub fn foo`, `pub async fn bar` → function declarations  
- `export class Foo`, `export function bar` → TypeScript/JS
- `def foo`, `class Foo` → Python
- `func Foo`, `type Foo struct` → Go

### How the message is built

```
<type>(<scope>): <subject>
```

**Type** — inferred from what changed:

| Type | Condition |
|---|---|
| `feat` | New public types (`struct`/`enum`/`trait`) added to source |
| `feat` | Majority of deltas are new source files |
| `fix` | Majority of deltas are modifications to existing source |
| `refactor` | Mix of adds + deletes, or all renames |
| `chore` | All deletions |
| `test` | All test files (`tests/`, `*_test.*`, `test_*`) |
| `docs` | All documentation files (`.md`, `.rst`, `README`, …) |
| `build` | Build/config files (`Cargo.toml`, `package.json`, `Makefile`, …) |

**Scope** — the first meaningful subdirectory under `src/`, when ≥ 60% of files share it:

| Files changed | Scope |
|---|---|
| `src/git/push.rs`, `src/git/fetch.rs` | `git` |
| `src/daemon/ipc.rs` | `daemon` |
| `src/config.rs` | `config` |
| `README.md` | *(none)* |

**Subject** — built from the extracted symbols:

| Situation | Verb | Example |
|---|---|---|
| New `struct`/`enum`/`trait` | `introduce` | `introduce PushQueue` |
| New `fn`/`async fn` | `implement` | `implement fetch_all_remotes` |
| Same symbol in `+` and `-` | `rework` | `rework validate` |
| Symbol only in `-` | `drop` | `drop legacy_fetch` |
| No symbols found (fallback) | file-based | `update config` |

PascalCase type names are kept as-is (`PushQueue`, `IpcCommand`).  
snake_case function names are expanded to English (`fetch_all_remotes` → "fetch all remotes").  
Known abbreviations are expanded: `ipc` → `IPC`, `pid` → `PID`, `ctx` → `context`, `cfg` → `config`, etc.

### Real examples

```
feat(git): introduce PushQueue and implement try_push
feat(ipc): introduce IpcCommand and IpcResponse, implement send_command
fix(config): rework validate
refactor(daemon): introduce DaemonContext and implement run
chore: drop stale lock files
test(secrets): implement test_aws_key_detected and test_github_token_detected
docs: update README
build: update Cargo.toml
```

For changesets larger than 5 files, a body is appended:

```
feat(git): introduce PushQueue and implement try_push

- add(git): PushQueue, PushState, PushResult
- add(git): try_push, record_commits, push_now
- update src/git/mod.rs
```

### Customising the template

By default the generated message is used verbatim (`message: "{summary}"`). You can wrap it:

```yaml
commit:
  message: "[{branch}] {summary}"
  # → "[main] feat(git): introduce PushQueue"

  message: "{summary} ({count} files changed)"
  # → "feat(git): introduce PushQueue (3 files changed)"

  message: "{summary}\n\nTimestamp: {timestamp}"
  # → multiline with ISO-8601 timestamp in body
```

Available tokens:

| Token | Value |
|---|---|
| `{summary}` | Full generated conventional commit line |
| `{count}` | Number of files changed in this commit |
| `{branch}` | Current branch name |
| `{timestamp}` | ISO-8601 UTC timestamp (`2025-04-01T12:00:00Z`) |

---

## 7. How auto-pushing works

After a commit is created, it is added to the push queue. The push ticker fires every `push.interval` seconds and sends everything in the queue to `origin/<branch>`:

```yaml
push:
  strategy: batch
  interval: 300     # push every 5 minutes
  branch: main      # which branch to push
```

### Credential handling

`fg` tries credentials in this order:
1. SSH agent (via `ssh-agent`)
2. `~/.ssh/id_ed25519`
3. `~/.ssh/id_rsa`
4. `~/.ssh/id_ecdsa`
5. Git credential helper (for HTTPS remotes)

### Failure backoff

If a push fails 3 times in a row, `fg` **automatically pauses** auto-push and logs an error. Use `fg resume` to re-enable it after fixing the underlying issue (network, auth, conflict).

---

## 8. Background fetch

`fg` fetches from all remotes on a configurable interval:

```yaml
repo:
  auto_fetch: true
  fetch_interval: 60    # seconds between fetches (default 60)
```

This keeps your local remote-tracking branches (`origin/main`, etc.) fresh without you running `git fetch`. It does **not** merge or rebase — it only updates the refs. After each fetch, `fg` checks for merge conflicts and pauses push if one is detected.

To disable background fetch:

```yaml
repo:
  auto_fetch: false
```

---

## 9. Secret scanning

With `safety.block_secrets: true` (the default), `fg` scans the diff before every push. If any of the following patterns are found on `+` lines, the push is **blocked**:

| Pattern | Detected secret |
|---|---|
| `AKIA[0-9A-Z]{16}` | AWS Access Key ID |
| `aws_secret_access_key = ...` | AWS Secret Access Key |
| `sk_live_...` | Stripe live secret key |
| `ghp_...` | GitHub Personal Access Token |
| `ghs_...` | GitHub App Token |
| `-----BEGIN ... PRIVATE KEY-----` | Private key header (RSA, EC, OPENSSH) |
| `AIza...` | Google API Key |
| `xox...` | Slack token |
| `password = "..."` (8+ chars) | Hardcoded password |
| `api_key = "..."` etc. | Hardcoded API key / token |

When a hit is found:
- The push is blocked
- An error is logged to `fg status`'s recent errors list
- The commit **remains on your local branch** — it is not rolled back

To fix: scrub the secret from your working tree, amend or revert the commit, and let `fg` push cleanly.

To disable (not recommended):

```yaml
safety:
  block_secrets: false
```

---

## 10. Hooks

```yaml
hooks:
  pre_commit: "cargo fmt --check && cargo clippy -- -D warnings"
  post_commit: "cargo test --quiet"
```

### `pre_commit`

Runs **before** the commit is created. If the command exits non-zero:
- The commit is **aborted**
- Staged files remain staged — nothing is lost
- The skip reason is recorded in the commit result

Common uses:
```yaml
pre_commit: "cargo fmt --check"          # abort if code isn't formatted
pre_commit: "prettier --check ."         # JS formatting gate
pre_commit: "python -m pytest -q"        # run tests before committing
```

### `post_commit`

Runs **after** the commit is created. If the command exits non-zero:
- The failure is **logged** but the commit is not rolled back
- The daemon continues normally

Common uses:
```yaml
post_commit: "cargo test"                # run tests after every commit
post_commit: "make build"                # trigger a build
```

Both hooks run with `sh -c "<command>"` and inherit the daemon's environment. The working directory is set to the repo root.

---

## 11. Controlling the daemon at runtime

All control commands communicate with the running daemon over the Unix socket at `.fg/daemon.sock`.

### Pause auto-push

```sh
fg pause
```

Stops automatic pushing. Auto-staging and auto-committing continue normally. Use this when you need to do local cleanup before the next push goes out.

### Resume auto-push

```sh
fg resume
```

Re-enables auto-push after a pause.

### Force immediate push

```sh
fg push
```

Triggers a push right now, bypassing the timer. The daemon runs the same push logic (including secret scanning) as a scheduled push.

---

## 12. Reading the status display

```sh
fg status
```

```
⚡ fastgit — /home/alice/projects/my-project
  branch   main → origin/main
  ahead    3 commits (queued to push, last push 42s ago)
  behind   0
  staged   2 files
  watching 1,204 files
  daemon   running (pid 12345)
```

| Field | Description |
|---|---|
| Icon | ⚡ healthy · ⬇ behind remote · ⏸ push paused · ✗ error |
| `branch` | Local branch and its remote tracking ref |
| `ahead` | Commits created locally but not yet pushed |
| `behind` | Commits on the remote not yet fetched locally |
| `staged` | Files currently in the index |
| `watching` | Number of files the watcher is tracking |
| `daemon` | Running / not running, with PID |

If there are recent errors (failed push, secret hit, hook failure) they appear at the bottom in red.

---

## 13. Viewing the auto-commit log

```sh
fg log          # last 10 auto-commits
fg log -n 25    # last 25
```

Shows only commits created by `fg` (those where the message was generated by the conventional commit generator). Output:

```
a3f2b1c0  2025-04-01 14:32:11  feat(git): introduce PushQueue and implement try_push
d9e17aa2  2025-04-01 14:28:05  fix(config): rework validate
```

---

## 14. Stopping the daemon

```sh
fg down
```

Sends `SIGTERM` to the daemon process. The daemon:
1. Exits the select loop cleanly
2. Removes `.fg/daemon.pid`
3. Removes `.fg/daemon.sock`

If the daemon is not running, `fg down` reports an error and exits with code 2.

---

## 15. Conflict handling

When a merge conflict is detected (the repository enters `Merge` state), `fg` automatically pauses auto-push. Auto-staging and auto-committing continue, so your work is still checkpointed locally.

To resume after resolving:
1. Resolve the conflict normally (`git mergetool`, manual edit, etc.)
2. Stage the resolved files (`git add` or let `fg` stage them)
3. `fg resume` — this clears the conflict flag and re-enables push

---

## 16. Background mode (-d)

```sh
fg up -d
```

Launches the daemon as a detached background process using `setsid`. The parent process prints the child PID and exits immediately:

```
daemon started (pid 18432)
```

The background daemon reads the same `fg.yml` and writes to the same `.fg/` directory. Use `fg status`, `fg pause`, etc. to interact with it. `fg down` stops it.

To check if a background daemon is running:

```sh
fg status     # shows "daemon   running (pid 18432)"
# or
cat .fg/daemon.pid && kill -0 $(cat .fg/daemon.pid)
```

---

## 17. Logging and debugging

`fg` uses structured logging via the `tracing` crate. Control verbosity with:

```sh
# Foreground daemon with info-level logs
fg up -v

# Debug logs (shows staging, commit, push details)
fg up -vv

# Trace logs (shows every file event and IPC message)
fg up -vvv

# Override via environment variable (takes precedence over -v flags)
RUST_LOG=debug fg up
RUST_LOG=fastgit=trace,git2=warn fg up
```

Log format:
```
2025-04-01T14:32:11Z INFO  committed oid=a3f2b1c0 files=3 message="feat(git): introduce PushQueue"
2025-04-01T14:33:05Z INFO  pushed branch=main commits=1
2025-04-01T14:34:00Z INFO  fetched remote=origin refs_updated=2
```

The `.fg/` directory does **not** currently write a log file — all output goes to stdout/stderr. To capture logs from a background daemon, redirect when launching:

```sh
fg up -vv > .fg/daemon.log 2>&1 &
```

Or use `fg up -d` and check stderr via your process supervisor.

---

## 18. Auto-sync base branch

When you're working on a feature branch, `fg` can automatically keep `main` (or `master`) up to date and rebase your branch onto it — so you're always developing on a fresh base without ever running `git fetch`, `git checkout main`, `git pull`, or `git rebase main` manually.

### What it does

Every time the fetch ticker fires, `fg` runs three steps:

1. **Fetch** from all remotes (standard background fetch)
2. **Fast-forward `main`** — updates the local `main` ref to match `origin/main` without checking it out. This is a pure ref update, so your working tree is completely untouched.
3. **Rebase your branch** — if your working tree is clean (no unsaved edits), runs `git rebase main` to put your commits on top of the new base.

### When `fg` skips the rebase

The rebase is skipped — safely, automatically — in these situations:

| Situation | What happens |
|---|---|
| You have unsaved edits (working tree dirty) | Skip this cycle, retry on next fetch tick |
| You're already on `main` | Nothing to rebase — only fast-forward runs |
| Repo is in mid-rebase / mid-merge state | Skip until state is clean |
| `main` didn't advance | Nothing to do |

If a rebase conflict occurs, `fg` **immediately aborts** the rebase (running `git rebase --abort`), pauses auto-push, and logs an error to `fg status`. Your working tree is restored exactly as it was. You resolve the conflict manually, then `fg resume`.

### Configuration

```yaml
repo:
  auto_fetch: true          # fetch must be enabled for sync to run
  fetch_interval: 60        # seconds between fetch+sync cycles

  auto_sync_base: true      # enable base-branch sync (default: true)
  base_branch: main         # the branch to keep updated (default: "main")
  rebase_on_sync: true      # rebase current branch after base advances (default: true)
```

To just fast-forward `main` without rebasing (less disruptive, you merge manually when ready):

```yaml
repo:
  auto_sync_base: true
  base_branch: main
  rebase_on_sync: false
```

To disable the feature entirely:

```yaml
repo:
  auto_sync_base: false
```

### Typical workflow

```
# You create a feature branch and start working
git checkout -b feature/my-thing

# Start fg
fg up -d

# Work normally — edit files, fg auto-commits
# Meanwhile, behind the scenes every 60s:
#   - fg fetches origin
#   - fg fast-forwards main  (+3 commits from teammates)
#   - fg rebases feature/my-thing onto the new main
#   (only when your tree is clean between saves)

# When you're done, push and open a PR
fg push
```

Your branch is always close to `main`, merge conflicts are caught early (one commit at a time rather than a massive divergence), and you never had to context-switch to run Git plumbing commands.
