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

use crate::config::{AiConfig, AiProvider};

// ============================================================================
// Anthropic Messages API types
// ============================================================================

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<AnthropicMessage<'a>>,
}

#[derive(Serialize)]
struct AnthropicMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

// ============================================================================
// OpenAI Chat Completions API types (also used by compatible providers)
// ============================================================================

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<OpenAiMessage<'a>>,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
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
        .context("failed to resolve AI API key")?;

    if ai_cfg.model.trim().is_empty() {
        return Err(anyhow!(
            "ai.model is not set — specify a model name in gd.yml (e.g. claude-haiku-4-5-20251001 or gpt-4o-mini)"
        ));
    }

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
        provider = ?ai_cfg.provider,
        model = %ai_cfg.model,
        base_url = %ai_cfg.resolved_base_url(),
        diff_chars = diff_slice.len(),
        "requesting AI commit message"
    );

    let client = Client::new();
    let message = match ai_cfg.provider {
        AiProvider::Anthropic => {
            call_anthropic(&client, ai_cfg, &api_key, &prompt).await?
        }
        AiProvider::OpenAi => {
            call_openai(&client, ai_cfg, &api_key, &prompt).await?
        }
    };

    debug!(message = %message, "AI commit message generated");
    Ok(message)
}

// ── Anthropic ─────────────────────────────────────────────────────────────────

async fn call_anthropic(
    client: &Client,
    ai_cfg: &AiConfig,
    api_key: &str,
    prompt: &str,
) -> Result<String> {
    let url = format!("{}/v1/messages", ai_cfg.resolved_base_url());
    let request = AnthropicRequest {
        model: &ai_cfg.model,
        max_tokens: 256,
        messages: vec![AnthropicMessage {
            role: "user",
            content: prompt,
        }],
    };

    let response = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
        .context("HTTP request to Anthropic API failed")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Anthropic API returned {}: {}", status, body.trim()));
    }

    let parsed: AnthropicResponse = response
        .json()
        .await
        .context("failed to parse Anthropic API response")?;

    parsed
        .content
        .into_iter()
        .find(|b| b.kind == "text")
        .and_then(|b| b.text)
        .map(|t| t.trim().to_string())
        .ok_or_else(|| anyhow!("Anthropic API response contained no text block"))
}

// ── OpenAI-compatible ─────────────────────────────────────────────────────────

async fn call_openai(
    client: &Client,
    ai_cfg: &AiConfig,
    api_key: &str,
    prompt: &str,
) -> Result<String> {
    let url = format!("{}/v1/chat/completions", ai_cfg.resolved_base_url());
    let request = OpenAiRequest {
        model: &ai_cfg.model,
        max_tokens: 256,
        messages: vec![OpenAiMessage {
            role: "user",
            content: prompt,
        }],
    };

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
        .context("HTTP request to OpenAI API failed")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("OpenAI API returned {}: {}", status, body.trim()));
    }

    let parsed: OpenAiResponse = response
        .json()
        .await
        .context("failed to parse OpenAI API response")?;

    parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .map(|t| t.trim().to_string())
        .ok_or_else(|| anyhow!("OpenAI API response contained no choices"))
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
        // slice [..5] excludes the \n itself → "line1"
        let result = truncate_diff(diff, 10);
        assert_eq!(result, "line1");
    }

    #[test]
    fn truncate_no_newline_cuts_hard() {
        let diff = "abcdefghij";
        assert_eq!(truncate_diff(diff, 5), "abcde");
    }
}
