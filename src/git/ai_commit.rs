//! AI-powered commit message generation via the Anthropic Messages API.
//!
//! When `ai.enabled = true` in `gd.yml`, the daemon calls this module instead
//! of the heuristic `build_summary` generator.  If the API call fails for any
//! reason (network error, missing key, rate-limit, etc.) the caller is
//! expected to fall back to the heuristic generator so that commits are never
//! blocked.

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::debug;

use crate::config::AiConfig;

// ============================================================================
// Anthropic Messages API types (minimal — only what we need)
// ============================================================================

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<Message<'a>>,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

// ============================================================================
// Public API
// ============================================================================

/// Call the Claude API and return a conventional commit message for `diff`.
///
/// `repo_root` is used to locate a `.env` file when resolving the API key.
///
/// Returns an error if the key is missing, the network request fails, or the
/// response cannot be parsed.  The caller should fall back to the heuristic
/// generator on any error.
pub async fn generate_ai_commit_message(
    diff: &str,
    ai_cfg: &AiConfig,
    repo_root: &Path,
) -> Result<String> {
    let api_key = ai_cfg
        .resolve_api_key(repo_root)
        .context("failed to resolve Anthropic API key")?;

    let diff_slice = truncate_diff(diff, ai_cfg.max_diff_chars);

    let prompt = format!(
        "Generate a conventional commit message for the following git diff.\n\
         \n\
         Rules:\n\
         - Format: <type>(<scope>): <subject>\n\
         - Types: feat, fix, refactor, chore, test, docs, build, ci, perf, style\n\
         - Subject: imperative mood, ≤72 chars, no trailing period\n\
         - Scope: the module or area affected (omit when changes span many areas)\n\
         - For large changesets add a blank line then ≤3 concise bullet points as the body\n\
         - Output ONLY the commit message — no explanation, no markdown fences, no quotes\n\
         \n\
         Git diff:\n\
         ```\n\
         {}\n\
         ```",
        diff_slice
    );

    debug!(
        model = %ai_cfg.model,
        diff_chars = diff_slice.len(),
        "requesting AI commit message"
    );

    let client = Client::new();
    let request = MessagesRequest {
        model: &ai_cfg.model,
        max_tokens: 256,
        messages: vec![Message {
            role: "user",
            content: &prompt,
        }],
    };

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
        .context("HTTP request to Anthropic API failed")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Anthropic API returned {}: {}",
            status,
            body.trim()
        ));
    }

    let parsed: MessagesResponse = response
        .json()
        .await
        .context("failed to parse Anthropic API response as JSON")?;

    let text = parsed
        .content
        .into_iter()
        .find(|b| b.kind == "text")
        .and_then(|b| b.text)
        .ok_or_else(|| anyhow!("Anthropic API response contained no text block"))?;

    let message = text.trim().to_string();
    debug!(message = %message, "AI commit message generated");
    Ok(message)
}

// ============================================================================
// Helpers
// ============================================================================

/// Truncate `diff` to at most `max_chars` characters, cutting at the last
/// newline before the limit so we never split a diff line in the middle.
fn truncate_diff(diff: &str, max_chars: usize) -> &str {
    if diff.len() <= max_chars {
        return diff;
    }
    // Find a clean newline boundary.
    let boundary = diff[..max_chars]
        .rfind('\n')
        .unwrap_or(max_chars);
    &diff[..boundary]
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_diff_unchanged() {
        let diff = "hello\nworld\n";
        assert_eq!(truncate_diff(diff, 1000), diff);
    }

    #[test]
    fn truncate_cuts_at_newline() {
        let diff = "line1\nline2\nline3\n";
        // max_chars = 10 → "line1\nline" → last \n at index 5
        let result = truncate_diff(diff, 10);
        assert_eq!(result, "line1\n");
    }

    #[test]
    fn truncate_no_newline_cuts_hard() {
        let diff = "abcdefghij";
        assert_eq!(truncate_diff(diff, 5), "abcde");
    }
}
