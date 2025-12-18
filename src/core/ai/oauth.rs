use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use serde::Serialize;
use sha2::{Digest, Sha256};

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTH_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const SCOPES: &str = "org:create_api_key user:profile user:inference";

#[derive(Debug, serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub token_type: String,
}

#[derive(Serialize)]
struct TokenRequest {
    code: String,
    state: String,
    grant_type: String,
    client_id: String,
    redirect_uri: String,
    code_verifier: String,
}

pub struct OAuthFlow {
    code_verifier: String,
    state: String,
}

impl OAuthFlow {
    pub fn new() -> Self {
        Self {
            code_verifier: generate_random_string(64),
            state: generate_random_string(32),
        }
    }

    pub fn authorization_url(&self) -> String {
        let code_challenge = generate_code_challenge(&self.code_verifier);

        format!(
            "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
            AUTH_URL,
            CLIENT_ID,
            urlencoding::encode(REDIRECT_URI),
            urlencoding::encode(SCOPES),
            code_challenge,
            &self.state
        )
    }

    pub fn state(&self) -> &str {
        &self.state
    }

    pub async fn exchange_code(&self, code: &str) -> Result<TokenResponse> {
        let clean_code = code.trim();

        let (pure_code, state) = match clean_code.split_once('#') {
            Some((c, s)) => (c.to_string(), s.to_string()),
            None => (clean_code.to_string(), String::new()),
        };

        let request_body = TokenRequest {
            code: pure_code,
            state,
            grant_type: "authorization_code".to_string(),
            client_id: CLIENT_ID.to_string(),
            redirect_uri: REDIRECT_URI.to_string(),
            code_verifier: self.code_verifier.clone(),
        };

        let client = reqwest::Client::new();

        let response = client
            .post(TOKEN_URL)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("User-Agent", "anthropic")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send token exchange request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Token exchange failed ({}): {}", status, body);
        }

        response
            .json()
            .await
            .context("Failed to parse token response")
    }
}

impl Default for OAuthFlow {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize)]
struct RefreshTokenRequest {
    grant_type: String,
    refresh_token: String,
    client_id: String,
}

pub async fn refresh_token(refresh_token: &str) -> Result<TokenResponse> {
    let request_body = RefreshTokenRequest {
        grant_type: "refresh_token".to_string(),
        refresh_token: refresh_token.to_string(),
        client_id: CLIENT_ID.to_string(),
    };

    let client = reqwest::Client::new();

    let response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("User-Agent", "anthropic")
        .json(&request_body)
        .send()
        .await
        .context("Failed to send token refresh request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        anyhow::bail!("Token refresh failed ({}): {}", status, body);
    }

    response
        .json()
        .await
        .context("Failed to parse refresh token response")
}

fn generate_random_string(length: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::rng();

    (0..length)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_string_length() {
        let s = generate_random_string(64);
        assert_eq!(s.len(), 64);
    }

    #[test]
    fn test_code_challenge_format() {
        let verifier = "test_verifier_string";
        let challenge = generate_code_challenge(verifier);
        assert!(!challenge.is_empty());
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
        assert!(!challenge.contains('='));
    }

    #[test]
    fn test_authorization_url_contains_required_params() {
        let flow = OAuthFlow::new();
        let url = flow.authorization_url();

        assert!(url.contains("client_id="));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("scope="));
        assert!(url.contains("code_challenge="));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state="));
    }
}
