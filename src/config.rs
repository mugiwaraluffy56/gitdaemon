use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub repo: RepoConfig,
    #[serde(default)]
    pub commit: CommitConfig,
    #[serde(default)]
    pub push: PushConfig,
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub safety: SafetyConfig,
    #[serde(default)]
    pub hooks: HooksConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    #[serde(default = "default_true")]
    pub auto_stage: bool,
    #[serde(default = "default_true")]
    pub auto_fetch: bool,
    #[serde(default = "default_fetch_interval")]
    pub fetch_interval: u64,
    /// Fast-forward `base_branch` after every fetch and optionally rebase the
    /// current working branch onto it.
    #[serde(default = "default_true")]
    pub auto_sync_base: bool,
    /// The long-lived branch to keep updated (typically `main` or `master`).
    #[serde(default = "default_base_branch")]
    pub base_branch: String,
    /// When `true`, rebase the current branch onto `base_branch` after it
    /// advances — only when the working tree is clean.
    #[serde(default = "default_true")]
    pub rebase_on_sync: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitStrategy {
    Time,
    ChangeCount,
}

impl Default for CommitStrategy {
    fn default() -> Self {
        CommitStrategy::Time
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitConfig {
    #[serde(default)]
    pub strategy: CommitStrategy,
    #[serde(default = "default_commit_interval")]
    pub interval: u64,
    #[serde(default = "default_commit_message")]
    pub message: String,
    #[serde(default = "default_change_threshold")]
    pub change_threshold: usize,
    /// When `true`, files from different top-level directories are committed
    /// separately (e.g. one commit for `src/git/` changes, one for `src/daemon/`).
    #[serde(default)]
    pub group_by_directory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PushStrategy {
    #[serde(rename = "batch")]
    Batch,
}

impl Default for PushStrategy {
    fn default() -> Self {
        PushStrategy::Batch
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushConfig {
    #[serde(default)]
    pub strategy: PushStrategy,
    #[serde(default = "default_push_interval")]
    pub interval: u64,
    #[serde(default = "default_branch")]
    pub branch: String,
    /// Branches that gd must never auto-push to. Pushing to any of these
    /// requires an explicit `gd push` with the daemon paused.
    #[serde(default = "default_protected_branches")]
    pub protected_branches: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyConfig {
    #[serde(default)]
    pub confirm_first: bool,
    #[serde(default = "default_true")]
    pub block_secrets: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HooksConfig {
    #[serde(default)]
    pub pre_commit: String,
    #[serde(default)]
    pub post_commit: String,
    /// Shell command run after a successful push (non-zero exit is logged, not fatal).
    /// Environment: `$FG_BRANCH`, `$FG_COMMITS` (count pushed).
    #[serde(default)]
    pub on_push_success: String,
    /// Shell command run when a conflict or rebase failure is detected.
    /// Environment: `$FG_BRANCH`, `$FG_ERROR`.
    #[serde(default)]
    pub on_conflict: String,
}

// ============================================================================
// Defaults
// ============================================================================

const fn default_true() -> bool {
    true
}

const fn default_commit_interval() -> u64 {
    120
}

const fn default_change_threshold() -> usize {
    10
}

const fn default_push_interval() -> u64 {
    300
}

const fn default_fetch_interval() -> u64 {
    60
}

fn default_commit_message() -> String {
    "{summary}".to_string()
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_base_branch() -> String {
    "main".to_string()
}

fn default_protected_branches() -> Vec<String> {
    vec!["main".to_string(), "master".to_string(), "develop".to_string()]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            repo: RepoConfig::default(),
            commit: CommitConfig::default(),
            push: PushConfig::default(),
            ignore: Vec::new(),
            safety: SafetyConfig::default(),
            hooks: HooksConfig::default(),
        }
    }
}

impl Default for RepoConfig {
    fn default() -> Self {
        Self {
            auto_stage: true,
            auto_fetch: true,
            fetch_interval: default_fetch_interval(),
            auto_sync_base: true,
            base_branch: default_base_branch(),
            rebase_on_sync: true,
        }
    }
}

impl Default for CommitConfig {
    fn default() -> Self {
        Self {
            strategy: CommitStrategy::Time,
            interval: default_commit_interval(),
            message: default_commit_message(),
            change_threshold: default_change_threshold(),
            group_by_directory: false,
        }
    }
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            strategy: PushStrategy::Batch,
            interval: default_push_interval(),
            branch: default_branch(),
            protected_branches: default_protected_branches(),
        }
    }
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            confirm_first: false,
            block_secrets: true,
        }
    }
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            pre_commit: String::new(),
            post_commit: String::new(),
            on_push_success: String::new(),
            on_conflict: String::new(),
        }
    }
}

// ============================================================================
// CommitConfig helpers
// ============================================================================

impl CommitConfig {
    /// Format a commit message using this config's template.
    /// Tokens: {summary}, {count}, {branch}, {timestamp}
    pub fn format_message(&self, summary: &str, changed_count: usize, branch: &str) -> String {
        let trimmed = summary.trim();
        let truncated = if trimmed.len() > 72 { &trimmed[..72] } else { trimmed };

        let timestamp = chrono::Utc::now().to_rfc3339();

        let mut tokens: HashMap<&str, String> = HashMap::new();
        tokens.insert("summary", truncated.to_string());
        tokens.insert("count", changed_count.to_string());
        tokens.insert("branch", branch.to_string());
        tokens.insert("timestamp", timestamp);

        let mut result = self.message.clone();
        for (key, value) in &tokens {
            result = result.replace(&format!("{{{}}}", key), value);
        }
        result
    }
}

// ============================================================================
// Config public API
// ============================================================================

impl Config {
    /// Load Config from a YAML file at the given path.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config from {}", path.display()))?;
        Self::load_from_str(&contents)
    }

    /// Load Config from gd.yml in the given repository root.
    pub fn load_from_repo(repo_root: &Path) -> Result<Self> {
        Self::load(&repo_root.join("gd.yml"))
    }

    /// Parse and validate from a YAML string.
    pub fn load_from_str(yaml: &str) -> Result<Self> {
        let config: Config = serde_yaml::from_str(yaml)
            .context("failed to parse YAML config")?;
        config.validate()?;
        Ok(config)
    }

    /// Validate the configuration values.
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(anyhow!("invalid version: {} (must be 1)", self.version));
        }
        if self.commit.interval == 0 {
            return Err(anyhow!("commit.interval must be > 0"));
        }
        if self.push.interval == 0 {
            return Err(anyhow!("push.interval must be > 0"));
        }
        if self.commit.change_threshold == 0 {
            return Err(anyhow!("commit.change_threshold must be > 0"));
        }
        if self.push.branch.trim().is_empty() {
            return Err(anyhow!("push.branch cannot be empty"));
        }
        if self.commit.message.trim().is_empty() {
            return Err(anyhow!("commit.message cannot be empty"));
        }
        Ok(())
    }

    /// Generate a canonical default gd.yml with comments.
    pub fn generate_default() -> String {
        r#"# gitdaemon configuration — https://github.com/mugiwaraluffy56/gitdaemon
version: 1

repo:
  # Automatically stage working-tree changes before committing
  auto_stage: true
  # Fetch from remotes in the background
  auto_fetch: true
  # Seconds between background fetches
  fetch_interval: 60
  # Fast-forward base_branch to origin/<base_branch> after every fetch,
  # then rebase the current working branch onto it (if the tree is clean).
  auto_sync_base: true
  # The long-lived branch to keep updated (typically main or master)
  base_branch: main
  # Rebase the current branch onto base_branch after it advances
  rebase_on_sync: true

commit:
  # Strategy: "time" (every N seconds) or "change_count" (every N files)
  strategy: time
  # Seconds between auto-commits (time strategy)
  interval: 120
  # Files accumulated before committing (change_count strategy)
  change_threshold: 10
  # Commit message template. {summary} is a full conventional commit line
  # generated automatically (e.g. "feat(git): add push queue").
  # Other tokens: {count}, {branch}, {timestamp}
  message: "{summary}"

push:
  # Push strategy ("batch" = queue commits and push together)
  strategy: batch
  # Seconds between auto-pushes
  interval: 300
  # Branch to push
  branch: main
  # Branches that gd must never auto-push to (use `gd push` to push manually)
  protected_branches:
    - main
    - master
    - develop

# Patterns to exclude from staging (gitignore-style)
ignore:
  - "*.log"
  - "node_modules/"
  - ".env"

safety:
  # Prompt before the first push in a session
  confirm_first: false
  # Scan diffs for secrets before pushing
  block_secrets: true

hooks:
  # Shell command run before each commit (non-zero exit aborts commit)
  pre_commit: ""
  # Shell command run after each commit (non-zero exit is logged, not fatal)
  post_commit: ""
  # Shell command run after a successful push ($FG_BRANCH, $FG_COMMITS available)
  on_push_success: ""
  # Shell command run when a conflict is detected ($FG_BRANCH, $FG_ERROR available)
  on_conflict: ""
"#
        .to_string()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_defaults() {
        let config = Config::default();
        assert_eq!(config.version, 1);
        assert!(config.repo.auto_stage);
        assert!(config.repo.auto_fetch);
        assert_eq!(config.repo.fetch_interval, 60);
        assert!(config.repo.auto_sync_base);
        assert_eq!(config.repo.base_branch, "main");
        assert!(config.repo.rebase_on_sync);
        assert!(matches!(config.commit.strategy, CommitStrategy::Time));
        assert_eq!(config.commit.interval, 120);
        assert_eq!(config.commit.change_threshold, 10);
        assert_eq!(config.commit.message, "{summary}");
        assert!(matches!(config.push.strategy, PushStrategy::Batch));
        assert_eq!(config.push.interval, 300);
        assert_eq!(config.push.branch, "main");
        assert!(config.safety.block_secrets);
        assert!(!config.safety.confirm_first);
        assert!(config.hooks.pre_commit.is_empty());
        assert!(config.hooks.post_commit.is_empty());
    }

    #[test]
    fn test_version_wrong() {
        let yaml = "version: 2\n";
        assert!(Config::load_from_str(yaml).is_err());
    }

    #[test]
    fn test_zero_commit_interval() {
        let yaml = "version: 1\ncommit:\n  interval: 0\n";
        assert!(Config::load_from_str(yaml).is_err());
    }

    #[test]
    fn test_load_nonexistent() {
        assert!(Config::load(Path::new("/nonexistent/path/gd.yml")).is_err());
    }

    #[test]
    fn test_format_message_all_tokens() {
        let cfg = CommitConfig {
            message: "feat: {summary} ({count} files) [{branch}] {timestamp}".to_string(),
            ..Default::default()
        };
        let msg = cfg.format_message("my feature", 5, "feature-branch");
        assert!(msg.contains("feat: my feature"));
        assert!(msg.contains("5 files"));
        assert!(msg.contains("feature-branch"));
        assert!(msg.contains('T')); // ISO-8601 contains T
    }

    #[test]
    fn test_format_message_unknown_token_preserved() {
        let cfg = CommitConfig {
            message: "commit: {summary} {unknown_token}".to_string(),
            ..Default::default()
        };
        let msg = cfg.format_message("fix bug", 1, "main");
        assert!(msg.contains("commit: fix bug {unknown_token}"));
    }

    #[test]
    fn test_generate_default_parses() {
        let yaml = Config::generate_default();
        let config = Config::load_from_str(&yaml).expect("generated YAML should parse");
        assert_eq!(config.version, 1);
    }
}
