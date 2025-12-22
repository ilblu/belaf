use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("API returned error: {status} - {message}")]
    ApiResponse { status: u16, message: String },

    #[error("Authentication required. Run 'belaf install' first.")]
    Unauthorized,

    #[error("Device authorization expired. Please try again.")]
    DeviceCodeExpired,

    #[error("Device authorization denied by user.")]
    DeviceCodeDenied,

    #[error("Authorization pending. Waiting for user...")]
    AuthorizationPending,

    #[error("Rate limited. Slow down polling.")]
    SlowDown,

    #[error("Token storage error: {0}")]
    TokenStorage(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
}
