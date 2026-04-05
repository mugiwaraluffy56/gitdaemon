# Troubleshooting

Common issues and how to fix them.

---

## Daemon won't start

### "not a git repository"

```
error: not a git repository at /path/to/dir: ...
```

`gd up` must be run from inside a Git repository (or a subdirectory of one).
The working directory must contain a `.git` folder.

```sh
git init   # if starting fresh
gd init
gd up
```

### "failed to load config from gd.yml"

The config file doesn't exist. Create it:

```sh
gd init
```

Or specify a path:

```sh
gd up --config /path/to/gd.yml
```

### "daemon already running (pid XXXXX)"

A daemon is already running for this repo. Use `gd down` to stop it, or check:

```sh
gd status
ps aux | grep gd
```

If the process is gone but the PID file is stale:

```sh
rm .gd/daemon.pid
gd up
```

---

## No commits being created

### Check that auto-stage is on

```yaml
repo:
  auto_stage: true
```

### Check the commit interval

The default is 120 seconds. Lower it to test:

```yaml
commit:
  interval: 10
```

### Check `gd status`

```sh
gd status
```

Look at `staged` and `watching` counts. If `watching: 0`, the watcher failed
to start (see logs).

### Run in foreground with debug logging

```sh
RUST_LOG=debug gd up
```

This will print every staged file, commit attempt, and ticker fire.

---

## Changes are staged but not committed

### Pre-commit hook is failing

```sh
RUST_LOG=debug gd up
# look for: "pre-commit hook exited with code N"
```

Fix or disable the hook:

```yaml
hooks:
  pre_commit: ""
```

### `change_count` threshold not reached

If your strategy is `change_count`, commits only happen once `change_threshold`
files have accumulated:

```yaml
commit:
  strategy: change_count
  change_threshold: 10   # lower this
```

---

## Push is not happening

### Auto-push is paused

```sh
gd status   # look for "push paused"
gd resume
```

### You're on a protected branch

```yaml
push:
  protected_branches:
    - main
    - master
```

Push manually with `gd push`, or remove your branch from the list.

### Secret scanning blocked the push

```sh
gd status   # look for "push blocked: secret detected"
```

See [secret-scanning.md](secret-scanning.md) for how to resolve.

### No remote configured

```sh
git remote -v
git remote add origin git@github.com:you/repo.git
```

### SSH authentication failing

```sh
ssh -T git@github.com   # test SSH key
ssh-add ~/.ssh/id_ed25519   # add key to agent if needed
```

Run with debug logging to see the exact `git2` error:

```sh
RUST_LOG=debug gd up
```

---

## `gd status` shows "daemon not running"

The daemon is not running. Start it:

```sh
gd up -d
```

If the socket file exists but the daemon isn't running:

```sh
rm .gd/daemon.sock .gd/daemon.pid
gd up -d
```

---

## `gd status` / `gd push` / `gd pause` hang

The daemon is not responding on the Unix socket. Check:

```sh
ps aux | grep gd      # is it running?
ls -la .gd/           # is daemon.sock present?
```

If it's stuck, force-kill and restart:

```sh
kill -9 $(cat .gd/daemon.pid)
rm .gd/daemon.sock .gd/daemon.pid
gd up -d
```

---

## Commits have wrong author

`gd` uses the Git identity from `git config`:

```sh
git config --global user.name "Your Name"
git config --global user.email "you@example.com"
```

Or set it per-repo in `.git/config`.

---

## AI commit messages not working

See [ai-commit-messages.md — Troubleshooting](ai-commit-messages.md#troubleshooting).

---

## High CPU usage

### Filesystem watcher firing too often

Add patterns to the ignore list to filter out high-frequency generated files:

```yaml
ignore:
  - "*.log"
  - "node_modules/"
  - "dist/"
  - ".next/"
  - "__pycache__/"
  - "*.pyc"
  - "target/"
```

### Commit interval too short

```yaml
commit:
  interval: 120   # default, increase if needed
```

---

## Collecting logs for bug reports

```sh
RUST_LOG=debug gd up 2>&1 | tee /tmp/gd-debug.log
# reproduce the issue
# review /tmp/gd-debug.log and sanitise before sharing
```

Always redact:
- API keys and tokens
- Repository paths if sensitive
- File contents visible in diff lines
