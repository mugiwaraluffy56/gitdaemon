# Changelog

All notable changes to `gitdaemon` (`gd`) are documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added
- **AI-generated commit messages** — when `ai.enabled: true` in `gd.yml`, Claude
  generates a proper conventional commit from the staged diff instead of the
  heuristic generator. Falls back automatically if the API is unreachable.
- `ai.api_key` supports `env:VAR_NAME` syntax to read from an environment
  variable, or auto-loads `ANTHROPIC_API_KEY` from a `.env` file in the repo root.

---

## [0.8.0] — 2024-11-14

### Added
- `gd squash N` — squash the last N auto-commits into one clean commit with a
  regenerated message.
- `gd undo --force` — undo commits that don't match the auto-commit heuristic.
- `on_push_success` and `on_conflict` hooks with environment variables
  `$FG_BRANCH`, `$FG_COMMITS`, `$FG_ERROR`.
- Branch-push guard: auto-push is blocked on branches listed in
  `push.protected_branches`.

### Changed
- Commit message generator now parses Go exported function names (capitalised
  identifiers after `func`).
- `gd log --follow` polls every 2 s instead of 1 s to reduce CPU usage.

### Fixed
- Daemon failed to start when `.gd/` directory did not exist (#142).
- Race condition in PID file write on fast restarts (#138).
- `gd status` printed stale `behind` count after a successful rebase (#131).

---

## [0.7.0] — 2024-09-03

### Added
- Auto-sync base branch: after every fetch, `main` is fast-forwarded in place
  and the current feature branch is rebased onto it automatically.
- Auto-stash before rebase — dirty working trees no longer block the sync.
- `gd ls` command listing all tracked files by state (staged / modified /
  untracked / deleted / renamed / conflict).
- `gd log -f` / `--follow` streams new auto-commits in real time.
- Conventional commit type `ci` recognised in `gd log --all` filter.

### Changed
- Minimum Rust version bumped to 1.75.

### Fixed
- Filesystem watcher emitted spurious events for `.gd/daemon.sock` (#119).
- Secret scanner produced false positives on base64-encoded image data (#113).

---

## [0.6.2] — 2024-07-18

### Fixed
- `gd down` left a stale PID file when the daemon had already exited (#108).
- Push ticker fired immediately on daemon start, causing an empty push (#104).

---

## [0.6.1] — 2024-07-01

### Fixed
- Compilation failure on Linux with `nix` 0.28 signal feature flag (#100).

---

## [0.6.0] — 2024-06-12

### Added
- Secret scanning before push: AWS keys, GitHub tokens, Stripe keys, Google API
  keys, Slack tokens, private key headers, and generic `password=` / `api_key=`
  patterns.
- `safety.block_secrets` config field (default: `true`).
- `gd undo [N]` — soft-reset the last N auto-commits back to the index.

### Changed
- IPC protocol switched from line-delimited text to newline-delimited JSON.
- `gd push` now waits for the daemon's acknowledgement before exiting.

### Removed
- Legacy `auto:` commit message prefix dropped in favour of conventional commits.

---

## [0.5.0] — 2024-04-22

### Added
- Conventional commit message generation: diff is parsed for symbol declarations
  (structs, enums, traits, functions) to produce messages like
  `feat(git): introduce PushQueue and implement try_push`.
- Multi-language symbol extraction: Rust, TypeScript/JavaScript, Python, Go.
- `group_by_directory` commit config: files from different top-level directories
  are committed separately.
- `commit.change_threshold` strategy for high-frequency editing workflows.

### Fixed
- `gd status` crashed when HEAD was an unborn branch (#82).

---

## [0.4.0] — 2024-02-08

### Added
- Background fetch ticker (`repo.fetch_interval`, default 60 s).
- `gd resume` command to re-enable auto-push after `gd pause`.
- Merge conflict detection: daemon pauses push and fires `on_conflict` hook.
- `post_commit` hook.

### Changed
- Socket path moved from `/tmp/gd-<uid>.sock` to `.gd/daemon.sock` inside the
  repo, allowing per-repo daemon instances without conflicts.

---

## [0.3.0] — 2023-11-27

### Added
- `gd pause` / `gd push` IPC commands over Unix socket.
- `gd status` rendering with health icons.
- `pre_commit` hook support.
- Protected branch guard for `main`, `master`, and `develop`.

### Changed
- Daemon now writes `daemon.pid` to `.gd/` and cleans it up on shutdown.

---

## [0.2.0] — 2023-09-14

### Added
- Batched push queue — commits accumulate between push ticks.
- `push.interval` and `push.branch` configuration.
- `gd init` creates a commented `gd.yml` template.

### Fixed
- Daemon exited immediately when repository had no remote (#41).

---

## [0.1.0] — 2023-07-01

### Added
- Initial release.
- Filesystem watcher with `.gitignore` and `gd.yml` ignore list support.
- Auto-stage and auto-commit on `time` strategy.
- `gd up`, `gd down`, `gd status` commands.
- Basic IPC over Unix socket.

[Unreleased]: https://github.com/mugiwaraluffy56/gitdaemon/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/mugiwaraluffy56/gitdaemon/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/mugiwaraluffy56/gitdaemon/compare/v0.6.2...v0.7.0
[0.6.2]: https://github.com/mugiwaraluffy56/gitdaemon/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/mugiwaraluffy56/gitdaemon/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/mugiwaraluffy56/gitdaemon/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/mugiwaraluffy56/gitdaemon/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/mugiwaraluffy56/gitdaemon/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/mugiwaraluffy56/gitdaemon/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/mugiwaraluffy56/gitdaemon/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/mugiwaraluffy56/gitdaemon/releases/tag/v0.1.0
