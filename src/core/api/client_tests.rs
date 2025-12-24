use super::*;
use wiremock::matchers::{bearer_token, body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn create_test_token() -> StoredToken {
    StoredToken {
        access_token: "test-token-12345".to_string(),
        expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
    }
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
async fn test_get_latest_release_exists() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/cli/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tag_name": "v1.2.3",
            "version": "1.2.3",
            "html_url": "https://github.com/ilblu/belaf/releases/tag/v1.2.3",
            "published_at": "2024-01-01T00:00:00Z"
        })))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.get_latest_release().await;

    assert!(result.is_ok());
    let release = result.unwrap();
    assert!(release.is_some());
    let release = release.unwrap();
    assert_eq!(release.version, "1.2.3");
    assert_eq!(release.tag_name, "v1.2.3");
}

#[tokio::test]
async fn test_get_latest_release_not_found() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/cli/releases/latest"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    let client = ApiClient::with_base_url(&mock_server.uri()).unwrap();
    let result = client.get_latest_release().await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
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
