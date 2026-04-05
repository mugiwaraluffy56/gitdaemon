# Hooks

`gd` supports four shell hooks that fire at defined points in the commit/push
lifecycle. Hooks are configured in `gd.yml` and run in the repository root.

---

## Hook overview

| Hook | When it fires | Non-zero exit |
|---|---|---|
| `pre_commit` | Before creating each auto-commit | Aborts the commit |
| `post_commit` | After a successful commit | Logged as warning, commit kept |
| `on_push_success` | After a successful push | Logged as warning |
| `on_conflict` | When a rebase or merge conflict is detected | Logged as warning |

---

## Configuration

```yaml
hooks:
  pre_commit: ""
  post_commit: ""
  on_push_success: ""
  on_conflict: ""
```

Each field is a shell command string executed with `sh -c`. Leave it empty
(the default) to disable.

---

## `pre_commit`

Runs before `gd` calls `git commit`. A non-zero exit code **aborts** the
commit — staged changes are left in the index and will be retried on the next
commit tick.

**Common uses:**

```yaml
hooks:
  # Abort commit if code doesn't format cleanly
  pre_commit: "cargo fmt --check"

  # Abort commit if there are clippy warnings
  pre_commit: "cargo clippy --quiet -- -D warnings 2>&1"

  # Multiple checks with &&
  pre_commit: "cargo fmt --check && cargo clippy --quiet -- -D warnings"

  # Node.js projects
  pre_commit: "npm run lint --silent"

  # Python projects
  pre_commit: "ruff check . --quiet"
```

**Note:** Keep pre-commit hooks fast. They run on every commit tick — a slow
hook (e.g. running a full test suite) will delay all commits. Use `post_commit`
for slower checks.

---

## `post_commit`

Runs after a successful commit. A non-zero exit is **logged but not fatal** —
the commit is kept and the daemon continues.

**Common uses:**

```yaml
hooks:
  # Run tests after each commit (failure doesn't undo the commit)
  post_commit: "cargo test --quiet 2>&1 | tail -5"

  # Notify on macOS
  post_commit: "osascript -e 'display notification \"committed\" with title \"gd\"'"

  # Log to a file
  post_commit: "echo \"$(date): committed\" >> /tmp/gd.log"
```

---

## `on_push_success`

Fires after a successful push. Receives two environment variables:

| Variable | Value |
|---|---|
| `$FG_BRANCH` | The branch that was pushed |
| `$FG_COMMITS` | Number of commits pushed in this batch |

**Common uses:**

```yaml
hooks:
  # macOS notification
  on_push_success: |
    osascript -e "display notification \"pushed $FG_COMMITS commit(s)\" with title \"gd → $FG_BRANCH\""

  # Slack webhook (requires curl)
  on_push_success: |
    curl -s -X POST "$SLACK_WEBHOOK_URL" \
      -H 'Content-type: application/json' \
      --data "{\"text\":\"Pushed $FG_COMMITS commit(s) to \`$FG_BRANCH\`\"}"

  # Linux desktop notification
  on_push_success: "notify-send 'gd' \"pushed $FG_COMMITS to $FG_BRANCH\""
```

---

## `on_conflict`

Fires when `gd` detects a merge conflict or failed rebase. Push is
automatically paused. The hook receives:

| Variable | Value |
|---|---|
| `$FG_BRANCH` | The current branch |
| `$FG_ERROR` | Short error description |

After resolving the conflict manually, run `gd resume` to re-enable auto-push.

**Common uses:**

```yaml
hooks:
  # macOS notification
  on_conflict: |
    osascript -e "display alert \"gd conflict on $FG_BRANCH\" message \"$FG_ERROR\""

  # Linux desktop notification
  on_conflict: "notify-send -u critical 'gd conflict' \"$FG_ERROR on $FG_BRANCH\""

  # Play a sound (macOS)
  on_conflict: "afplay /System/Library/Sounds/Basso.aiff"
```

---

## Examples

### Rust project — full quality gate

```yaml
hooks:
  pre_commit: "cargo fmt --check && cargo clippy --quiet -- -D warnings"
  post_commit: "cargo test --quiet 2>&1 | tail -3"
  on_push_success: "osascript -e 'display notification \"pushed\" with title \"gd\"'"
  on_conflict: "osascript -e 'display alert \"Conflict!\" message \"$FG_ERROR\"'"
```

### Node.js project

```yaml
hooks:
  pre_commit: "npm run lint --silent && npm run type-check --silent"
  post_commit: "npm test -- --silent 2>&1 | tail -5"
```

### Python project

```yaml
hooks:
  pre_commit: "ruff check . --quiet && mypy . --quiet"
  post_commit: "pytest -q --tb=short 2>&1 | tail -10"
```

### Minimal — format only

```yaml
hooks:
  pre_commit: "gofmt -l . | grep -q . && exit 1 || exit 0"
```

---

## Hook working directory

All hooks run with the **repository root** as the working directory —
equivalent to `sh -c '<command>'` run from the repo root.

## Hook environment

Hooks inherit the daemon's full environment, including:
- All variables loaded from `.env` in the repo root (via `dotenvy`)
- `PATH` as seen by the process that ran `gd up`
- `$FG_BRANCH`, `$FG_COMMITS`, `$FG_ERROR` where documented above

## Debugging hooks

Run the daemon in the foreground with debug logging to see hook output:

```sh
RUST_LOG=debug gd up
```

Hook stdout and stderr are captured and logged at the `debug` level.
