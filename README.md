# gitdaemon (gd)

> **Declarative background Git sync engine — auto-stage, commit, push, and rebase while you work**

[![CI](https://github.com/mugiwaraluffy56/gitdaemon/actions/workflows/ci.yml/badge.svg)](https://github.com/mugiwaraluffy56/gitdaemon/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/gitdaemon.svg)](https://crates.io/crates/gitdaemon)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rustc-1.75%2B-orange.svg)](https://www.rust-lang.org)

`gd` is a daemon that runs next to your editor and handles the entire Git loop —
watching for file changes, staging, generating meaningful commit messages,
pushing, and keeping your branch rebased on the latest base — so you never
have to stop and run `git add / commit / push / rebase` manually.

---

## Features

| Feature | Details |
|---|---|
| **Heuristic commit messages** | Generates conventional commits (`feat`, `fix`, `refactor`, `chore`, `docs`, `test`, `build`) by parsing symbol declarations from the actual diff — no API key needed |
| **AI commit messages** | Optional Claude integration produces higher-quality messages; falls back to the heuristic automatically |
| **Auto-stage** | Watches the working tree (150 ms debounce), respects `.gitignore` and your custom ignore list |
| **Batched push** | Queues commits and pushes in intervals — no hammering the remote |
| **Background fetch** | Keeps local refs fresh without running `git fetch` |
| **Auto-sync base branch** | Fast-forwards `main` and rebases your feature branch onto it with auto-stash |
| **Secret scanning** | Scans diffs for API keys, tokens, and private keys before every push |
| **Branch-push guard** | Blocks auto-push to protected branches (`main`, `master`, `develop`) |
| **Commit strategies** | `time` (every N seconds) or `change_count` (every N files) |
| **Hooks** | `pre_commit`, `post_commit`, `on_push_success`, `on_conflict` shell hooks |
| **IPC control** | `gd pause`, `gd push`, `gd status`, `gd undo`, `gd squash`, `gd ls` via Unix socket |
| **Conflict guard** | Detects merge conflicts, pauses push, fires `on_conflict` hook |

---

## Installation

### Pre-built binary

```sh
# macOS (Apple Silicon)
curl -L https://github.com/mugiwaraluffy56/gitdaemon/releases/latest/download/gd-aarch64-apple-darwin.tar.gz \
  | tar xz && sudo mv gd /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/mugiwaraluffy56/gitdaemon/releases/latest/download/gd-x86_64-apple-darwin.tar.gz \
  | tar xz && sudo mv gd /usr/local/bin/

# Linux (x86_64)
curl -L https://github.com/mugiwaraluffy56/gitdaemon/releases/latest/download/gd-x86_64-unknown-linux-gnu.tar.gz \
  | tar xz && sudo mv gd /usr/local/bin/
```

### From source

```sh
git clone https://github.com/mugiwaraluffy56/gitdaemon
cd gitdaemon
cargo build --release
cp target/release/gd ~/.local/bin/
```

See [docs/installation.md](docs/installation.md) for shell completions, updates, and uninstall.

---

## Quick start

```sh
cd ~/projects/my-project   # any git repo

gd init                    # create gd.yml
gd up -d                   # start daemon in the background

# Work normally — gd auto-stages, commits, and pushes

gd status                  # check what's happening
gd ls                      # see which files are being tracked
gd log                     # view recent auto-commits
gd down                    # stop the daemon
```

---

## Commit messages

`gd` generates **conventional commit** messages directly from the diff, not
just from file names. Two modes are available and can be switched in `gd.yml`:

### Heuristic (default — no API key needed)

The built-in generator parses symbol declarations (`struct`, `enum`, `fn`,
`class`, `def`, etc.) from diff lines using language-aware regexes:

```
feat(git): introduce PushQueue and implement try_push
fix(config): rework validate to reject zero-value intervals
refactor(daemon): extract IPC server into dedicated module
chore: drop stale lock files
test(secrets): add coverage for AWS key and GitHub token patterns
docs: update README with AI commit messages section
build: add reqwest and dotenvy dependencies
```

Supported languages: **Rust, TypeScript, JavaScript, Python, Go**.

### AI-generated (optional — any LLM)

When `ai.enabled: true`, `gd` sends the staged diff to any LLM and uses the
response as the commit message. Supports **Anthropic Claude, OpenAI, Groq,
Together AI, Ollama, LM Studio**, and any OpenAI-compatible endpoint.
Falls back to the heuristic automatically on any error. **Commits are never
blocked by the AI path.**

```yaml
# Anthropic Claude
ai:
  enabled: true
  provider: anthropic
  api_key: ""                           # reads ANTHROPIC_API_KEY from env/.env
  model: "claude-haiku-4-5-20251001"

# OpenAI
ai:
  enabled: true
  provider: openai
  api_key: ""                           # reads OPENAI_API_KEY from env/.env
  model: "gpt-4o-mini"

# Ollama (local, no key needed)
ai:
  enabled: true
  provider: openai
  api_key: "local"
  model: "llama3"
  base_url: "http://localhost:11434"
```

See [docs/ai-commit-messages.md](docs/ai-commit-messages.md) for all providers,
model examples, privacy considerations, and troubleshooting.

### Customising the template

```yaml
commit:
  message: "{summary}"   # default
  # message: "[{branch}] {summary} ({count} files, {timestamp})"
```

Tokens: `{summary}` · `{count}` · `{branch}` · `{timestamp}`

---

## Configuration (`gd.yml`)

```yaml
version: 1

repo:
  auto_stage: true        # stage changes automatically
  auto_fetch: true        # fetch from remotes in the background
  fetch_interval: 60      # seconds between fetches
  auto_sync_base: true    # fast-forward main and rebase your branch
  base_branch: main
  rebase_on_sync: true

commit:
  strategy: time          # "time" or "change_count"
  interval: 120           # seconds between auto-commits
  change_threshold: 10    # files before committing (change_count strategy)
  message: "{summary}"

push:
  strategy: batch
  interval: 300
  branch: main
  protected_branches: [main, master, develop]

ignore:
  - "*.log"
  - "node_modules/"
  - ".env"

safety:
  confirm_first: false
  block_secrets: true     # scan diffs for secrets before every push

hooks:
  pre_commit: ""          # e.g. "cargo fmt --check"
  post_commit: ""         # e.g. "cargo test --quiet"
  on_push_success: ""     # env: $FG_BRANCH, $FG_COMMITS
  on_conflict: ""         # env: $FG_BRANCH, $FG_ERROR

ai:
  enabled: false          # set true to enable AI commit messages
  provider: anthropic     # "anthropic" or "openai" (covers Groq, Ollama, etc.)
  api_key: ""             # blank = auto-read ANTHROPIC_API_KEY / OPENAI_API_KEY
  model: ""               # required — e.g. claude-haiku-4-5-20251001, gpt-4o-mini
  base_url: ""            # leave blank for default; set for local/custom endpoints
  max_diff_chars: 12000
```

See [examples/gd.full.yml](examples/gd.full.yml) for a fully-annotated config and
[docs/config-reference.md](docs/config-reference.md) for the complete field reference.

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
gd status        # live sync state: branch, ahead/behind, staged, health
gd ls            # list tracked files by state (staged/modified/untracked/…)
gd log           # last 10 gd auto-commits
gd log -n 25     # last 25
gd log -f        # follow mode — stream new commits in real time
gd log --all     # all commits, not just gd ones
```

### History editing
```sh
gd undo          # soft-reset last auto-commit back to index
gd undo 3        # soft-reset last 3 auto-commits
gd undo --force  # undo even non-gd commits
gd squash 5      # squash last 5 auto-commits into one
```

### Control
```sh
gd pause         # pause auto-push (staging + committing continue)
gd resume        # resume auto-push
gd push          # trigger immediate push
gd init          # create gd.yml in current repo
gd init --force  # overwrite existing gd.yml
```

---

## `gd status` output

```
⚡ gitdaemon — /home/alice/projects/my-project
  branch   feature/auth → origin/feature/auth
  ahead    3 commits (queued to push, last push 42s ago)
  behind   0
  staged   2 files
  watching 1,204 files
  daemon   running (pid 12345)
```

Health icons: `⚡` healthy · `⬇` behind remote · `⏸` push paused · `✗` error

---

## Security

- **Secret scanning** before every push — AWS keys, GitHub tokens, Stripe keys,
  Google API keys, Slack tokens, private key headers, and generic credential
  patterns. See [docs/secret-scanning.md](docs/secret-scanning.md).
- **Protected branch guard** — auto-push blocked on `main`, `master`, `develop`.
- **AI privacy** — when enabled, the staged diff is sent to Anthropic's API.
  See [docs/ai-commit-messages.md#privacy-considerations](docs/ai-commit-messages.md#privacy-considerations).

---

## Documentation

| Document | Description |
|---|---|
| [docs/installation.md](docs/installation.md) | Install, update, uninstall, shell completions |
| [docs/user-guide.md](docs/user-guide.md) | Full feature walkthrough |
| [docs/config-reference.md](docs/config-reference.md) | Every `gd.yml` field documented |
| [docs/ai-commit-messages.md](docs/ai-commit-messages.md) | Claude integration setup and guide |
| [docs/hooks.md](docs/hooks.md) | Shell hook reference with real-world examples |
| [docs/secret-scanning.md](docs/secret-scanning.md) | Scanner patterns and false-positive handling |
| [docs/architecture.md](docs/architecture.md) | Internal design and data flow diagrams |
| [docs/troubleshooting.md](docs/troubleshooting.md) | Common problems and fixes |
| [CHANGELOG.md](CHANGELOG.md) | Version history |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to contribute |

---

## Development

```sh
cargo build              # debug build
cargo build --release    # optimised build
cargo check              # fast type + borrow check
cargo clippy             # lint
cargo fmt                # format
cargo test               # all tests
cargo bench              # benchmarks → target/criterion/
RUST_LOG=debug gd up     # daemon with debug logging
```

Use `just` for common workflows:

```sh
just ci       # fmt-check + clippy + test
just bench    # run benchmarks
just docs     # cargo doc --open
just install  # cargo install --path .
```

---

## Non-goals

- Not a replacement for intentional, context-rich commits during code review
- Does not resolve merge conflicts automatically
- Not designed for monorepos where CI gates every push
- Does not support Windows (Unix sockets required for IPC)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports and feature requests go
through [GitHub Issues](https://github.com/mugiwaraluffy56/gitdaemon/issues).

---

## License

[MIT](LICENSE) — Copyright © 2022 gitdaemon contributors
