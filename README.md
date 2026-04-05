# fastgit (fg)

> **Declarative background Git sync engine with intelligent auto-commits**

`gd` is a daemon that runs next to your editor and handles the entire Git loop — watching for changes, staging files, generating meaningful commit messages, pushing, and keeping your branch rebased on the latest base — so you never have to stop and run `git add / commit / push / rebase` manually.

---

## Features

| Feature | Details |
|---|---|
| **Intelligent commit messages** | Generates conventional commits (`feat`, `fix`, `refactor`, `chore`, `docs`, `test`, `build`) by parsing the actual diff for symbol names |
| **Auto-stage** | Watches working tree (150 ms debounce), respects `.gitignore` and your custom ignore list |
| **Batched push** | Queues commits and pushes in intervals — no hammering the remote |
| **Background fetch** | Keeps local refs fresh without you running `git fetch` |
| **Auto-sync base branch** | Fast-forwards `main` in the background and rebases your feature branch onto it with auto-stash |
| **Secret scanning** | Scans diffs for API keys, tokens, and private keys before every push |
| **Branch-push guard** | Blocks auto-push to protected branches (`main`, `master`, `develop`) |
| **Commit strategies** | `time` (every N seconds) or `change_count` (every N files) |
| **Hooks** | `pre_commit`, `post_commit`, `on_push_success`, `on_conflict` shell hooks |
| **IPC control** | `gd pause`, `gd push`, `gd status`, `gd undo`, `gd squash`, `gd ls` via Unix socket |
| **Conflict guard** | Detects merge conflicts, pauses push, fires `on_conflict` hook |

---

## Installation

### From source

```sh
git clone https://github.com/fastgit/fg
cd fg
cargo build --release
cp target/release/gd ~/.local/bin/
```

### Requirements

- Rust 1.75+
- `libgit2` (bundled by the `git2` crate — no separate install needed)
- SSH key at `~/.ssh/id_ed25519`, `id_rsa`, or `id_ecdsa` (for SSH remotes)
- `git config --global user.name` and `user.email` must be set

---

## Quick start

```sh
# 1. Go to a git repo
cd ~/projects/my-project

# 2. Create the config file
gd init

# 3. Start the daemon in the background
gd up -d

# 4. Work normally — gd auto-stages, commits, and pushes
#    main is kept up to date; your branch gets rebased automatically

# 5. Check what's happening
gd status

# 6. See files being tracked
gd ls

# 7. Stop when you're done
gd down
```

---

## Commit messages

`gd` generates **conventional commit** messages automatically, directly from the diff — not just file names:

```
feat(git): introduce PushQueue and implement try_push
fix(config): rework validate
refactor(daemon): introduce DaemonContext and implement run
feat(ipc): introduce IpcCommand and IpcResponse, implement send_command
chore: drop stale lock files
docs: update README
build: add serde_json, tighten nix feature flags
```

### How it works

1. **Diff parsing** — `gd` walks every `+`/`-` line in the staged diff extracting declared symbols (structs, enums, traits, functions) using language-specific regexes for Rust, TypeScript, JavaScript, Python, and Go
2. **Type** is inferred from what changed (new files → `feat`, modifications → `fix`, mix of adds+deletes → `refactor`, etc.)
3. **Scope** is the first shared subdirectory under `src/` when ≥ 60% of files share one
4. **Subject** uses action verbs on the extracted symbols: `introduce` (new types), `implement` (new functions), `rework` (modified symbols), `drop` (deleted symbols)

### Customising the message

```yaml
commit:
  message: "{summary}"  # default — uses generated conventional commit verbatim
```

Available tokens: `{summary}`, `{count}`, `{branch}`, `{timestamp}`

---

## Configuration (`fg.yml`)

```yaml
version: 1

repo:
  auto_stage: true        # stage working-tree changes automatically
  auto_fetch: true        # fetch from remotes in the background
  fetch_interval: 60      # seconds between fetches
  auto_sync_base: true    # fast-forward main and rebase your branch onto it
  base_branch: main       # the branch to keep updated
  rebase_on_sync: true    # rebase current branch after base advances

commit:
  strategy: time          # "time" or "change_count"
  interval: 120           # seconds between auto-commits (time strategy)
  change_threshold: 10    # files before committing (change_count strategy)
  message: "{summary}"    # commit message template

push:
  strategy: batch         # push queued commits together
  interval: 300           # seconds between auto-pushes
  branch: main            # branch to push
  protected_branches:     # never auto-push to these
    - main
    - master
    - develop

ignore:
  - "*.log"
  - "node_modules/"
  - ".env"
  - "dist/"

safety:
  confirm_first: false    # prompt before first push in a session
  block_secrets: true     # scan diffs for secrets before pushing

hooks:
  pre_commit: ""          # e.g. "cargo fmt --check" — non-zero aborts commit
  post_commit: ""         # e.g. "cargo test" — non-zero is logged, not fatal
  on_push_success: ""     # fired after successful push ($FG_BRANCH, $FG_COMMITS)
  on_conflict: ""         # fired on rebase/merge conflict ($FG_BRANCH, $FG_ERROR)
```

---

## CLI reference

### Daemon lifecycle

```sh
gd up            # start daemon (foreground, Ctrl-C to stop)
gd up -d         # start daemon detached in the background
gd down          # send SIGTERM to the running daemon
```

### Status & history

```sh
gd status        # live sync state: branch, ahead/behind, staged files, health
gd ls            # list all tracked files by state (staged / modified / untracked)
gd log           # last 10 gd auto-commits
gd log -n 25     # last 25
gd log -f        # follow mode — stream new commits in real time as they're created
gd log --all     # show all commits, not just gd ones
```

### Undo & history editing

```sh
gd undo          # soft-reset the last auto-commit back to the index
gd undo 3        # soft-reset the last 3 auto-commits
gd undo --force  # undo even if the commit doesn't look like a gd auto-commit
gd squash 5      # squash last 5 auto-commits into one clean commit
```

### Control

```sh
gd pause         # pause auto-push (staging and committing continue)
gd resume        # resume auto-push
gd push          # trigger an immediate push right now
gd init          # create fg.yml in the current repo
gd init --force  # overwrite an existing fg.yml
```

---

## `gd status` output

```
⚡ fastgit — /home/alice/projects/my-project
  branch   feature/auth → origin/feature/auth
  ahead    3 commits (queued to push, last push 42s ago)
  behind   0
  staged   2 files
  watching 1,204 files
  daemon   running (pid 12345)
```

Health icons:

| Icon | Meaning |
|---|---|
| `⚡` | Healthy — in sync |
| `⬇` | Behind remote — fetch pending |
| `⏸` | Push paused |
| `✗` | Error condition |

---

## `gd ls` output

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

File state legend: `S` staged · `M` modified · `?` untracked · `D` deleted · `R` renamed · `!` conflict

---

## Auto-sync base branch

When you're on a feature branch, `gd` automatically:

1. **Fast-forwards `main`** to `origin/main` without checking it out — pure ref update, your working tree untouched
2. **Auto-stashes** any in-progress work if the tree is dirty
3. **Rebases your branch** onto the updated `main`
4. **Pops the stash** to restore your in-progress edits

This happens silently every `fetch_interval` seconds. You never need to run `git pull`, `git checkout main`, or `git rebase main` manually.

If a rebase conflict occurs, `gd` aborts the rebase, restores your stash, pauses push, fires the `on_conflict` hook, and logs the error in `gd status`. You resolve it manually, then `gd resume`.

To disable:
```yaml
repo:
  auto_sync_base: false
```

---

## Safety

### Protected branches

Auto-push is blocked when you're on a protected branch. This prevents accidentally pushing commits straight to `main`:

```yaml
push:
  protected_branches:
    - main
    - master
    - develop
```

Use `gd push` to push manually to a protected branch when you intend to.

### Secret scanning

When `safety.block_secrets: true` (the default), `gd` scans the diff **before every push** for:

- AWS Access Key IDs (`AKIA…`)
- AWS Secret Access Keys
- GitHub Personal Access Tokens (`ghp_…`, `ghs_…`)
- Stripe live secret keys (`sk_live_…`)
- Google API keys (`AIza…`)
- Slack tokens (`xox…`)
- Private key headers (`-----BEGIN … PRIVATE KEY-----`)
- Hardcoded passwords and generic API key assignments

If a match is found the push is **blocked** and an error is logged. The commit is kept locally — scrub the secret, amend or revert, then let `gd` push cleanly.

### Merge conflicts

When a merge conflict is detected, `gd` automatically pauses push. It resumes once the conflict is resolved.

### Hooks

```yaml
hooks:
  pre_commit: "cargo fmt --check && cargo clippy -- -D warnings"
  post_commit: "cargo test --quiet"
  on_push_success: "osascript -e 'display notification \"pushed\" with title \"fg\"'"
  on_conflict: "notify-send "gd conflict' \"$FG_ERROR\""
```

- `pre_commit` non-zero → commit aborted, staged changes remain
- `post_commit` non-zero → logged as warning, commit kept
- `on_push_success` → receives `$FG_BRANCH`, `$FG_COMMITS`
- `on_conflict` → receives `$FG_BRANCH`, `$FG_ERROR`

---

## Daemon internals

The daemon runs a single `tokio::select!` loop over six concurrent channels:

1. **Filesystem watcher** — `notify` crate, 150 ms debounce, respects `.gitignore` and `fg.yml` ignore list
2. **Commit ticker** — fires every `commit.interval` seconds
3. **Push ticker** — fires every `push.interval` seconds
4. **Force-push notifier** — woken by `gd push` via IPC
5. **Fetch + sync ticker** — fires every `repo.fetch_interval` seconds, runs fetch → fast-forward base → rebase
6. **IPC channel** — Unix socket at `.fg/daemon.sock`

State files written to `.fg/`:

| File | Purpose |
|---|---|
| `daemon.pid` | PID of the running daemon |
| `daemon.sock` | Unix socket for IPC |

---

## Non-goals

- Not a replacement for intentional, context-rich commits during feature work
- Does not resolve merge conflicts automatically
- Not designed for monorepos with CI gating every push
- Does not support Windows (Unix sockets are used for IPC)

---

## Development

```sh
cargo build              # debug build
cargo build --release    # optimised build
cargo check              # fast type + borrow check
cargo clippy             # lint
cargo fmt                # format
cargo test               # all tests (53 tests)
RUST_LOG=debug gd up     # run daemon with debug logging
```
