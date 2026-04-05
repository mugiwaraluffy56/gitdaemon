# Secret scanning

`gd` scans the full diff **before every push** to prevent credentials and
API keys from leaving your machine. If a secret is detected, the push is
blocked and the commit is held locally until you remove it.

---

## Enabling and disabling

Secret scanning is on by default:

```yaml
safety:
  block_secrets: true   # default
```

To disable (not recommended):

```yaml
safety:
  block_secrets: false
```

---

## Detected patterns

| Category | Pattern | Example match |
|---|---|---|
| AWS Access Key ID | `AKIA[0-9A-Z]{16}` | `AKIAIOSFODNN7EXAMPLE` |
| AWS Secret Access Key | high-entropy 40-char base64 after `aws_secret` | `wJalrXUtnFEMI/K7MDENG/...` |
| GitHub PAT (classic) | `ghp_[0-9a-zA-Z]{36}` | `ghp_abc123...` |
| GitHub fine-grained | `github_pat_[0-9a-zA-Z_]{82}` | `github_pat_...` |
| Stripe live secret | `sk_live_[0-9a-zA-Z]{24,}` | `sk_live_abc...` |
| Google API key | `AIza[0-9A-Za-z\-_]{35}` | `AIzaSy...` |
| Slack bot token | `xoxb-[0-9A-Za-z\-]{50,}` | `xoxb-123-456-...` |
| Slack user token | `xoxp-[0-9A-Za-z\-]{50,}` | `xoxp-...` |
| RSA private key | `-----BEGIN RSA PRIVATE KEY-----` | (header line) |
| EC private key | `-----BEGIN EC PRIVATE KEY-----` | (header line) |
| Generic private key | `-----BEGIN PRIVATE KEY-----` | (header line) |
| OpenSSH private key | `-----BEGIN OPENSSH PRIVATE KEY-----` | (header line) |
| Generic `password` | `password\s*=\s*['"][^'"]{8,}['"]` | `password = "hunter2"` |
| Generic `api_key` | `api_key\s*=\s*['"][^'"]{8,}['"]` | `api_key = "abc..."` |
| Generic `secret` | `secret\s*=\s*['"][^'"]{8,}['"]` | `secret = "xyz..."` |
| Generic `token` | `token\s*=\s*['"][^'"]{8,}['"]` | `token = "tok_..."` |

All patterns are applied to the full diff text — not just `+` lines — so
that secrets in context lines are also caught.

---

## What happens when a secret is detected

1. The push is **blocked** — no data leaves your machine.
2. `gd status` shows a `✗ push blocked: secret detected` health state.
3. The daemon logs the match: file, line number, and the pattern that matched
   (the actual secret value is never logged).
4. Commits are **not** undone — your work is safe locally.

To unblock:

1. Remove or rotate the secret from the affected file.
2. Amend the commit or create a new commit to overwrite it: `gd undo && gd push`.
3. Verify with `git diff HEAD~1` that the secret is gone.

---

## False positives

Patterns are intentionally conservative to minimise false positives. If you
encounter one:

**Option 1 — Use a placeholder in the config template**

```python
# config.py
API_KEY = os.environ["MY_API_KEY"]   # not a literal — won't match
```

**Option 2 — Shorten the value below the minimum length**

The `password`, `api_key`, `secret`, and `token` patterns require values of
at least 8 characters. Test passwords like `password = "test"` won't match.

**Option 3 — Disable scanning for a specific push**

```sh
gd pause            # pause auto-push
# manually: git push
gd resume
```

**Option 4 — Disable scanning entirely**

```yaml
safety:
  block_secrets: false
```

---

## Scanning internals

The scanner lives in [src/git/secrets.rs](../src/git/secrets.rs). All patterns
are compiled once at startup using `once_cell::sync::Lazy` to avoid
recompilation on every scan. The scan is performed synchronously in a
`spawn_blocking` task to avoid blocking the async runtime.

The diff scanned is the full `git diff HEAD` output before the push, not just
the staged diff — this catches secrets that were committed in earlier ticks and
are now queued for pushing.

---

## Reporting a missed pattern

If you know of a common credential format that `gd` doesn't detect, please
[open an issue](https://github.com/mugiwaraluffy56/gitdaemon/issues/new?template=feature_request.yml)
with:

- The token format (redact actual values)
- A public source describing the format (docs, blog post, etc.)
- Whether a regex would have a high false-positive rate
