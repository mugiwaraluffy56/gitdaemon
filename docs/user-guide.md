# gitdaemon (gd) User Guide

This guide walks through every feature of `gd`, from getting started to advanced daily use.

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
19. [Auto-stash before rebase](#19-auto-stash-before-rebase)
20. [Branch-push guard](#20-branch-push-guard)
21. [Listing tracked files (`gd ls`)](#21-listing-tracked-files-gd-ls)
22. [Undoing auto-commits (`gd undo`)](#22-undoing-auto-commits-gd-undo)
23. [Squashing auto-commits (`gd squash`)](#23-squashing-auto-commits-gd-squash)
24. [Real-time commit log (`gd log -f`)](#24-real-time-commit-log-gd-log--f)
25. [Notification hooks](#25-notification-hooks)

---

## 1. Installation

### Build from source

```sh
git clone https://github.com/mugiwaraluffy56/gitdaemon
cd gd
cargo build --release
cp target/release/gd ~/.local/bin/
```

### Requirements

| Requirement | Notes |
|---|---|
| Rust 1.75+ | `rustup update stable` to upgrade |
| libgit2 | Bundled by the `git2` crate — no separate install required |
| SSH keys | `~/.ssh/id_ed25519`, `id_rsa`, or `id_ecdsa` for SSH remotes |
| Git user config | `git config --global user.name` and `user.email` must be set — `gd` uses them for commits |

---

## 2. Initialising a repository

Run `gd init` inside any Git repository:

```sh
cd ~/projects/my-project
gd init
```

This writes a `gd.yml` config file at the repo root with sensible defaults. If one already exists, `gd init` will refuse to overwrite it unless you pass `--force`:

```sh
gd init --force   # overwrite an existing gd.yml
```

The generated file includes full comments — open it and read through before starting the daemon.

---

## 3. Starting the daemon

```sh
gd up            # foreground (Ctrl-C to stop)
gd up -d         # detached background process
```

On startup the daemon:
1. Reads and validates `gd.yml`
2. Writes its PID to `.gd/daemon.pid`
3. Opens the IPC socket at `.gd/daemon.sock`
4. Starts the filesystem watcher
5. Enters the main `tokio::select!` loop

> **Note:** `gd up` looks for `gd.yml` in the current directory by default.  
> Use `--repo /path/to/repo` to point it at a different directory:
> ```sh
> gd up --repo ~/projects/other-project
> ```

---

## 4. How auto-staging works

When a file in the working tree changes, the filesystem watcher fires within ~150 ms (after debouncing). `gd` then runs the equivalent of `git add` on all modified, new, and deleted files — **but skips**:

- Anything matching patterns in `gd.yml`'s `ignore` list  
- Anything already excluded by `.gitignore`  
- Everything inside `.gd/` (daemon state) and `.git/` (git internals)

```yaml
# gd.yml
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

After staging, `gd` waits for a **commit trigger** — either a timer or a file-count threshold, depending on your configured strategy.

### Strategy: `time` (default)

```yaml
commit:
  strategy: time
  interval: 120    # commit every 2 minutes if there are staged changes
```

Every `interval` seconds, if there is anything staged, `gd` creates a commit. If nothing is staged, the tick is a no-op.

### Strategy: `change_count`

```yaml
commit:
  strategy: change_count
  change_threshold: 10   # commit once 10 or more files have accumulated
```

`gd` counts files as they are staged. Once the count reaches `change_threshold`, it commits everything — regardless of how much time has elapsed.

> **Which should I use?**  
> Use `time` (the default) when you want regular checkpoints regardless of velocity.  
> Use `change_count` when you're on a slow machine or doing bulk edits and don't want commits on every small save.

---

## 6. Commit message generation

`gd` generates **conventional commit** messages by reading the actual diff — not just file names.

### What it reads

For every file in the staged diff, `gd` scans `+` and `-` lines for public symbol declarations in any of these languages: **Rust, TypeScript, JavaScript, Python, Go**.

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

`gd` tries credentials in this order:
1. SSH agent (via `ssh-agent`)
2. `~/.ssh/id_ed25519`
3. `~/.ssh/id_rsa`
4. `~/.ssh/id_ecdsa`
5. Git credential helper (for HTTPS remotes)

### Failure backoff

If a push fails 3 times in a row, `gd` **automatically pauses** auto-push and logs an error. Use `gd resume` to re-enable it after fixing the underlying issue (network, auth, conflict).

---

## 8. Background fetch

`gd` fetches from all remotes on a configurable interval:

```yaml
repo:
  auto_fetch: true
  fetch_interval: 60    # seconds between fetches (default 60)
```

This keeps your local remote-tracking branches (`origin/main`, etc.) fresh without you running `git fetch`. It does **not** merge or rebase — it only updates the refs. After each fetch, `gd` checks for merge conflicts and pauses push if one is detected.

To disable background fetch:

```yaml
repo:
  auto_fetch: false
```

---

## 9. Secret scanning

With `safety.block_secrets: true` (the default), `gd` scans the diff before every push. If any of the following patterns are found on `+` lines, the push is **blocked**:

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
- An error is logged to `gd status`'s recent errors list
- The commit **remains on your local branch** — it is not rolled back

To fix: scrub the secret from your working tree, amend or revert the commit, and let `gd` push cleanly.

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

All control commands communicate with the running daemon over the Unix socket at `.gd/daemon.sock`.

### Pause auto-push

```sh
gd pause
```

Stops automatic pushing. Auto-staging and auto-committing continue normally. Use this when you need to do local cleanup before the next push goes out.

### Resume auto-push

```sh
gd resume
```

Re-enables auto-push after a pause.

### Force immediate push

```sh
gd push
```

Triggers a push right now, bypassing the timer. The daemon runs the same push logic (including secret scanning) as a scheduled push.

---

## 12. Reading the status display

```sh
gd status
```

```
⚡ gitdaemon — /home/alice/projects/my-project
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
gd log           # last 10 gd auto-commits
gd log -n 25     # last 25
gd log --all     # all commits, not just gd ones
gd log -f        # follow mode — stream new commits live (Ctrl-C to stop)
```

Shows commits created by `gd` (identified by conventional commit format or `auto:` prefix). Output:

```
gd a3f2b1c0  2025-04-01 14:32:11 UTC  feat(git): introduce PushQueue and implement try_push
gd d9e17aa2  2025-04-01 14:28:05 UTC  fix(config): rework validate
```

The `gd` tag in the first column marks auto-commits. Non-gd commits show blank when `--all` is used.

See [section 24](#24-real-time-commit-log-gd-log--f) for follow mode details.

---

## 14. Stopping the daemon

```sh
gd down
```

Sends `SIGTERM` to the daemon process. The daemon:
1. Exits the select loop cleanly
2. Removes `.gd/daemon.pid`
3. Removes `.gd/daemon.sock`

If the daemon is not running, `gd down` reports an error and exits with code 2.

---

## 15. Conflict handling

When a merge conflict is detected (the repository enters `Merge` state), `gd` automatically pauses auto-push. Auto-staging and auto-committing continue, so your work is still checkpointed locally.

To resume after resolving:
1. Resolve the conflict normally (`git mergetool`, manual edit, etc.)
2. Stage the resolved files (`git add` or let `gd` stage them)
3. `gd resume` — this clears the conflict flag and re-enables push

---

## 16. Background mode (-d)

```sh
gd up -d
```

Launches the daemon as a detached background process using `setsid`. The parent process prints the child PID and exits immediately:

```
daemon started (pid 18432)
```

The background daemon reads the same `gd.yml` and writes to the same `.gd/` directory. Use `gd status`, `gd pause`, etc. to interact with it. `gd down` stops it.

To check if a background daemon is running:

```sh
gd status     # shows "daemon   running (pid 18432)"
# or
cat .gd/daemon.pid && kill -0 $(cat .gd/daemon.pid)
```

---

## 17. Logging and debugging

`gd` uses structured logging via the `tracing` crate. Control verbosity with:

```sh
# Foreground daemon with info-level logs
gd up -v

# Debug logs (shows staging, commit, push details)
gd up -vv

# Trace logs (shows every file event and IPC message)
gd up -vvv

# Override via environment variable (takes precedence over -v flags)
RUST_LOG=debug gd up
RUST_LOG=gitdaemon=trace,git2=warn gd up
```

Log format:
```
2025-04-01T14:32:11Z INFO  committed oid=a3f2b1c0 files=3 message="feat(git): introduce PushQueue"
2025-04-01T14:33:05Z INFO  pushed branch=main commits=1
2025-04-01T14:34:00Z INFO  fetched remote=origin refs_updated=2
```

The `.gd/` directory does **not** currently write a log file — all output goes to stdout/stderr. To capture logs from a background daemon, redirect when launching:

```sh
gd up -vv > .gd/daemon.log 2>&1 &
```

Or use `gd up -d` and check stderr via your process supervisor.

---

## 18. Auto-sync base branch

When you're working on a feature branch, `gd` can automatically keep `main` (or `master`) up to date and rebase your branch onto it — so you're always developing on a fresh base without ever running `git fetch`, `git checkout main`, `git pull`, or `git rebase main` manually.

### What it does

Every time the fetch ticker fires, `gd` runs three steps:

1. **Fetch** from all remotes (standard background fetch)
2. **Fast-forward `main`** — updates the local `main` ref to match `origin/main` without checking it out. This is a pure ref update, so your working tree is completely untouched.
3. **Rebase your branch** — if your working tree is clean (no unsaved edits), runs `git rebase main` to put your commits on top of the new base.

### When `gd` skips the rebase

| Situation | What happens |
|---|---|
| Working tree has unsaved edits | **Auto-stash** → rebase → **pop stash** (see §19) |
| Already on `main` / `base_branch` | Only fast-forward runs, no rebase needed |
| Repo in mid-rebase / mid-merge state | Skip until state is clean |
| `main` didn't advance | No-op |

If a rebase conflict occurs, `gd` immediately runs `git rebase --abort`, restores your stash, pauses push, fires `on_conflict` hook, and logs the error. You resolve manually, then `gd resume`.

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

# Start gd
gd up -d

# Work normally — edit files, gd auto-commits
# Meanwhile, behind the scenes every 60s:
#   - gd fetches origin
#   - gd fast-forwards main  (+3 commits from teammates)
#   - gd rebases feature/my-thing onto the new main
#   (only when your tree is clean between saves)

# When you're done, push and open a PR
gd push
```

Your branch is always close to `main`, merge conflicts are caught early (one commit at a time rather than a massive divergence), and you never had to context-switch to run Git plumbing commands.

---

## 19. Auto-stash before rebase

Previously, if your working tree had unsaved edits when `gd` attempted a rebase, it would skip the cycle entirely. Now `gd` **auto-stashes**:

1. Runs `git stash push --include-untracked`
2. Runs `git rebase <base_branch>`
3. If rebase succeeds → runs `git stash pop` to restore your edits
4. If rebase conflicts → runs `git rebase --abort` + `git stash pop`, pauses push, logs error

This means the rebase now happens every fetch cycle regardless of whether you have unsaved edits. Your in-progress work is always returned to you — either cleanly after the rebase, or unchanged after an aborted rebase.

The `skipped_dirty` state is still used as a last-resort fallback if the stash itself fails (e.g. git can't stash for some reason), logged in `gd status`.

---

## 20. Branch-push guard

Auto-push is silently blocked when the current branch is in the `push.protected_branches` list:

```yaml
push:
  protected_branches:
    - main
    - master
    - develop
```

**Default protected branches:** `main`, `master`, `develop`

This prevents accidentally pushing auto-commits straight to the integration branch. When you're on a protected branch, staging and committing still happen normally — only auto-push is blocked.

To push manually when you intend to:

```sh
gd push    # triggers an immediate push regardless of protection
```

To disable the guard entirely:

```yaml
push:
  protected_branches: []
```

---

## 21. Listing tracked files (`gd ls`)

```sh
gd ls
```

Shows the current working-tree / index state in a git-status style layout, grouped and coloured by category:

```
tracking 4 files

  staged:
    S src/git/push.rs
    S src/git/fetch.rs

  modified:
    M src/config.rs

  untracked:
    ? notes.txt
```

**State legend:**

| Symbol | Meaning |
|---|---|
| `S` (green) | Staged — in the index, not yet committed |
| `M` (yellow) | Modified in working tree but not staged |
| `?` (dim) | Untracked new file |
| `D` (red) | Deleted |
| `R` (cyan) | Renamed — shows `old → new` |
| `!` (red bold) | Merge conflict |

Files in `.gd/` are always excluded. Groups appear in priority order: conflicts first, then staged, renamed, deleted, modified, untracked.

`gd ls` reads directly from the repository — it does not require the daemon to be running.

---

## 22. Undoing auto-commits (`gd undo`)

Soft-reset the last N auto-commits back into the staging area. No work is lost — all changes land back in the index, ready to be recommitted.

```sh
gd undo          # undo the last auto-commit
gd undo 3        # undo the last 3 auto-commits
gd undo --force  # undo even if the commit doesn't look like an gd auto-commit
```

**Guard:** `gd undo` refuses to undo commits that don't look like gd auto-commits (i.e. not conventional commit format and not `auto:` prefix), unless `--force` is passed. This prevents accidentally blowing away a hand-crafted commit message.

**What "soft reset" means:**
- `HEAD` moves back N commits
- All changes from those commits are placed back in the index (staged)
- Your working tree files are unchanged

After undoing, `gd` will re-commit the staged changes on the next commit tick. If you want to keep them un-committed, `gd pause` first.

> The daemon must be running for `gd undo` to work — it sends the command over the IPC socket so the daemon can update its internal state.

---

## 23. Squashing auto-commits (`gd squash`)

Collapse the last N auto-commits into one single clean commit, with a freshly-generated conventional commit message from the combined diff:

```sh
gd squash 5    # squash last 5 commits into one
```

**How it works:**
1. Finds the base commit (parent of the oldest commit to squash)
2. Diffs `base..HEAD` to get the combined changeset
3. Runs the symbol-aware message generator on the full diff
4. Soft-resets to base, reads the HEAD tree back into the index, creates one new commit

**Example:**

Before:
```
abc12345  feat(git): introduce PushQueue
def67890  fix(git): rework try_push
ff1a2b3c  chore: drop stale tests
bba99001  feat(git): implement record_commits
cc44dd55  build: add serde_json
```

After `gd squash 5`:
```
11223344  feat(git): introduce PushQueue and implement try_push, record_commits
```

Squash is useful before opening a pull request — the auto-commit history collapses into one meaningful commit with a proper message.

> Requires at least 2 commits. The daemon must be running.

---

## 24. Real-time commit log (`gd log -f`)

```sh
gd log -f           # follow gd auto-commits as they're created
gd log -f --all     # follow all commits
gd log -f -n 5      # show last 5 first, then follow
```

Follow mode polls the repository HEAD every 2 seconds. When new commits appear, they are printed immediately:

```
gd log --follow watching for new commits on /home/alice/projects/my-project (Ctrl-C to stop)
gd a3f2b1c0  14:32:11  feat(git): introduce PushQueue and implement try_push
gd d9e17aa2  14:34:05  fix(config): rework validate
--- live ---
gd 8f3a1b2c  14:36:00  chore(tests): add coverage for undo and squash
```

The `--- live ---` separator marks where the historical log ends and the live feed begins.

Press `Ctrl-C` to exit.

> `gd log -f` reads directly from git — it does not require the daemon to be running, and it works even if the daemon is paused.

---

## 25. Notification hooks

Fire shell commands when push succeeds or a conflict is detected. Use these to integrate with system notifications, webhooks, Slack, or any other tool.

```yaml
hooks:
  on_push_success: ""    # fires after a successful push
  on_conflict: ""        # fires when rebase/merge conflict detected
```

### Environment variables

**`on_push_success`** receives:

| Variable | Value |
|---|---|
| `$FG_BRANCH` | The branch that was pushed |
| `$FG_COMMITS` | Number of commits pushed |

**`on_conflict`** receives:

| Variable | Value |
|---|---|
| `$FG_BRANCH` | The branch being pushed |
| `$FG_ERROR` | Error message from the failed rebase or push |

### Examples

```yaml
hooks:
  # macOS system notification on push
  on_push_success: >
    osascript -e
    'display notification "pushed $FG_COMMITS commits on $FG_BRANCH"
    with title "gitdaemon" sound name "Glass"'

  # Linux desktop notification on push
  on_push_success: "notify-send "gitdaemon" "pushed $FG_COMMITS commits on $FG_BRANCH'"

  # macOS alert on conflict
  on_conflict: >
    osascript -e
    'display alert "gitdaemon conflict on $FG_BRANCH"
    message "$FG_ERROR" as critical'

  # Slack webhook on push
  on_push_success: >
    curl -s -X POST $SLACK_WEBHOOK
    -H 'Content-type: application/json'
    -d '{"text":"gd pushed $FG_COMMITS commits on $FG_BRANCH"}'

  # Write to a local file
  on_push_success: "echo \"$(date): pushed $FG_COMMITS on $FG_BRANCH\" >> ~/.gd-push.log"
```

Both hooks are non-fatal: a non-zero exit code is logged as a debug message but does not affect the daemon's operation.
