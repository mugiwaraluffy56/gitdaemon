# gitdaemon (gd) Configuration Reference

Complete reference for every field in `gd.yml`.

---

## Table of contents

1. [File location and format](#1-file-location-and-format)
2. [Top-level fields](#2-top-level-fields)
3. [`repo` — repository behaviour](#3-repo--repository-behaviour)
4. [`commit` — auto-commit settings](#4-commit--auto-commit-settings)
5. [`push` — auto-push settings](#5-push--auto-push-settings)
6. [`ignore` — staging exclusions](#6-ignore--staging-exclusions)
7. [`safety` — safety controls](#7-safety--safety-controls)
8. [`hooks` — shell hooks](#8-hooks--shell-hooks)
9. [`ai` — AI commit messages](#9-ai--ai-commit-messages)
10. [Full annotated example](#10-full-annotated-example)
11. [Validation rules](#11-validation-rules)
12. [Configuration recipes](#12-configuration-recipes)

---

## 1. File location and format

`gd.yml` must live at the **root of the Git repository** being managed. It is created by `gd init` and is YAML 1.1 (parsed by the `serde_yaml` crate).

```
my-project/
├── .git/
├── gd.yml          ← here
├── src/
└── ...
```

To use a different path, pass `--config <path>` to `gd up`:

```sh
gd up --config /etc/gd/project.yml
```

> **Version field is required.** Every `gd.yml` must start with `version: 1`.  
> If `version` is missing or not `1`, the daemon refuses to start.

---

## 2. Top-level fields

| Field | Type | Required | Default |
|---|---|---|---|
| `version` | integer | **yes** | — |
| `repo` | object | no | see [§3](#3-repo--repository-behaviour) |
| `commit` | object | no | see [§4](#4-commit--auto-commit-settings) |
| `push` | object | no | see [§5](#5-push--auto-push-settings) |
| `ignore` | list of strings | no | `[]` |
| `safety` | object | no | see [§7](#7-safety--safety-controls) |
| `hooks` | object | no | see [§8](#8-hooks--shell-hooks) |

---

## 3. `repo` — repository behaviour

Controls how `gd` interacts with the Git working tree at a foundational level.

```yaml
repo:
  auto_stage: true
  auto_fetch: true
  fetch_interval: 60
  auto_sync_base: true
  base_branch: main
  rebase_on_sync: true
```

All fields and their defaults:

| Field | Type | Default |
|---|---|---|
| `auto_stage` | boolean | `true` |
| `auto_fetch` | boolean | `true` |
| `fetch_interval` | integer (seconds) | `60` |
| `auto_sync_base` | boolean | `true` |
| `base_branch` | string | `"main"` |
| `rebase_on_sync` | boolean | `true` |

### `repo.auto_stage`

| | |
|---|---|
| **Type** | boolean |
| **Default** | `true` |

When `true`, `gd` calls the equivalent of `git add` on every modified, added, or deleted file each time the filesystem watcher fires. Files matching the `ignore` list or `.gitignore` are skipped. The `.gd/` and `.git/` directories are always excluded.

When `false`, `gd` still auto-commits and auto-pushes — but only what is **already staged**. You are responsible for staging files yourself (`git add`).

```yaml
# Manual staging workflow
repo:
  auto_stage: false
```

### `repo.auto_fetch`

| | |
|---|---|
| **Type** | boolean |
| **Default** | `true` |

When `true`, `gd` periodically fetches from all configured remotes. This updates remote-tracking refs (`origin/main`, etc.) without merging or rebasing. After each fetch, `gd` checks for divergence and pauses push if the remote has advanced ahead.

When `false`, no background fetch is performed. `gd status` may show stale `behind` counts.

### `repo.fetch_interval`

| | |
|---|---|
| **Type** | integer (seconds) |
| **Default** | `60` |
| **Minimum** | `1` |

How often (in seconds) to fetch from remotes. Lower values keep remote-tracking refs fresher at the cost of more network traffic.

```yaml
repo:
  fetch_interval: 300   # fetch every 5 minutes
```

### `repo.auto_sync_base`

| | |
|---|---|
| **Type** | boolean |
| **Default** | `true` |

When `true`, after every successful fetch `gd` will:
1. Fast-forward the local `base_branch` ref to `origin/<base_branch>` — **no checkout**, pure ref update
2. Rebase the current working branch onto it (if `rebase_on_sync: true` and the working tree is clean)

This keeps your feature branch on a fresh base automatically. You never need to run `git fetch`, `git checkout main`, `git pull`, or `git rebase main` manually.

When `false`, only the regular background fetch runs — no ref updates or rebasing.

### `repo.base_branch`

| | |
|---|---|
| **Type** | string |
| **Default** | `"main"` |

The long-lived branch to fast-forward after each fetch. Must match your repository's primary integration branch name.

```yaml
repo:
  base_branch: master   # for older repositories
```

```yaml
repo:
  base_branch: develop  # for gitflow-style repos
```

### `repo.rebase_on_sync`

| | |
|---|---|
| **Type** | boolean |
| **Default** | `true` |

When `true`, after fast-forwarding `base_branch`, `gd` runs `git rebase <base_branch>` on your current working branch. This is skipped silently when:

| Condition | Behaviour |
|---|---|
| Working tree has unsaved edits | Skip this cycle, retry on next fetch tick |
| Already on `base_branch` | Only fast-forward runs, no rebase needed |
| Repo in mid-rebase / mid-merge state | Skip until state is clean |
| `base_branch` did not advance | No-op |

If the rebase produces a **conflict**, `gd` immediately runs `git rebase --abort` to restore a clean working tree, pauses auto-push, and logs an error visible in `gd status`. Resolve the conflict manually, then run `gd resume`.

When `false`, `gd` only fast-forwards the local base branch ref. Your working branch stays where it is — you rebase or merge manually when you're ready.

```yaml
# Fast-forward main but don't touch the current branch
repo:
  auto_sync_base: true
  base_branch: main
  rebase_on_sync: false
```

---

## 4. `commit` — auto-commit settings

Controls when and how `gd` creates commits.

```yaml
commit:
  strategy: time
  interval: 120
  change_threshold: 10
  message: "{summary}"
```

### `commit.strategy`

| | |
|---|---|
| **Type** | string enum |
| **Default** | `time` |
| **Values** | `time`, `change_count` |

Selects which trigger fires a commit.

**`time`** — commits every `interval` seconds if there are staged changes. Nothing happens if the index is clean.

**`change_count`** — commits once `change_threshold` or more files have been staged since the last commit, regardless of elapsed time.

```yaml
# Time-based (default): checkpoint every 2 minutes
commit:
  strategy: time
  interval: 120

# Change-count: commit after every 5 files
commit:
  strategy: change_count
  change_threshold: 5
```

> **Choosing a strategy**  
> Use `time` when you want regular checkpoints while working.  
> Use `change_count` on slow machines or during bulk file operations where frequent timer commits would create too much noise.

### `commit.interval`

| | |
|---|---|
| **Type** | integer (seconds) |
| **Default** | `120` |
| **Minimum** | `1` |
| **Used by** | `time` strategy only |

How often the commit ticker fires. Only staged changes trigger an actual commit — if the index is empty, the tick is a no-op.

```yaml
commit:
  strategy: time
  interval: 60    # commit every minute if there's anything staged
```

### `commit.change_threshold`

| | |
|---|---|
| **Type** | integer (file count) |
| **Default** | `10` |
| **Minimum** | `1` |
| **Used by** | `change_count` strategy only |

The number of staged files that must accumulate before a commit is created. `gd` counts files as they enter the index; once the running total reaches this threshold, it commits immediately.

```yaml
commit:
  strategy: change_count
  change_threshold: 20   # wait until 20 files are staged
```

### `commit.message`

| | |
|---|---|
| **Type** | string (template) |
| **Default** | `"{summary}"` |
| **Must not be** | empty or whitespace-only |

A template for the commit message. `gd` first generates a **conventional commit summary** from the diff (e.g. `feat(git): introduce PushQueue`), then substitutes tokens into this template.

#### Available tokens

| Token | Value |
|---|---|
| `{summary}` | Full generated conventional commit line |
| `{count}` | Number of files changed in this commit |
| `{branch}` | Current Git branch name |
| `{timestamp}` | ISO-8601 UTC timestamp (`2025-04-01T12:00:00Z`) |

Unknown tokens are left verbatim — `{ticket}` stays `{ticket}` if not a known token.

#### Examples

```yaml
# Default — use generated message verbatim
message: "{summary}"
# → feat(git): introduce PushQueue and implement try_push

# Prepend a Jira ticket
message: "[PROJ-123] {summary}"
# → [PROJ-123] feat(git): introduce PushQueue and implement try_push

# Include branch in message
message: "[{branch}] {summary}"
# → [main] feat(git): introduce PushQueue and implement try_push

# Include file count
message: "{summary} ({count} files changed)"
# → feat(git): introduce PushQueue and implement try_push (3 files changed)

# Append an ISO timestamp
message: "{summary}\n\nTimestamp: {timestamp}"
# → multiline commit with timestamp in body

# Minimal custom prefix
message: "auto: {summary}"
# → auto: feat(git): introduce PushQueue
```

#### How the summary is generated

The `{summary}` token is a full conventional commit line:

```
<type>(<scope>): <subject>
```

**Type** is inferred from what changed:

| Type | Condition |
|---|---|
| `feat` | New public types (struct/enum/trait) added, or majority of deltas are new source files |
| `fix` | Majority of deltas are modifications to existing source files |
| `refactor` | Mix of adds + deletes, or all renames |
| `chore` | All deletions |
| `test` | All files are test files (`tests/`, `*_test.*`, `test_*`) |
| `docs` | All files are documentation (`.md`, `.rst`, `README`, …) |
| `build` | All files are build/config (`Cargo.toml`, `package.json`, `Makefile`, …) |

**Scope** is the first meaningful subdirectory under `src/` when ≥ 60% of changed files share it:

| Changed files | Scope |
|---|---|
| `src/git/push.rs`, `src/git/fetch.rs` | `git` |
| `src/daemon/ipc.rs` | `daemon` |
| `src/config.rs` | `config` |
| `README.md` | *(none)* |

**Subject** is extracted from public symbol declarations found in the diff:

| Situation | Verb | Example subject |
|---|---|---|
| New `struct`/`enum`/`trait` | `introduce` | `introduce PushQueue` |
| New `fn`/`async fn` | `implement` | `implement fetch_all_remotes` |
| Same symbol in `+` and `-` lines | `rework` | `rework validate` |
| Symbol only in `-` lines | `drop` | `drop legacy_fetch` |
| No symbols found | file-based fallback | `update config` |

Recognised languages: **Rust**, **TypeScript**, **JavaScript**, **Python**, **Go**.

For changesets larger than 5 files a body is automatically appended:

```
feat(git): introduce PushQueue and implement try_push

- add(git): PushQueue, PushState, PushResult
- add(git): try_push, record_commits, push_now
- update src/git/mod.rs
```

---

## 5. `push` — auto-push settings

Controls how and when commits are pushed to the remote.

```yaml
push:
  strategy: batch
  interval: 300
  branch: main
```

### `push.strategy`

| | |
|---|---|
| **Type** | string enum |
| **Default** | `batch` |
| **Values** | `batch` |

The only current strategy is `batch`: commits are queued locally and pushed together when the push ticker fires. This avoids hammering the remote with one push per commit.

### `push.interval`

| | |
|---|---|
| **Type** | integer (seconds) |
| **Default** | `300` |
| **Minimum** | `1` |

How often (in seconds) the push ticker fires. All queued commits are pushed in a single `git push` call.

```yaml
push:
  interval: 60    # push every minute
```

If three consecutive push attempts fail, auto-push is **automatically paused** and an error is logged. Use `gd resume` to re-enable it.

### `push.branch`

| | |
|---|---|
| **Type** | string |
| **Default** | `"main"` |
| **Must not be** | empty or whitespace-only |

The local branch to push. Must match your repository's primary branch name.

```yaml
push:
  branch: master    # for older repositories
```

> **Note:** `gd` pushes to `origin/<branch>`. Multi-remote setups are not currently configurable — all pushes go to `origin`.

### `push.protected_branches`

| | |
|---|---|
| **Type** | list of strings |
| **Default** | `["main", "master", "develop"]` |

Branches that `gd` must **never auto-push to**. When the current working branch is in this list, auto-push is silently skipped. Staging and committing continue normally — only the automatic push is blocked.

Use `gd push` to push to a protected branch manually when you intend to.

```yaml
push:
  protected_branches:
    - main
    - master
    - release
    - production
```

To disable the guard (allow auto-push to any branch):

```yaml
push:
  protected_branches: []
```

---

## 6. `ignore` — staging exclusions

A list of gitignore-style glob patterns. Files matching any pattern are **not staged** by `gd`, even if `auto_stage: true`.

```yaml
ignore:
  - "*.log"
  - "node_modules/"
  - ".env"
  - "dist/"
  - "target/"
  - "*.tmp"
  - "coverage/"
  - ".DS_Store"
```

### Pattern syntax

Patterns follow `.gitignore` glob conventions:

| Pattern | Matches |
|---|---|
| `*.log` | Any `.log` file in any directory |
| `node_modules/` | The `node_modules` directory (trailing `/` anchors to directories) |
| `.env` | A file named `.env` in any directory |
| `dist/` | The `dist` directory |
| `src/**/*.test.ts` | All `.test.ts` files anywhere under `src/` |
| `!important.log` | Un-ignore `important.log` even if `*.log` matches |

### Interaction with `.gitignore`

`gd` respects `.gitignore` independently — files already excluded by `.gitignore` are also not staged, regardless of the `ignore` list here. The `ignore` list is additive: it only adds more exclusions.

### Always-excluded directories

The following are always excluded regardless of configuration:

- `.gd/` — daemon state files
- `.git/` — Git internals

---

## 7. `safety` — safety controls

Guards against accidental leaks and destructive pushes.

```yaml
safety:
  confirm_first: false
  block_secrets: true
```

### `safety.confirm_first`

| | |
|---|---|
| **Type** | boolean |
| **Default** | `false` |

When `true`, `gd` prompts for confirmation before the **first push** in each daemon session. Subsequent pushes in the same session proceed automatically.

> Currently only meaningful when running `gd up` in the foreground — background daemons cannot prompt interactively.

```yaml
safety:
  confirm_first: true
```

### `safety.block_secrets`

| | |
|---|---|
| **Type** | boolean |
| **Default** | `true` |

When `true`, `gd` scans the diff of every push attempt for secret patterns. If any pattern matches a `+` line (added content), the push is **blocked** and an error is logged. The commit remains on your local branch — nothing is rolled back.

Detected patterns:

| Pattern | Secret type |
|---|---|
| `AKIA[0-9A-Z]{16}` | AWS Access Key ID |
| `aws_secret_access_key\s*=\s*.+` | AWS Secret Access Key |
| `sk_live_[0-9a-zA-Z]{24,}` | Stripe live secret key |
| `ghp_[0-9a-zA-Z]{36}` | GitHub Personal Access Token |
| `ghs_[0-9a-zA-Z]{36}` | GitHub App installation token |
| `-----BEGIN .* PRIVATE KEY-----` | Private key header (RSA/EC/OPENSSH) |
| `AIza[0-9A-Za-z_\-]{35}` | Google API key |
| `xox[baprs]-[0-9A-Za-z\-]+` | Slack token |
| `password\s*=\s*["'][^"']{8,}["']` | Hardcoded password (8+ chars) |
| `api_key\s*=\s*["'][^"']{8,}["']` | Hardcoded API key/token |

To fix a blocked push:
1. Scrub or rotate the secret from your working tree
2. Amend or revert the commit that introduced it (`git commit --amend` or `git revert`)
3. `gd` will push cleanly on the next push tick, or trigger one with `gd push`

To disable (strongly not recommended):

```yaml
safety:
  block_secrets: false
```

---

## 8. `hooks` — shell hooks

Shell commands that run at commit lifecycle points. Both hooks run with `sh -c "<command>"` and inherit the daemon's environment. The working directory is set to the repository root.

```yaml
hooks:
  pre_commit: ""
  post_commit: ""
```

### `hooks.pre_commit`

| | |
|---|---|
| **Type** | string (shell command) |
| **Default** | `""` (disabled) |

Runs **before** the commit is created. If the command exits non-zero:
- The commit is **aborted**
- Staged files remain staged — no work is lost
- The skip reason is recorded in the daemon's internal log

```yaml
hooks:
  # Abort if code isn't formatted
  pre_commit: "cargo fmt --check"

  # Abort if linting fails
  pre_commit: "cargo fmt --check && cargo clippy -- -D warnings"

  # JavaScript projects
  pre_commit: "prettier --check . && eslint src/"

  # Python projects
  pre_commit: "black --check . && ruff check ."

  # Run fast unit tests before committing
  pre_commit: "python -m pytest tests/unit -q"
```

> **Keep pre_commit fast.** It runs before every auto-commit. A hook that takes 30 seconds will significantly delay your commit rhythm. Reserve slow operations (`cargo test`, full test suite) for `post_commit`.

### `hooks.on_push_success`

| | |
|---|---|
| **Type** | string (shell command) |
| **Default** | `""` (disabled) |

Runs **after a successful push**. Environment variables available: `$FG_BRANCH` (branch pushed), `$FG_COMMITS` (number of commits pushed). Exit code is ignored.

```yaml
hooks:
  on_push_success: "notify-send "gd" 'pushed $FG_COMMITS commits on $FG_BRANCH'"
```

### `hooks.on_conflict`

| | |
|---|---|
| **Type** | string (shell command) |
| **Default** | `""` (disabled) |

Runs when a **rebase conflict or push conflict** is detected. Environment variables: `$FG_BRANCH`, `$FG_ERROR` (error message). Exit code is ignored.

```yaml
hooks:
  on_conflict: "osascript -e 'display alert \"fg conflict\" message \"$FG_ERROR\"'"
```

### `hooks.post_commit`

| | |
|---|---|
| **Type** | string (shell command) |
| **Default** | `""` (disabled) |

Runs **after** the commit is created. If the command exits non-zero:
- The failure is **logged** as a warning
- The commit is **not** rolled back
- The daemon continues normally

```yaml
hooks:
  # Run tests after every commit
  post_commit: "cargo test --quiet"

  # Trigger a build
  post_commit: "make build"

  # Notify a webhook
  post_commit: "curl -s -X POST $WEBHOOK_URL -d '{\"event\": \"commit\"}'"

  # Run the full test suite after committing
  post_commit: "python -m pytest -q"
```

### Combining both hooks

```yaml
hooks:
  pre_commit: "cargo fmt --check && cargo clippy -- -D warnings"
  post_commit: "cargo test --quiet"
```

This gates formatting and lint errors (commit aborted if they fail) while running tests asynchronously after the commit lands.

---

## 9. Full annotated example

A production-ready `gd.yml` for a Rust project:

```yaml
version: 1

repo:
  # Stage all working-tree changes automatically
  auto_stage: true
  # Keep remote-tracking refs fresh
  auto_fetch: true
  # Fetch every 2 minutes
  fetch_interval: 120
  # Keep main up to date and rebase feature branch onto it automatically
  auto_sync_base: true
  base_branch: main
  rebase_on_sync: true

commit:
  # Commit every 3 minutes if changes are staged
  strategy: time
  interval: 180
  # No custom prefix — use the generated conventional commit verbatim
  message: "{summary}"

push:
  strategy: batch
  # Push every 10 minutes
  interval: 600
  branch: main

# Don't stage these
ignore:
  - "*.log"
  - "target/"         # Rust build output — large and irrelevant
  - ".env"
  - ".env.local"
  - "*.tmp"
  - ".DS_Store"

safety:
  # No interactive prompt before first push
  confirm_first: false
  # Block pushes if a secret pattern is detected
  block_secrets: true

hooks:
  # Abort commit if code is not formatted or lint fails
  pre_commit: "cargo fmt --check && cargo clippy --quiet -- -D warnings"
  # Run tests after every commit (non-zero logged but commit kept)
  post_commit: "cargo test --quiet 2>&1"
```

---

A `gd.yml` for a Node.js/TypeScript project:

```yaml
version: 1

repo:
  auto_stage: true
  auto_fetch: true
  fetch_interval: 60

commit:
  strategy: time
  interval: 120
  message: "{summary}"

push:
  strategy: batch
  interval: 300
  branch: main

ignore:
  - "node_modules/"
  - "dist/"
  - "build/"
  - ".next/"
  - "*.log"
  - ".env"
  - ".env.local"
  - ".env.*.local"
  - "coverage/"
  - ".DS_Store"

safety:
  confirm_first: false
  block_secrets: true

hooks:
  pre_commit: "npx prettier --check . && npx eslint src/ --quiet"
  post_commit: "npm test -- --silent"
```

---

A `gd.yml` for a Python project with aggressive commit frequency:

```yaml
version: 1

repo:
  auto_stage: true
  auto_fetch: true
  fetch_interval: 30

commit:
  # Commit once 5 files accumulate — good for Jupyter notebooks
  strategy: change_count
  change_threshold: 5
  message: "[{branch}] {summary}"  # always include branch in message

push:
  strategy: batch
  interval: 180
  branch: develop

ignore:
  - "__pycache__/"
  - "*.pyc"
  - "*.pyo"
  - ".venv/"
  - "venv/"
  - ".env"
  - "*.egg-info/"
  - "dist/"
  - "build/"
  - ".pytest_cache/"
  - "htmlcov/"
  - ".ipynb_checkpoints/"

safety:
  confirm_first: true   # prompt before first push
  block_secrets: true

hooks:
  pre_commit: "black --check . && ruff check ."
  post_commit: "python -m pytest tests/ -q --tb=no"
```

---

## 10. Validation rules

`gd` validates `gd.yml` on startup. Any violation causes the daemon to refuse to start.

| Rule | Error message |
|---|---|
| `version` must equal `1` | `invalid version: N (must be 1)` |
| `commit.interval` must be ≥ 1 | `commit.interval must be > 0` |
| `push.interval` must be ≥ 1 | `push.interval must be > 0` |
| `commit.change_threshold` must be ≥ 1 | `commit.change_threshold must be > 0` |
| `push.branch` must not be empty | `push.branch cannot be empty` |
| `commit.message` must not be empty or whitespace | `commit.message cannot be empty` |

Unknown fields in `gd.yml` are silently ignored (forward-compatible with future versions).

---

## 11. Configuration recipes

### Disable all automation except status monitoring

```yaml
version: 1
repo:
  auto_stage: false
  auto_fetch: true
  fetch_interval: 60
commit:
  strategy: time
  interval: 3600   # very long — effectively never commits
push:
  interval: 3600
  branch: main
```

Then `gd pause` to stop auto-push entirely. Use `gd push` to push on demand.

### Commit message with ticket prefix from environment

```yaml
commit:
  message: "[${JIRA_TICKET:-no-ticket}] {summary}"
```

Set `JIRA_TICKET=PROJ-456` in your shell before starting the daemon.

### High-frequency local checkpointing with infrequent push

```yaml
commit:
  strategy: time
  interval: 60       # checkpoint every minute
push:
  interval: 1800     # push every 30 minutes
```

### Quiet mode — minimal noise for large refactors

```yaml
commit:
  strategy: change_count
  change_threshold: 50   # only commit after 50 files staged
push:
  interval: 900
hooks:
  pre_commit: ""         # no hooks
  post_commit: ""
```

### Maximum safety before pushing

```yaml
safety:
  confirm_first: true
  block_secrets: true
hooks:
  pre_commit: "cargo fmt --check && cargo test --quiet"
```

---

## 9. `ai` — AI commit messages

When `ai.enabled` is `true`, `gd` calls the Anthropic Messages API to generate
a conventional commit message from the staged diff instead of using the
built-in heuristic generator. The heuristic is always used as a fallback if
the API is unreachable or the key is missing.

See [ai-commit-messages.md](ai-commit-messages.md) for the full guide.

### Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Enable AI-generated commit messages |
| `api_key` | string | `""` | Key resolution (see below) |
| `model` | string | `"claude-haiku-4-5-20251001"` | Claude model |
| `max_diff_chars` | usize | `12000` | Max diff characters sent to the API |

### `api_key` resolution

The key is resolved in this order:

1. If the value starts with `env:`, read the named environment variable:
   `api_key: "env:MY_KEY"` → reads `$MY_KEY`.
2. If the value is a non-empty string that does not start with `env:`, use it
   literally (not recommended for version-controlled files).
3. If the value is empty (the default), auto-load `.env` from the repo root and
   then read `ANTHROPIC_API_KEY` from the environment.

### Model options

| Model ID | Speed | Cost | Recommended for |
|---|---|---|---|
| `claude-haiku-4-5-20251001` | Fast | Low | Default — everyday commits |
| `claude-sonnet-4-6` | Medium | Medium | Complex multi-file changes |
| `claude-opus-4-6` | Slow | High | Large refactors, squash commits |

### Example

```yaml
ai:
  enabled: true
  api_key: ""                        # reads ANTHROPIC_API_KEY from env / .env
  model: "claude-haiku-4-5-20251001"
  max_diff_chars: 12000
```

With `env:` reference:

```yaml
ai:
  enabled: true
  api_key: "env:ANTHROPIC_API_KEY"
  model: "claude-sonnet-4-6"
  max_diff_chars: 8000
```

---

*For runtime control commands (`gd pause`, `gd resume`, `gd push`, `gd status`), see the [User Guide](user-guide.md).*
