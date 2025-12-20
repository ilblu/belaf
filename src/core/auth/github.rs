use crate::error::{CliError, Result};
use octocrab::Octocrab;
use reqwest::header::ACCEPT;
use secrecy::{ExposeSecret, SecretString};
use tracing::debug;

pub const REQUIRED_SCOPES: &[&str] = &["repo"];
pub const REQUESTED_SCOPES: &[&str] = &["repo", "read:org", "user:email", "read:packages"];

pub struct DeviceFlowCodes {
    pub user_code: String,
    pub verification_uri: String,
    device_codes: octocrab::auth::DeviceCodes,
    client: Octocrab,
    client_id: SecretString,
}

impl DeviceFlowCodes {
    pub async fn poll_for_token(self) -> Result<String> {
        let auth = self
            .device_codes
            .poll_until_available(&self.client, &self.client_id)
            .await
            .map_err(|e| CliError::GitHubApi(format!("Authorization failed: {}", e)))?;

        let token = auth.access_token.expose_secret().to_string();
        debug!("Token received via OAuth Device Flow");
        Ok(token)
    }
}

pub async fn request_device_code(client_id: &str) -> Result<DeviceFlowCodes> {
    let secret_client_id = SecretString::from(client_id.to_string());

    let client = Octocrab::builder()
        .base_uri("https://github.com")
        .map_err(|e| CliError::GitHubApi(format!("Failed to configure GitHub client: {}", e)))?
        .add_header(ACCEPT, "application/json".to_string())
        .build()
        .map_err(|e| CliError::GitHubApi(format!("Failed to build GitHub client: {}", e)))?;

    let device_codes = client
        .authenticate_as_device(&secret_client_id, REQUESTED_SCOPES.to_vec())
        .await
        .map_err(|e| CliError::GitHubApi(format!("Failed to request device code: {}", e)))?;

    Ok(DeviceFlowCodes {
        user_code: device_codes.user_code.clone(),
        verification_uri: device_codes.verification_uri.clone(),
        device_codes,
        client,
        client_id: secret_client_id,
    })
}

pub async fn get_username(token: &str) -> Result<String> {
    let client = Octocrab::builder()
        .personal_token(token.to_string())
        .build()
        .map_err(|e| CliError::GitHubApi(format!("Failed to build GitHub client: {}", e)))?;

    let user = client
        .current()
        .user()
        .await
        .map_err(|e| CliError::GitHubApi(format!("Failed to get user info: {}", e)))?;

    Ok(user.login)
}

pub async fn validate_token_scopes(token: &str, required_scopes: &[&str]) -> Result<()> {
    let client = reqwest::Client::new();

    let response = client
        .get(GITHUB_API_USER_URL)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "belaf")
        .send()
        .await
        .map_err(|e| CliError::GitHubApi(format!("Failed to validate token: {}", e)))?;

    validate_response_and_scopes(response.status(), response.headers(), required_scopes)
}

pub fn validate_token_scopes_blocking(token: &str, required_scopes: &[&str]) -> Result<()> {
    let client = reqwest::blocking::Client::new();

    let response = client
        .get(GITHUB_API_USER_URL)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "belaf")
        .send()
        .map_err(|e| CliError::GitHubApi(format!("Failed to validate token: {}", e)))?;

    validate_response_and_scopes(response.status(), response.headers(), required_scopes)
}

const GITHUB_API_USER_URL: &str = "https://api.github.com/user";
const GITHUB_REVOKE_URL: &str = "https://github.com/settings/connections/applications/Ov23liuSrRXBZ7PDX61o";

pub async fn get_token_scopes(token: &str) -> Result<Vec<String>> {
    let client = reqwest::Client::new();

    let response = client
        .get(GITHUB_API_USER_URL)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "belaf")
        .send()
        .await
        .map_err(|e| CliError::GitHubApi(format!("Failed to check token scopes: {}", e)))?;

    if !response.status().is_success() {
        return Err(CliError::GitHubApi(format!(
            "Token validation failed: HTTP {}",
            response.status()
        )));
    }

    let scopes = response
        .headers()
        .get("x-oauth-scopes")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    Ok(scopes
        .split(", ")
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_string())
        .collect())
}

pub fn get_revoke_url() -> &'static str {
    GITHUB_REVOKE_URL
}

fn validate_response_and_scopes(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    required_scopes: &[&str],
) -> Result<()> {
    if !status.is_success() {
        return Err(CliError::GitHubApi(format!(
            "Token validation failed: HTTP {}",
            status
        )));
    }

    let scopes = headers
        .get("x-oauth-scopes")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    check_required_scopes(scopes, required_scopes)?;

    debug!("Token scopes validated: {}", scopes);
    Ok(())
}

fn check_required_scopes(scopes: &str, required_scopes: &[&str]) -> Result<()> {
    let token_scopes: Vec<&str> = scopes.split(", ").map(|s| s.trim()).collect();

    let missing: Vec<&str> = required_scopes
        .iter()
        .filter(|&required| !token_scopes.iter().any(|s| s == required))
        .copied()
        .collect();

    if !missing.is_empty() {
        return Err(CliError::GitHubApi(format!(
            "Token missing required scopes: {}. Please re-authenticate with 'belaf auth login'",
            missing.join(", ")
        )));
    }

    Ok(())
}
