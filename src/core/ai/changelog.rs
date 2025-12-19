use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::client::AnthropicClient;

const SYSTEM_PROMPT: &str = r###"You are an expert technical writer specializing in software changelogs.
Your task is to improve and polish an existing changelog entry using the Keep a Changelog format.

You will receive:
1. A draft changelog entry with version header and categorized changes
2. The raw commit messages for additional context

Your job:
- PRESERVE the exact version header format: ## [X.Y.Z] - YYYY-MM-DD
- PRESERVE the category headers: ### Added, ### Changed, ### Fixed, etc.
- Improve each entry to be clear, concise, and user-focused
- Each entry MUST start with a verb: Add, Fix, Update, Remove, Improve, etc.
- Use backticks for code elements: functions, variables, file names, commands
- Use **bold** for important scopes or modules
- Group related changes from multiple commits into single, coherent entries
- Remove redundant or duplicate information
- Focus on WHAT changed and WHY it matters to users
- Keep entries concise (one line each when possible)

Format rules:
- Version header: ## [X.Y.Z] - YYYY-MM-DD (KEEP EXACTLY AS PROVIDED)
- Category headers: ### Added, ### Changed, ### Fixed, etc.
- List items: - Description starting with verb

Output ONLY the improved changelog entry in markdown.
Do NOT add explanations, code blocks, or any text outside the changelog.
Do NOT change the version number or date."###;

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
