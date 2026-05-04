use crate::core::api::error::ApiError;

/// Returns `true` when GitHub Actions OIDC is available in the current
/// environment. Both env vars are set automatically by the runner when
/// the workflow has `permissions: id-token: write` in scope.
pub fn is_actions_oidc_available() -> bool {
    std::env::var("ACTIONS_ID_TOKEN_REQUEST_URL").is_ok()
        && std::env::var("ACTIONS_ID_TOKEN_REQUEST_TOKEN").is_ok()
}

/// Fetches a GitHub Actions OIDC JWT from the given runner endpoint.
///
/// `request_url` and `request_bearer` come from the runner-injected env
/// vars `ACTIONS_ID_TOKEN_REQUEST_URL` / `ACTIONS_ID_TOKEN_REQUEST_TOKEN`.
/// `audience` should be the public URL of the belaf API (e.g.
/// `https://api.belaf.dev`); the runner will mint a JWT bound to that
/// audience.
pub async fn fetch_actions_oidc_jwt_with(
    client: &reqwest::Client,
    request_url: &str,
    request_bearer: &str,
    audience: &str,
) -> Result<String, ApiError> {
    #[derive(serde::Deserialize)]
    struct Resp {
        value: String,
    }

    let response = client
        .get(request_url)
        .query(&[("audience", audience)])
        .bearer_auth(request_bearer)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(ApiError::ApiResponse {
            status: response.status().as_u16(),
            message: response.text().await.unwrap_or_default(),
        });
    }

    let body: Resp = response.json().await?;
    Ok(body.value)
}

/// Convenience wrapper: reads the runner endpoint + bearer from the
/// standard GitHub-Actions env vars and fetches an OIDC JWT.
pub async fn fetch_actions_oidc_jwt(
    client: &reqwest::Client,
    audience: &str,
) -> Result<String, ApiError> {
    let url = std::env::var("ACTIONS_ID_TOKEN_REQUEST_URL").map_err(|_| {
        ApiError::InvalidConfiguration(
            "ACTIONS_ID_TOKEN_REQUEST_URL not set (run inside GitHub Actions with \
             `permissions: id-token: write` set on the job)"
                .into(),
        )
    })?;
    let bearer = std::env::var("ACTIONS_ID_TOKEN_REQUEST_TOKEN").map_err(|_| {
        ApiError::InvalidConfiguration("ACTIONS_ID_TOKEN_REQUEST_TOKEN not set".into())
    })?;
    fetch_actions_oidc_jwt_with(client, &url, &bearer, audience).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{bearer_token, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn fetch_actions_oidc_jwt_with_returns_jwt_value() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/_oidc"))
            .and(query_param("audience", "https://api.belaf.dev"))
            .and(bearer_token("runner-bearer"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "value": "eyJalgRS256-test-jwt" })),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/_oidc", server.uri());
        let jwt =
            fetch_actions_oidc_jwt_with(&client, &url, "runner-bearer", "https://api.belaf.dev")
                .await
                .expect("should fetch JWT");
        assert_eq!(jwt, "eyJalgRS256-test-jwt");
    }

    #[tokio::test]
    async fn fetch_actions_oidc_jwt_with_propagates_api_error_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/_oidc"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/_oidc", server.uri());
        let err = fetch_actions_oidc_jwt_with(&client, &url, "x", "y")
            .await
            .expect_err("403 should fail");
        assert!(matches!(err, ApiError::ApiResponse { status: 403, .. }));
    }
}
