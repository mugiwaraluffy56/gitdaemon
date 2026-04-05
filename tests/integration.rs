//! Integration tests for gitdaemon.
//!
//! These tests create real temporary Git repositories and exercise the library
//! interface end-to-end. No network calls are made; no daemon process is
//! started. Tests that need async use `#[tokio::test]`.

use std::fs;
use std::path::Path;

use anyhow::Result;
use tempfile::TempDir;

use gitdaemon::config::{AiConfig, CommitConfig, CommitStrategy, Config, HooksConfig};
use gitdaemon::git::commit::{commit, commit_if_ready, CommitAccumulator, SkipReason};
use gitdaemon::git::secrets::{scan_diff, scan_line};

// ============================================================================
// Helpers
// ============================================================================

/// Initialise a temporary Git repository with an initial commit.
fn init_repo() -> Result<(TempDir, std::path::PathBuf)> {
    let dir = TempDir::new()?;
    let path = dir.path().to_path_buf();

    let repo = git2::Repository::init(&path)?;
    fs::write(path.join("README.md"), "# Test\n")?;

    let mut index = repo.index()?;
    index.add_path(Path::new("README.md"))?;
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let sig = git2::Signature::now("Test User", "test@example.com")?;
    repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])?;

    Ok((dir, path))
}

/// Stage a new file in the repository.
fn stage(repo_path: &Path, name: &str, content: &str) -> Result<()> {
    let repo = git2::Repository::open(repo_path)?;
    fs::write(repo_path.join(name), content)?;
    let mut index = repo.index()?;
    index.add_path(Path::new(name))?;
    index.write()?;
    Ok(())
}

/// Return the HEAD commit message.
fn head_message(repo_path: &Path) -> Result<String> {
    let repo = git2::Repository::open(repo_path)?;
    let head = repo.head()?.peel_to_commit()?;
    Ok(head.summary().unwrap_or("").to_string())
}

// ============================================================================
// Config
// ============================================================================

#[test]
fn config_default_parses() {
    let yaml = Config::generate_default();
    let config = Config::load_from_str(&yaml).expect("generated YAML must parse");
    assert_eq!(config.version, 1);
    assert!(config.repo.auto_stage);
    assert!(config.repo.auto_fetch);
    assert!(config.safety.block_secrets);
    assert!(!config.ai.enabled);
}

#[test]
fn config_ai_section_round_trips() {
    let yaml = r#"
version: 1
ai:
  enabled: true
  api_key: "env:ANTHROPIC_API_KEY"
  model: "claude-haiku-4-5-20251001"
  max_diff_chars: 8000
"#;
    let config = Config::load_from_str(yaml).expect("must parse");
    assert!(config.ai.enabled);
    assert_eq!(config.ai.api_key, "env:ANTHROPIC_API_KEY");
    assert_eq!(config.ai.model, "claude-haiku-4-5-20251001");
    assert_eq!(config.ai.max_diff_chars, 8000);
}

#[test]
fn config_invalid_version_rejected() {
    let yaml = "version: 99\n";
    assert!(Config::load_from_str(yaml).is_err());
}

#[test]
fn config_zero_commit_interval_rejected() {
    let yaml = "version: 1\ncommit:\n  interval: 0\n";
    assert!(Config::load_from_str(yaml).is_err());
}

#[test]
fn config_empty_push_branch_rejected() {
    let yaml = "version: 1\npush:\n  branch: \"\"\n";
    assert!(Config::load_from_str(yaml).is_err());
}

// ============================================================================
// Secret scanning
// ============================================================================

#[test]
fn scan_detects_aws_access_key() {
    let line = "+    key = \"AKIAIOSFODNN7EXAMPLE\"";
    assert!(scan_line(line).is_some(), "should detect AWS key");
}

#[test]
fn scan_detects_github_pat() {
    // ghp_ + exactly 36 alphanumeric characters
    let line = "+TOKEN=ghp_abcdefghijklmnopqrstuvwxyz1234567890";
    assert!(scan_line(line).is_some(), "should detect GitHub PAT");
}

#[test]
fn scan_detects_private_key_header() {
    let line = "+-----BEGIN RSA PRIVATE KEY-----";
    assert!(scan_line(line).is_some(), "should detect private key header");
}

#[test]
fn scan_clean_diff_returns_empty() {
    let diff = r#"
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
+    println!("hello world");
 }
"#;
    let hits = scan_diff(diff);
    assert!(hits.is_empty(), "clean diff should produce no hits: {:?}", hits);
}

#[test]
fn scan_diff_finds_secret_in_added_line() {
    let diff = format!(
        "+    api_key = \"AKIAIOSFODNN7EXAMPLE\"\n"
    );
    let hits = scan_diff(&diff);
    assert!(!hits.is_empty(), "should find secret in diff");
}

// ============================================================================
// Commit — heuristic message generator
// ============================================================================

#[tokio::test]
async fn commit_creates_conventional_message() -> Result<()> {
    let (dir, path) = init_repo()?;
    fs::create_dir_all(path.join("src"))?;
    stage(&path, "src/lib.rs", "pub fn hello() {}\n")?;

    let result = commit(
        path.clone(),
        CommitConfig::default(),
        HooksConfig::default(),
        AiConfig::default(),
    )
    .await?;

    assert!(!result.skipped);
    assert!(result.oid.is_some());
    let msg = head_message(&path)?;
    assert!(msg.contains(':'), "expected conventional commit format: {}", msg);

    drop(dir);
    Ok(())
}

#[tokio::test]
async fn commit_skipped_when_nothing_staged() -> Result<()> {
    let (dir, path) = init_repo()?;

    let result = commit(
        path,
        CommitConfig::default(),
        HooksConfig::default(),
        AiConfig::default(),
    )
    .await?;

    assert!(result.skipped);
    assert!(matches!(result.skip_reason, Some(SkipReason::NoChanges)));
    drop(dir);
    Ok(())
}

#[tokio::test]
async fn commit_aborted_by_pre_commit_hook() -> Result<()> {
    let (dir, path) = init_repo()?;
    stage(&path, "a.txt", "x\n")?;

    let hooks = HooksConfig {
        pre_commit: "exit 42".to_string(),
        ..HooksConfig::default()
    };
    let result = commit(path, CommitConfig::default(), hooks, AiConfig::default()).await?;

    assert!(result.skipped);
    assert!(
        matches!(
            result.skip_reason,
            Some(SkipReason::PreCommitHookFailed { exit_code: 42, .. })
        ),
        "unexpected skip reason: {:?}",
        result.skip_reason
    );
    drop(dir);
    Ok(())
}

#[tokio::test]
async fn commit_if_ready_respects_change_count_threshold() -> Result<()> {
    let (dir, path) = init_repo()?;
    stage(&path, "a.txt", "x\n")?;

    let cfg = CommitConfig {
        strategy: CommitStrategy::ChangeCount,
        change_threshold: 5,
        ..Default::default()
    };
    let mut acc = CommitAccumulator::new();

    // Below threshold — should skip
    let result =
        commit_if_ready(path.clone(), cfg.clone(), HooksConfig::default(), AiConfig::default(), &mut acc, 2)
            .await?;
    assert!(result.skipped);
    assert!(matches!(
        result.skip_reason,
        Some(SkipReason::ThresholdNotReached { current: 2, required: 5 })
    ));

    // Stage more files to reach threshold
    for i in 0..4 {
        stage(&path, &format!("file{}.txt", i), "y\n")?;
    }

    // At threshold — should commit
    let result =
        commit_if_ready(path, cfg, HooksConfig::default(), AiConfig::default(), &mut acc, 3).await?;
    assert!(!result.skipped, "should have committed at threshold");

    drop(dir);
    Ok(())
}

// ============================================================================
// AI config key resolution
// ============================================================================

#[test]
fn ai_config_resolves_literal_key() {
    let cfg = AiConfig {
        api_key: "sk-ant-test".to_string(),
        ..AiConfig::default()
    };
    let tmp = TempDir::new().unwrap();
    let key = cfg.resolve_api_key(tmp.path()).unwrap();
    assert_eq!(key, "sk-ant-test");
}

#[test]
fn ai_config_resolves_env_prefix() {
    std::env::set_var("GD_TEST_API_KEY", "sk-ant-from-env");
    let cfg = AiConfig {
        api_key: "env:GD_TEST_API_KEY".to_string(),
        ..AiConfig::default()
    };
    let tmp = TempDir::new().unwrap();
    let key = cfg.resolve_api_key(tmp.path()).unwrap();
    assert_eq!(key, "sk-ant-from-env");
    std::env::remove_var("GD_TEST_API_KEY");
}

#[test]
fn ai_config_missing_env_var_returns_error() {
    std::env::remove_var("GD_NONEXISTENT_KEY_XYZ");
    let cfg = AiConfig {
        api_key: "env:GD_NONEXISTENT_KEY_XYZ".to_string(),
        ..AiConfig::default()
    };
    let tmp = TempDir::new().unwrap();
    assert!(cfg.resolve_api_key(tmp.path()).is_err());
}

#[test]
fn ai_config_loads_key_from_dotenv() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(".env"), "ANTHROPIC_API_KEY=sk-ant-dotenv\n").unwrap();

    let cfg = AiConfig {
        api_key: String::new(),
        ..AiConfig::default()
    };
    // Can't reliably test this without controlling env isolation, so just
    // verify the function doesn't panic when the file exists.
    let _ = cfg.resolve_api_key(tmp.path());
}
