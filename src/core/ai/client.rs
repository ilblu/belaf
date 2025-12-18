use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;
use tracing::{debug, info, warn};

use super::credentials::{resolve_credential, ClaudeCredential};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: String,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
}

pub struct AnthropicClient {
    credential: ClaudeCredential,
    model: String,
}

impl AnthropicClient {
    pub async fn new() -> Result<Self> {
        let credential = resolve_credential()?;
        debug!(
            credential_type = credential.credential_type(),
            "initialized Anthropic client"
        );
        Ok(Self {
            credential,
            model: "claude-sonnet-4-5-20250929".to_string(),
        })
    }

    pub async fn complete(&self, system: &str, user: &str) -> Result<String> {
        match &self.credential {
            ClaudeCredential::ApiKey(_) => self.complete_via_api(system, user).await,
            ClaudeCredential::OAuthToken { .. } => self.complete_via_cli(system, user),
        }
    }

    async fn complete_via_api(&self, system: &str, user: &str) -> Result<String> {
        let api_key = match &self.credential {
            ClaudeCredential::ApiKey(key) => key,
            _ => anyhow::bail!("API key required for direct API calls"),
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("failed to create HTTP client")?;

        let request_body = MessagesRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            system: system.to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: user.to_string(),
            }],
        };

        debug!(model = %self.model, "sending request to Anthropic API");

        let response = client
            .post(API_URL)
            .header("Content-Type", "application/json")
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("x-api-key", api_key)
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to Anthropic API")?;

        let status = response.status();
        debug!(%status, "received response from Anthropic API");

        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            warn!(%status, error_body = %body, "Anthropic API request failed");
            anyhow::bail!("Anthropic API request failed ({}): {}", status, body);
        }

        let response: MessagesResponse = response
            .json()
            .await
            .context("Failed to parse Anthropic API response")?;

        response
            .content
            .first()
            .map(|block| block.text.clone())
            .ok_or_else(|| anyhow::anyhow!("Empty response from Anthropic API"))
    }

    fn complete_via_cli(&self, system: &str, user: &str) -> Result<String> {
        let claude_path = find_claude_binary().ok_or_else(|| {
            anyhow::anyhow!(
                "Claude CLI not found. Install with: npm install -g @anthropic-ai/claude-code"
            )
        })?;

        info!("using Claude CLI for OAuth-based completion");

        let prompt = if system.is_empty() {
            user.to_string()
        } else {
            format!("{}\n\n{}", system, user)
        };

        let mut env_vars = std::env::vars().collect::<std::collections::HashMap<_, _>>();
        env_vars.remove("ANTHROPIC_API_KEY");
        env_vars.insert("CLAUDE_CODE_ENTRYPOINT".to_string(), "belaf".to_string());

        let output = Command::new(&claude_path)
            .arg("--print")
            .arg(&prompt)
            .envs(env_vars)
            .output()
            .context("failed to execute Claude CLI")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            warn!(stderr = %stderr, stdout = %stdout, "Claude CLI failed");
            anyhow::bail!("Claude CLI failed: {}", stderr);
        }

        let response = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if response.is_empty() {
            anyhow::bail!("Empty response from Claude CLI");
        }

        Ok(response)
    }
}

fn find_claude_binary() -> Option<String> {
    let possible_paths = ["/usr/local/bin/claude", "/opt/homebrew/bin/claude"];

    for path in &possible_paths {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    if let Ok(output) = Command::new("which").arg("claude").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let npm_global = format!("{}/.npm-global/bin/claude", home);
        if std::path::Path::new(&npm_global).exists() {
            return Some(npm_global);
        }

        let nvm_path = format!("{}/.nvm/versions/node", home);
        if let Ok(entries) = std::fs::read_dir(&nvm_path) {
            for entry in entries.flatten() {
                let claude_bin = entry.path().join("bin/claude");
                if claude_bin.exists() {
                    return Some(claude_bin.to_string_lossy().to_string());
                }
            }
        }
    }

    None
}
