# fastgit (fg)

> **Declarative background Git sync engine with intelligent auto-commits**

`fg` is a daemon that runs next to your editor and handles the entire Git loop — watching for changes, staging files, generating meaningful commit messages, and pushing — so you never have to stop and run `git add / commit / push` manually.

---

## Features

| Feature | Details |
|---|---|
| **Intelligent commit messages** | Generates conventional commits (`feat`, `fix`, `refactor`, `chore`, `docs`, `test`, `build`) with scope inferred from your file tree |
| **Auto-stage** | Watches working tree, respects `.gitignore` and your custom ignore list |
| **Batched push** | Queues commits and pushes in intervals — no hammering the remote |
| **Background fetch** | Keeps your local refs fresh without you running `git fetch` |
| **Secret scanning** | Scans diffs for API keys, tokens, and private keys before every push |
| **Commit strategies** | `time` (every N seconds) or `change_count` (every N files) |
| **Hooks** | `pre_commit` / `post_commit` shell commands; non-zero exit aborts the commit |
| **IPC control** | `fg pause`, `fg push`, `fg status` talk to the daemon over a Unix socket |
| **Conflict guard** | Detects merge conflicts and pauses push automatically |

---

## Installation

### From source

```sh
git clone https://github.com/fastgit/fg
cd fg
cargo build --release
# Copy to your PATH
cp target/release/fg ~/.local/bin/
```

### Requirements

- Rust 1.75+
- `libgit2` (usually bundled by the `git2` crate — no separate install needed)
- SSH key at `~/.ssh/id_ed25519`, `id_rsa`, or `id_ecdsa` (for SSH remotes)

---

## Quick start

```sh
# 1. Go to a git repo
cd ~/projects/my-project

# 2. Create the config file
fg init

# 3. Start the daemon in the background
fg up -d

# 4. Work normally — fg auto-stages, commits, and pushes

# 5. Check what's happening
fg status

# 6. Stop when you're done
fg down
```

---

## Commit messages

`fg` generates **conventional commit** messages automatically based on what changed:

```
feat(git): add push queue and credential callback
fix(config): update branch validation and interval defaults
refactor(daemon): reorganise IPC server, context, and 2 others
chore: remove stale lock files
test(secrets): add coverage for AWS key and GitHub token patterns
docs: update README with installation guide
build: update Cargo.toml dependencies
```

### How the type is inferred

| Type | When |
|---|---|
| `feat` | Majority of changes are **new source files** |
| `fix` | Majority of changes are **modifications** to source files |
| `refactor` | Mix of adds + deletes, or all renames |
| `chore` | All deletions, or build/config-only files |
| `build` | Only build files (`Cargo.toml`, `package.json`, `Makefile`, …) |
| `docs` | Only documentation files (`.md`, `.rst`, `README`, …) |
| `test` | Only test files (`tests/`, `*_test.rs`, `test_*`, …) |

### How the scope is inferred

The scope is the first meaningful subdirectory under `src/`. For example:

| Changed files | Scope |
|---|---|
| `src/git/push.rs`, `src/git/fetch.rs` | `git` |
| `src/daemon/ipc.rs` | `daemon` |
| `src/config.rs` | `config` |
| `README.md` | *(none — docs has no scope)* |

### Customising the message

Edit `fg.yml`:

```yaml
commit:
  message: "{summary}"  # default — uses the generated conventional commit verbatim
```

Available tokens:

| Token | Value |
|---|---|
| `{summary}` | Full generated conventional commit line |
| `{count}` | Number of files changed |
| `{branch}` | Current branch name |
| `{timestamp}` | ISO-8601 UTC timestamp |

Example custom templates:

```yaml
# Prepend a ticket reference
message: "[PROJ-123] {summary}"

# Include branch and timestamp
message: "{summary} [{branch} @ {timestamp}]"
```

---

## Configuration (`fg.yml`)

```yaml
version: 1

repo:
  auto_stage: true        # stage working-tree changes automatically
  auto_fetch: true        # fetch from remotes in the background
  fetch_interval: 60      # seconds between fetches

commit:
  strategy: time          # "time" or "change_count"
  interval: 120           # seconds between auto-commits (time strategy)
  change_threshold: 10    # files before committing (change_count strategy)
  message: "{summary}"    # commit message template

push:
  strategy: batch         # push queued commits together
  interval: 300           # seconds between auto-pushes
  branch: main            # branch to push

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
  post_commit: ""         # e.g. "cargo test" — non-zero is logged but not fatal
```

### Commit strategies

| Strategy | Behaviour |
|---|---|
| `time` | Commits every `interval` seconds if there are staged changes |
| `change_count` | Commits once `change_threshold` files have accumulated |

---

## CLI reference

### Daemon lifecycle

```sh
fg up            # start daemon (foreground, Ctrl-C to stop)
fg up -d         # start daemon detached in the background
fg down          # send SIGTERM to the running daemon
```

### Status & history

```sh
fg status        # live sync state via IPC
fg log           # recent auto-commits (last 10)
fg log -n 25     # last 25 auto-commits
```

### Control

```sh
fg pause         # pause auto-push (staging and committing continue)
fg resume        # resume auto-push
fg push          # trigger an immediate push right now
fg init          # create fg.yml in the current repo
fg init --force  # overwrite an existing fg.yml
```

---

## `fg status` output

```
⚡ fastgit — /home/alice/projects/my-project
  branch   main → origin/main
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

## Safety

### Secret scanning

When `safety.block_secrets: true` (the default), `fg` scans the diff **before every push** for:

- AWS Access Key IDs (`AKIA…`)
- AWS Secret Access Keys
- GitHub Personal Access Tokens (`ghp_…`, `ghs_…`)
- Stripe live secret keys (`sk_live_…`)
- Google API keys (`AIza…`)
- Slack tokens (`xox…`)
- Private key headers (`-----BEGIN … PRIVATE KEY-----`)
- Hardcoded passwords and generic API key assignments

If a match is found the push is **blocked** and an error is logged. The commit is kept locally — you need to scrub the secret, amend or revert, then let `fg` push cleanly.

### Merge conflicts

When a merge conflict is detected, `fg` automatically pauses push. It resumes once the conflict is resolved.

### Hooks

```yaml
hooks:
  pre_commit: "cargo fmt --check && cargo clippy -- -D warnings"
  post_commit: "cargo test --quiet"
```

- `pre_commit` non-zero → commit is aborted, staged changes remain
- `post_commit` non-zero → logged as a warning, commit is **not** rolled back

---

## Daemon internals

The daemon runs a single `tokio::select!` loop over six concurrent channels:

1. **Filesystem watcher** — `notify` crate, 150 ms debounce, respects `.gitignore` and `fg.yml` ignore list; `.fg/` and `.git/` are always excluded
2. **Commit ticker** — fires every `commit.interval` seconds
3. **Push ticker** — fires every `push.interval` seconds
4. **Force-push notifier** — woken by `fg push` via IPC
5. **Fetch ticker** — fires every `repo.fetch_interval` seconds
6. **IPC channel** — Unix socket at `.fg/daemon.sock`, handles `status`, `pause`, `resume`, `push now`, `shutdown`

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
cargo test               # all tests
RUST_LOG=debug fg up     # run daemon with debug logging
```
