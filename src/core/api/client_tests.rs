use super::*;
use wiremock::matchers::{bearer_token, body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn create_test_token() -> StoredToken {
    StoredToken {
        access_token: "test-token-12345".to_string(),
        expires_at: Some(time::OffsetDateTime::now_utc() + time::Duration::hours(1)),
    }
}

#[test]
fn test_stored_token_serde_roundtrip() {
    let token = StoredToken::new("test-token".to_string(), Some(3600));
    let json = serde_json::to_string(&token).unwrap();
    let deserialized: StoredToken = serde_json::from_str(&json).unwrap();
    assert_eq!(token.access_token, deserialized.access_token);
    assert!(deserialized.expires_at.is_some());
}

#[tokio::test]
async fn test_request_device_code_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/auth/device/code"))
        .and(body_json(serde_json::json!({
            "client_id": "belaf-cli",
            "scope": "cli"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "device_code": "test-device-code",
            "user_code": "ABCD-1234",
            "verification_uri": "https://dashboard.belaf.dev/device",
            "verification_uri_complete": "https://dashboard.belaf.dev/device?code=ABCD-1234",
            "expires_in": 900,
            "interval": 5
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.request_device_code().await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.device_code, "test-device-code");
    assert_eq!(response.user_code, "ABCD-1234");
    assert_eq!(response.expires_in, 900);
    assert_eq!(response.interval, 5);
}

#[tokio::test]
async fn test_request_device_code_api_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/auth/device/code"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.request_device_code().await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, ApiError::ApiResponse { status: 500, .. }));
}

#[tokio::test]
async fn test_poll_for_token_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/auth/device/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "access-token-xyz",
            "token_type": "Bearer",
            "expires_in": 3600
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.poll_for_token("test-device-code").await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert!(response.is_success());
    assert_eq!(response.access_token, Some("access-token-xyz".to_string()));
    assert_eq!(response.expires_in, Some(3600));
}

#[tokio::test]
async fn test_poll_for_token_authorization_pending() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/auth/device/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "error": "authorization_pending",
            "error_description": "The authorization request is still pending"
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.poll_for_token("test-device-code").await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert!(!response.is_success());
    assert_eq!(response.error_code(), Some("authorization_pending"));
}

#[tokio::test]
async fn test_poll_for_token_slow_down() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/auth/device/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "error": "slow_down",
            "error_description": "You are polling too frequently"
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.poll_for_token("test-device-code").await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert!(!response.is_success());
    assert_eq!(response.error_code(), Some("slow_down"));
}

#[tokio::test]
async fn test_check_installation_installed() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("GET"))
        .and(path("/api/cli/check-installation"))
        .and(query_param("repo", "owner/repo"))
        .and(bearer_token(&token.access_token))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "installed": true,
            "installation_id": 12345,
            "repository_id": 67890
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.check_installation(&token, "owner/repo").await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert!(response.installed);
    assert_eq!(response.installation_id, Some(12345));
    assert_eq!(response.repository_id, Some(67890));
}

#[tokio::test]
async fn test_check_installation_not_installed() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("GET"))
        .and(path("/api/cli/check-installation"))
        .and(query_param("repo", "owner/repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "installed": false,
            "install_url": "https://github.com/apps/belaf/installations/new"
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.check_installation(&token, "owner/repo").await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert!(!response.installed);
    assert!(response.install_url.is_some());
}

#[tokio::test]
async fn test_check_installation_unauthorized() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("GET"))
        .and(path("/api/cli/check-installation"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.check_installation(&token, "owner/repo").await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ApiError::Unauthorized));
}

#[tokio::test]
async fn test_get_user_info_success() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("GET"))
        .and(path("/api/cli/me"))
        .and(bearer_token(&token.access_token))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "user-123",
            "username": "testuser",
            "name": "Test User",
            "email": "test@example.com"
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.get_user_info(&token).await;

    assert!(result.is_ok());
    let user = result.unwrap();
    assert_eq!(user.id, "user-123");
    assert_eq!(user.username, Some("testuser".to_string()));
    assert_eq!(user.display_name(), "testuser");
}

#[tokio::test]
async fn test_get_user_info_unauthorized() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("GET"))
        .and(path("/api/cli/me"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.get_user_info(&token).await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ApiError::Unauthorized));
}

#[tokio::test]
async fn test_get_git_credentials_success() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("GET"))
        .and(path("/api/cli/repos/owner/repo/git/credentials"))
        .and(bearer_token(&token.access_token))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": "ghs_temporary_token",
            "expires_at": "2024-01-01T12:00:00Z"
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.get_git_credentials(&token, "owner", "repo").await;

    assert!(result.is_ok());
    let creds = result.unwrap();
    assert_eq!(creds.token, "ghs_temporary_token");
}

#[tokio::test]
async fn test_api_error_is_transient() {
    assert!(ApiError::Network("connection reset".to_string()).is_transient());

    let server_error = ApiError::ApiResponse {
        status: 503,
        message: "Service Unavailable".to_string(),
    };
    assert!(server_error.is_transient());

    let client_error = ApiError::ApiResponse {
        status: 400,
        message: "Bad Request".to_string(),
    };
    assert!(!client_error.is_transient());

    assert!(!ApiError::Unauthorized.is_transient());
    assert!(!ApiError::DeviceCodeExpired.is_transient());
}

#[tokio::test]
async fn test_exchange_oidc_token_success() {
    let mock_server = MockServer::start().await;
    let expires_at = time::OffsetDateTime::now_utc() + time::Duration::minutes(30);
    let expires_at_str = expires_at
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();

    Mock::given(method("POST"))
        .and(path("/api/cli/auth/oidc/exchange"))
        .and(body_json(serde_json::json!({
            "token": "fake-oidc-jwt"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "ci-jwt-abc",
            "token_type": "Bearer",
            "expires_at": expires_at_str,
            "installation_id": 12345,
            "repository_id": 67890,
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let token = client
        .exchange_oidc_token("fake-oidc-jwt".to_string())
        .await
        .expect("should succeed");

    assert_eq!(token.access_token, "ci-jwt-abc");
    assert!(token.expires_at.is_some());
    assert!(
        !token.is_expired(),
        "fresh 30-min token must not look expired"
    );
}

#[tokio::test]
async fn test_exchange_oidc_token_403_when_app_not_installed() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/cli/auth/oidc/exchange"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "error": "belaf GitHub App not installed on this repository"
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let err = client
        .exchange_oidc_token("fake-oidc-jwt".to_string())
        .await
        .expect_err("403 should fail");
    assert!(matches!(err, ApiError::ApiResponse { status: 403, .. }));
}

#[tokio::test]
async fn test_exchange_oidc_token_401_on_bad_jwt() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/cli/auth/oidc/exchange"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(serde_json::json!({ "error": "Invalid OIDC token" })),
        )
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let err = client
        .exchange_oidc_token("rogue-jwt".to_string())
        .await
        .expect_err("401 should fail");
    assert!(matches!(err, ApiError::Unauthorized));
}

// ---------------------------------------------------------------------------
// Pull-request lookup and update — the calls that make `prepare` idempotent.
// ---------------------------------------------------------------------------

fn pull_request_json(number: i64, head_ref: &str) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "title": "chore(release): v1.2.3",
        "labels": ["release:minor"],
        "head_ref": head_ref,
        "html_url": format!("https://github.com/owner/repo/pull/{number}"),
        "state": "open"
    })
}

#[tokio::test]
async fn test_find_open_pull_request_filters_by_head() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("GET"))
        .and(path("/api/cli/repos/owner/repo/pulls"))
        .and(bearer_token(&token.access_token))
        .and(query_param("state", "open"))
        .and(query_param("head", "belaf/release--main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "pull_requests": [pull_request_json(42, "belaf/release--main")],
            "has_more": false
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let found = client
        .find_open_pull_request(&token, "owner", "repo", "belaf/release--main")
        .await
        .expect("lookup should succeed")
        .expect("a matching PR should be found");

    assert_eq!(found.number, 42);
    assert_eq!(found.head_ref.as_deref(), Some("belaf/release--main"));
}

#[tokio::test]
async fn test_find_open_pull_request_returns_none_when_empty() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("GET"))
        .and(path("/api/cli/repos/owner/repo/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "pull_requests": [],
            "has_more": false
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let found = client
        .find_open_pull_request(&token, "owner", "repo", "belaf/release--main")
        .await
        .expect("lookup should succeed");

    assert!(found.is_none());
}

/// A server that ignores the `head` filter must not make us update an
/// unrelated pull request — the client re-checks `head_ref` itself.
#[tokio::test]
async fn test_find_open_pull_request_rejects_mismatched_head() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("GET"))
        .and(path("/api/cli/repos/owner/repo/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "pull_requests": [pull_request_json(7, "feature/unrelated")],
            "has_more": false
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let found = client
        .find_open_pull_request(&token, "owner", "repo", "belaf/release--main")
        .await
        .expect("lookup should succeed");

    assert!(found.is_none());
}

#[tokio::test]
async fn test_find_open_pull_request_encodes_branch_name() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    // `query_param` matches the decoded value, so this only passes if the
    // slash survived the round trip intact.
    Mock::given(method("GET"))
        .and(path("/api/cli/repos/owner/repo/pulls"))
        .and(query_param("head", "belaf/release--feature-x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "pull_requests": [pull_request_json(9, "belaf/release--feature-x")],
            "has_more": false
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let found = client
        .find_open_pull_request(&token, "owner", "repo", "belaf/release--feature-x")
        .await
        .expect("lookup should succeed");

    assert_eq!(found.map(|pr| pr.number), Some(9));
}

#[tokio::test]
async fn test_get_pull_requests_still_asks_for_closed() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    // The changelog path depends on this: it enriches entries from merged
    // PRs, so widening it to open ones would pull in unreleased work.
    Mock::given(method("GET"))
        .and(path("/api/cli/repos/owner/repo/pulls"))
        .and(query_param("state", "closed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "pull_requests": [pull_request_json(3, "some-branch")],
            "has_more": false
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let prs = client
        .get_pull_requests(&token, "owner", "repo", 1, 30)
        .await
        .expect("listing closed PRs should succeed");

    assert_eq!(prs.len(), 1);
}

#[tokio::test]
async fn test_update_pull_request_sends_title_and_body() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("PATCH"))
        .and(path("/api/cli/repos/owner/repo/pulls/42"))
        .and(bearer_token(&token.access_token))
        .and(body_json(serde_json::json!({
            "title": "chore(release): v2.0.0",
            "body": "## Releases\n- foo 2.0.0"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 42,
            "html_url": "https://github.com/owner/repo/pull/42",
            "state": "open"
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let updated = client
        .update_pull_request(
            &token,
            "owner",
            "repo",
            42,
            "chore(release): v2.0.0",
            "## Releases\n- foo 2.0.0",
        )
        .await
        .expect("update should succeed");

    assert_eq!(updated.number, 42);
    assert_eq!(updated.html_url, "https://github.com/owner/repo/pull/42");
}

#[tokio::test]
async fn test_update_pull_request_surfaces_not_found() {
    let mock_server = MockServer::start().await;
    let token = create_test_token();

    Mock::given(method("PATCH"))
        .and(path("/api/cli/repos/owner/repo/pulls/404"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": "Pull request not found"
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let err = client
        .update_pull_request(&token, "owner", "repo", 404, "t", "b")
        .await
        .expect_err("a missing PR should be an error");

    assert!(matches!(err, ApiError::ApiResponse { status: 404, .. }));
}
