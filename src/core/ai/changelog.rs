use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::client::AnthropicClient;

const SYSTEM_PROMPT: &str = r###"You are an expert technical writer specializing in software changelogs.
Your task is to improve and polish an existing changelog entry while keeping the Keep a Changelog format.

You will receive:
1. A draft changelog entry (already categorized by conventional commits)
2. The raw commit messages for additional context

Your job:
- Improve the wording to be clear and user-friendly
- Ensure entries start with a verb (Add, Fix, Change, Remove, etc.)
- Group related changes if they were split across commits
- Add brief context where helpful (but stay concise)
- Keep the same categories (Added, Changed, Fixed, etc.)
- Maintain the Keep a Changelog format exactly

Output ONLY the improved changelog entry in markdown format.
Start with the version header (like ## [1.0.0] - 2025-01-01) exactly as provided.
Do NOT add any explanation or commentary outside the changelog content.
Do NOT wrap in code blocks."###;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AiChangelogOutput {
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub changed: Vec<String>,
    #[serde(default)]
    pub deprecated: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
    #[serde(default)]
    pub fixed: Vec<String>,
    #[serde(default)]
    pub security: Vec<String>,
}

impl AiChangelogOutput {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.changed.is_empty()
            && self.deprecated.is_empty()
            && self.removed.is_empty()
            && self.fixed.is_empty()
            && self.security.is_empty()
    }
}

pub struct AiChangelogGenerator {
    client: AnthropicClient,
}

impl AiChangelogGenerator {
    pub async fn new() -> Result<Self> {
        let client = AnthropicClient::new().await?;
        Ok(Self { client })
    }

    pub async fn polish(&self, draft_changelog: &str, raw_commits: &[String]) -> Result<String> {
        if draft_changelog.trim().is_empty() {
            return Ok(draft_changelog.to_string());
        }

        let commits_text = raw_commits.join("\n");
        let user_prompt = format!(
            "Please improve this changelog entry:\n\n---\n{}\n---\n\nRaw commits for context:\n{}",
            draft_changelog, commits_text
        );

        let response = self
            .client
            .complete(SYSTEM_PROMPT, &user_prompt)
            .await
            .context("Failed to polish changelog with AI")?;

        let cleaned = response.trim();
        if cleaned.starts_with("```") {
            Ok(cleaned
                .trim_start_matches("```markdown")
                .trim_start_matches("```md")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim()
                .to_string())
        } else {
            Ok(cleaned.to_string())
        }
    }
}
