use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub github: GitHubConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubConfig {
    #[serde(default = "default_github_oauth_client_id")]
    pub oauth_client_id: String,
}

fn default_github_oauth_client_id() -> String {
    "Ov23liNPpcjTMYaP841Y".to_string()
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            oauth_client_id: default_github_oauth_client_id(),
        }
    }
}
