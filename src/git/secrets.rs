//! Secret scanning — detects credentials in diffs before push.

use once_cell::sync::Lazy;
use regex::Regex;

/// A detected secret in a diff line.
#[derive(Debug, Clone)]
pub struct SecretHit {
    /// File path (relative to repo root) where the hit was found.
    pub file: String,
    /// Line number (1-based, 0 = unknown).
    pub line: u32,
    /// Human-readable pattern name.
    pub pattern: String,
    /// The matched substring (may be truncated for display).
    pub matched_text: String,
}

struct SecretPattern {
    regex: Regex,
    name: &'static str,
}

static SECRET_PATTERNS: Lazy<Vec<SecretPattern>> = Lazy::new(|| {
    vec![
        SecretPattern {
            regex: Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
            name: "AWS Access Key ID",
        },
        SecretPattern {
            regex: Regex::new(r"(?i)aws_secret_access_key\s*[:=]\s*[A-Za-z0-9/+]{40}").unwrap(),
            name: "AWS Secret Access Key",
        },
        SecretPattern {
            regex: Regex::new(r"(?i)sk_live_[0-9a-zA-Z]{24}").unwrap(),
            name: "Stripe Live Secret Key",
        },
        SecretPattern {
            regex: Regex::new(r"ghp_[0-9a-zA-Z]{36}").unwrap(),
            name: "GitHub Personal Access Token",
        },
        SecretPattern {
            regex: Regex::new(r"ghs_[0-9a-zA-Z]{36}").unwrap(),
            name: "GitHub App Token",
        },
        SecretPattern {
            regex: Regex::new(r"-----BEGIN (RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----").unwrap(),
            name: "Private Key Header",
        },
        SecretPattern {
            regex: Regex::new(r"AIza[0-9A-Za-z\-_]{35}").unwrap(),
            name: "Google API Key",
        },
        SecretPattern {
            regex: Regex::new(r#"(?i)password\s*[:=]\s*["'][^"']{8,}["']"#).unwrap(),
            name: "Hardcoded Password",
        },
        SecretPattern {
            regex: Regex::new(r#"(?i)(api_key|api_secret|auth_token|access_token)\s*[:=]\s*["'][A-Za-z0-9\-_+/]{16,}["']"#).unwrap(),
            name: "Hardcoded API Key / Token",
        },
        SecretPattern {
            regex: Regex::new(r"xox[baprs]-[0-9A-Za-z\-]{10,}").unwrap(),
            name: "Slack Token",
        },
    ]
});

/// Scan a single diff line for secret patterns.
///
/// Returns the first hit found, or `None` if the line is clean.
/// The `file` and `line` fields are left as defaults; callers should fill them in.
pub fn scan_line(content: &str) -> Option<SecretHit> {
    for pattern in SECRET_PATTERNS.iter() {
        if let Some(m) = pattern.regex.find(content) {
            // Truncate matched text to 60 chars for safe display
            let matched = m.as_str();
            let display = if matched.len() > 60 {
                format!("{}…", &matched[..57])
            } else {
                matched.to_string()
            };
            return Some(SecretHit {
                file: String::new(),
                line: 0,
                pattern: pattern.name.to_string(),
                matched_text: display,
            });
        }
    }
    None
}

/// Scan a full diff string, returning all hits across all lines.
pub fn scan_diff(diff_text: &str) -> Vec<SecretHit> {
    let mut hits = Vec::new();
    for (i, line) in diff_text.lines().enumerate() {
        // Only scan added lines in a unified diff
        if line.starts_with('+') && !line.starts_with("+++") {
            if let Some(mut hit) = scan_line(&line[1..]) {
                hit.line = (i + 1) as u32;
                hits.push(hit);
            }
        }
    }
    hits
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aws_key_detected() {
        let line = "let key = \"AKIAIOSFODNN7EXAMPLE\";";
        assert!(scan_line(line).is_some());
        assert_eq!(scan_line(line).unwrap().pattern, "AWS Access Key ID");
    }

    #[test]
    fn test_github_token_detected() {
        let token = "ghp_".to_string() + &"A".repeat(36);
        assert!(scan_line(&token).is_some());
    }

    #[test]
    fn test_private_key_header_detected() {
        let line = "-----BEGIN RSA PRIVATE KEY-----";
        assert!(scan_line(line).is_some());
    }

    #[test]
    fn test_openssh_private_key_detected() {
        let line = "-----BEGIN OPENSSH PRIVATE KEY-----";
        assert!(scan_line(line).is_some());
    }

    #[test]
    fn test_clean_line_passes() {
        let line = "let x = 42;";
        assert!(scan_line(line).is_none());
    }

    #[test]
    fn test_hardcoded_password() {
        let line = r#"password = "supersecret123""#;
        assert!(scan_line(line).is_some());
    }

    #[test]
    fn test_short_password_not_flagged() {
        // Fewer than 8 chars in the value — not flagged (avoids "password = 'abc'" style false positives)
        let line = r#"password = "abc""#;
        assert!(scan_line(line).is_none());
    }

    #[test]
    fn test_scan_diff_only_added_lines() {
        let diff = "-removed line with AKIAIOSFODNN7EXAMPLE\n+added clean line\n";
        // The removed line has a key but scan_diff only checks '+' lines
        let hits = scan_diff(diff);
        assert!(hits.is_empty(), "should not flag removed lines");
    }

    #[test]
    fn test_scan_diff_added_line_flagged() {
        let diff = format!("+let key = \"AKIAIOSFODNN7EXAMPLE\";\n");
        let hits = scan_diff(&diff);
        assert!(!hits.is_empty());
    }
}
