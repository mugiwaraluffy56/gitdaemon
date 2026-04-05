# AI-generated commit messages

`gd` can generate commit messages using [Claude](https://claude.ai) instead of
its built-in heuristic generator. Both systems produce conventional commit
messages — the AI path is optional, opt-in, and gracefully falls back to the
heuristic on any error.

---

## How it works

When `ai.enabled: true`, before creating each commit `gd`:

1. Collects the full unified diff of the staged changes.
2. Truncates the diff to `ai.max_diff_chars` characters (default 12 000) at a
   clean newline boundary to stay within the model's context window.
3. Posts the diff to the Anthropic Messages API with a prompt asking for a
   conventional commit message.
4. Uses the returned message as the `{summary}` token in the commit message
   template.

If the API call fails for any reason — network error, missing key, rate limit,
server error — `gd` logs a debug warning and falls back to the heuristic
generator automatically. **Commits are never blocked by the AI path.**

---

## Enabling AI commit messages

### 1. Get an Anthropic API key

Sign up at [console.anthropic.com](https://console.anthropic.com) and create
an API key. The key starts with `sk-ant-`.

### 2. Store the key

**Option A — `.env` file in the repo root (recommended for personal repos)**

```sh
echo "ANTHROPIC_API_KEY=sk-ant-..." >> .env
echo ".env" >> .gitignore   # make sure it's ignored
```

`gd` automatically loads `.env` from the repo root before resolving the key.

**Option B — environment variable**

```sh
export ANTHROPIC_API_KEY=sk-ant-...
# or add to ~/.zshrc / ~/.bashrc
```

**Option C — inline in gd.yml (not recommended)**

```yaml
ai:
  api_key: "sk-ant-..."   # committed to the repo — avoid this
```

**Option D — reference an env var by name**

```yaml
ai:
  api_key: "env:MY_ANTHROPIC_KEY"
```

This reads `MY_ANTHROPIC_KEY` from the environment at runtime.

### 3. Enable in gd.yml

```yaml
ai:
  enabled: true
  # api_key: ""   ← leave blank to use ANTHROPIC_API_KEY from env / .env
  model: "claude-haiku-4-5-20251001"
  max_diff_chars: 12000
```

### 4. Restart the daemon

```sh
gd down && gd up -d
```

---

## Configuration reference

| Field | Type | Default | Description |
|---|---|---|---|
| `ai.enabled` | bool | `false` | Enable AI commit messages |
| `ai.api_key` | string | `""` | Key, `env:VAR`, or blank (reads `ANTHROPIC_API_KEY`) |
| `ai.model` | string | `"claude-haiku-4-5-20251001"` | Claude model to use |
| `ai.max_diff_chars` | usize | `12000` | Max diff chars sent to the API |

### Choosing a model

| Model | Speed | Cost | Quality | Best for |
|---|---|---|---|---|
| `claude-haiku-4-5-20251001` | Fast | Low | Good | Default — most commits |
| `claude-sonnet-4-6` | Medium | Medium | Better | Complex multi-file changes |
| `claude-opus-4-6` | Slow | High | Best | Large refactors / squash |

Haiku is the recommended default. It produces good conventional commit messages
for typical diffs and completes in under a second.

---

## Output quality

The AI is prompted to follow the conventional commit specification:

```
<type>(<scope>): <subject>

[optional body with bullet points for large changesets]
```

Example outputs for real diffs:

```
feat(ai_commit): add generate_ai_commit_message with reqwest HTTP client
```
```
refactor(daemon): extract DaemonContext and remove positional argument threading
```
```
fix(push): handle non-fast-forward rejection and pause queue on conflict

- detect REJECTED status from remote
- set push_paused flag and log warning
- fire on_conflict hook with $FG_ERROR populated
```
```
chore: update reqwest to 0.12, add dotenvy for .env loading
```

---

## Interaction with the heuristic generator

The two systems coexist:

| `ai.enabled` | API reachable | Message source |
|---|---|---|
| `false` | — | Heuristic always |
| `true` | yes | AI |
| `true` | no (any error) | Heuristic fallback |

The heuristic generator is described in the [user guide](user-guide.md#6-commit-message-generation).
It parses symbol names from the diff and produces messages like
`feat(git): introduce PushQueue and implement try_push` without any network call.

---

## Privacy considerations

When AI commit messages are enabled, the **staged diff** is sent to Anthropic's
API. This includes the content of the changed files. Before enabling, consider:

- Does your repo contain proprietary algorithms or confidential business logic?
- Does it contain personal data covered by GDPR / CCPA?
- Is your `gd.yml` or `.env` file properly gitignored?

For repos where this is a concern, use the heuristic generator (the default).

You can also use `ai.max_diff_chars` to limit the amount of code sent:

```yaml
ai:
  enabled: true
  max_diff_chars: 4000   # send only the first ~100 lines of diff
```

---

## Troubleshooting

### "ANTHROPIC_API_KEY is not present in the environment or .env"

The key is not set. Check:

```sh
echo $ANTHROPIC_API_KEY          # should print the key
cat .env | grep ANTHROPIC        # if using .env
```

### "Anthropic API returned 401"

The API key is invalid or has been revoked. Generate a new one at
[console.anthropic.com](https://console.anthropic.com).

### "Anthropic API returned 429"

Rate limited. The fallback heuristic generator was used for this commit.
Consider switching to a higher-tier plan or using a lower-frequency commit
strategy.

### AI messages look generic / low quality

The diff being sent may be too large and getting truncated. Try:

```yaml
ai:
  max_diff_chars: 8000    # tighter window
  model: "claude-sonnet-4-6"  # stronger model
```

Alternatively, reduce the commit interval so each commit covers fewer files.

### Disable AI without editing gd.yml

```sh
ANTHROPIC_API_KEY=""  gd up    # key is blank → heuristic used
```
