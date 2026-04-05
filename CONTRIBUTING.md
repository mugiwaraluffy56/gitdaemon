# Contributing to gitdaemon

Thank you for your interest in contributing! This document covers how to get
set up, the coding conventions we follow, and the pull request process.

---

## Table of contents

1. [Getting started](#1-getting-started)
2. [Development workflow](#2-development-workflow)
3. [Coding conventions](#3-coding-conventions)
4. [Testing](#4-testing)
5. [Submitting a pull request](#5-submitting-a-pull-request)
6. [Issue reporting](#6-issue-reporting)
7. [Release process](#7-release-process)

---

## 1. Getting started

### Prerequisites

| Tool | Version | Install |
|---|---|---|
| Rust | ≥ 1.75 | `rustup update stable` |
| just | any | `cargo install just` |
| git | ≥ 2.38 | system package manager |

### Fork and clone

```sh
git clone https://github.com/<you>/gitdaemon
cd gitdaemon
cargo build
```

### Verify everything works

```sh
just ci          # fmt-check + clippy + test
cargo test       # all 50+ unit tests
```

---

## 2. Development workflow

```sh
# Edit code
$EDITOR src/git/commit.rs

# Fast type-check (no linking)
cargo check

# Lint
cargo clippy --all-targets -- -D warnings

# Format
cargo fmt

# Run all tests
cargo test

# Run a single test
cargo test test_commit_creates_commit

# Run the daemon against a local repo
cargo build && RUST_LOG=debug ./target/debug/gd up
```

Use `just` for common tasks:

```sh
just build      # cargo build
just test       # cargo test
just lint       # fmt-check + clippy
just ci         # full gate (lint + test)
just docs       # cargo doc --no-deps --open
```

---

## 3. Coding conventions

### Style

- **`rustfmt`** is enforced in CI. Run `cargo fmt` before committing.
- **`clippy`** is enforced at `-D warnings`. Fix all lints before opening a PR.
- Keep functions short; prefer many small, well-named helpers over long
  monolithic functions.
- Avoid `unwrap()` and `expect()` in production paths. Use `?` with `anyhow`
  context everywhere.

### Error handling

We use [`anyhow`](https://docs.rs/anyhow) for application errors and
[`thiserror`](https://docs.rs/thiserror) for library error types:

```rust
// Good — wraps errors with context
let config = Config::load(&path)
    .with_context(|| format!("failed to load config from {}", path.display()))?;

// Bad — drops context
let config = Config::load(&path)?;
```

### Async

- All daemon code is `async` via `tokio`. Prefer `tokio::task::spawn_blocking`
  for git2 calls (they are synchronous and can block).
- Do not call `std::thread::sleep` in async code. Use `tokio::time::sleep`.

### Commit messages

Follow the conventional commit spec:

```
feat(scope): add something new
fix(scope): correct a bug
refactor(scope): restructure without behaviour change
chore: update dependencies
docs: fix typo in README
test(scope): add coverage for X
```

---

## 4. Testing

### Unit tests

Each module has a `#[cfg(test)] mod tests` block. Run with:

```sh
cargo test
```

### Integration tests

```sh
cargo test --test integration
```

Integration tests create temporary Git repositories using `tempfile::TempDir`
and exercise the full commit / push / IPC flow without hitting a real remote.

### Writing tests

- Use `tempfile::TempDir` for temporary repositories; they auto-cleanup on drop.
- Do **not** mock `git2`. Test against real Git objects — that's where the bugs
  live.
- Async tests use `#[tokio::test]`.

---

## 5. Submitting a pull request

1. **Open an issue first** for any non-trivial change so we can discuss the
   approach before you invest time coding.
2. Create a branch: `git checkout -b feat/my-change`.
3. Make your changes with tests.
4. Ensure `just ci` passes locally.
5. Open a PR against `main` using the PR template.
6. One approving review from a maintainer is required to merge.

### PR checklist

- [ ] `just ci` passes (fmt + clippy + tests)
- [ ] New behaviour has tests
- [ ] `CHANGELOG.md` updated under `[Unreleased]`
- [ ] Docs updated if user-facing behaviour changed

---

## 6. Issue reporting

Use the GitHub issue templates:

- **Bug report** — unexpected behaviour, panics, incorrect output
- **Feature request** — new capability or configuration option

Before opening an issue, search existing ones. Duplicate issues will be closed.

When reporting a bug, always include:

```
gd --version
rustc --version
OS and version
RUST_LOG=debug gd up 2>&1  (sanitised if needed)
```

---

## 7. Release process

Releases are managed by maintainers. The process is:

1. Update `CHANGELOG.md` — move `[Unreleased]` to the new version with today's date.
2. Bump `version` in `Cargo.toml`.
3. Open a PR titled `chore: release v0.X.Y`.
4. After merge, tag: `just tag 0.X.Y`.
5. The `release.yml` GitHub Actions workflow builds binaries and creates the
   GitHub Release automatically.

---

## Code of Conduct

By contributing you agree to follow our [Code of Conduct](CODE_OF_CONDUCT.md).
