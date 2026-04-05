# Contributing to gitdaemon

Thank you for taking the time to contribute! This document explains how to get
started, what to work on, and how pull requests are reviewed.

---

## Table of contents

1. [Getting started](#getting-started)
2. [Development workflow](#development-workflow)
3. [Code style](#code-style)
4. [Testing](#testing)
5. [Commit messages](#commit-messages)
6. [Opening a pull request](#opening-a-pull-request)
7. [Reporting bugs](#reporting-bugs)
8. [Requesting features](#requesting-features)
9. [Release process](#release-process)

---

## Getting started

### Prerequisites

- Rust 1.75 or later (`rustup update stable`)
- `libgit2` — bundled via the `git2` crate, no separate install needed
- Optionally: [`just`](https://github.com/casey/just) task runner, `cargo-watch`

### Fork and clone

```sh
git clone https://github.com/mugiwaraluffy56/gitdaemon
cd gitdaemon
cargo build
cargo test
```

If all tests pass you are ready to go.

---

## Development workflow

```sh
# Run the full check suite quickly (no linking)
just check

# Start a file-watcher that re-runs check on every save
cargo watch -x check

# Run tests
just test

# Lint
just lint

# Full CI gate before pushing
just ci
```

### Running the daemon locally

```sh
# In a scratch repo
mkdir /tmp/test-repo && cd /tmp/test-repo
git init && git remote add origin git@github.com:you/test-repo.git

# Build and init
cargo build --manifest-path /path/to/gitdaemon/Cargo.toml
/path/to/gitdaemon/target/debug/gd init

# Run in the foreground with debug logging
RUST_LOG=debug /path/to/gitdaemon/target/debug/gd up
```

---

## Code style

- All code is formatted with `rustfmt` — run `just fmt` before committing.
- Clippy lints must pass with `-D warnings` — run `just lint`.
- No `unwrap()` / `expect()` in non-test code — use `?` and `anyhow::Context`.
- Every error path must be handled and wrapped with `.context("…")`.
- Async functions go in the `daemon/` or `git/` subtree; pure logic stays synchronous.
- Keep modules small and focused — the `git/` submodules are good examples.

### Module responsibilities

| Module | Rule |
|---|---|
| `config.rs` | Standalone — no deps beyond `serde` and standard library |
| `git/*` | Pure Git operations — no IPC or daemon lifecycle |
| `daemon/*` | Lifecycle and orchestration only — delegates to `git/` |
| `cli.rs` | `clap` structs only — no logic |

---

## Testing

### Unit tests

Each module has `#[cfg(test)] mod tests { … }` inline. Run them with:

```sh
cargo test
```

### Integration tests

Integration tests live in `tests/`. They spin up real temporary Git repos using
`tempfile::TempDir` and exercise the library interface end-to-end without
starting the background daemon.

When adding a new feature, add at least:
- One unit test for the core logic
- One integration test that exercises the feature through the public API

### Benchmarks

Benchmarks live in `benches/` and use Criterion. Run with:

```sh
cargo bench
```

Benchmark results are written to `target/criterion/` as HTML reports.

---

## Commit messages

This project uses **conventional commits** (which `gd` itself generates):

```
feat(scope): add short description
fix(scope): correct short description
refactor(scope): describe change
docs: update section heading
test: add coverage for X
build: bump dependency Y
```

- `feat` — new feature
- `fix` — bug fix
- `refactor` — restructuring without behaviour change
- `docs` — documentation only
- `test` — test only
- `build` — `Cargo.toml`, CI, scripts
- `chore` — maintenance tasks

Scope is the module name (`git`, `daemon`, `config`, `ipc`, `secrets`, etc.)

---

## Opening a pull request

1. Create a branch from `main`: `git checkout -b feat/my-feature`
2. Make your changes — keep commits atomic and conventional
3. Ensure `just ci` passes
4. Push and open a PR against `main`
5. Fill in the PR template

PRs are squash-merged. The squash commit message is the PR title, so make sure
the title follows the conventional-commit format.

### Review checklist

- [ ] `just ci` passes locally
- [ ] New behaviour is covered by tests
- [ ] Public API changes are reflected in `docs/`
- [ ] Breaking changes are noted in the PR description

---

## Reporting bugs

Use the [Bug Report](.github/ISSUE_TEMPLATE/bug_report.yml) template. Include:

- `gd --version` output
- OS and Rust version (`rustc --version`)
- Steps to reproduce
- Expected vs. actual behaviour
- `RUST_LOG=debug gd up` output (redact any secrets)

---

## Requesting features

Use the [Feature Request](.github/ISSUE_TEMPLATE/feature_request.yml) template.
Describe the problem you are trying to solve, not just the solution.

---

## Release process

Releases are cut by maintainers:

1. Update `CHANGELOG.md` — add a new `## [x.y.z]` section
2. Bump `version` in `Cargo.toml`
3. Open a PR titled `release: x.y.z`
4. After merge, tag: `git tag vx.y.z && git push origin vx.y.z`
5. GitHub Actions builds and publishes the release binaries automatically

---

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE).
