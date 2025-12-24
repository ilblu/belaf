use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ApiError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("Network error (transient): {0}")]
    Network(String),

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

    #[error("Failed to create HTTP client: {0}")]
    ClientCreation(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
}

impl ApiError {
    pub fn is_transient(&self) -> bool {
        match self {
            ApiError::Network(_) => true,
            ApiError::Request(_) => true,
            ApiError::ApiResponse { status, .. } => *status >= 500,
            _ => false,
        }
    }
}
