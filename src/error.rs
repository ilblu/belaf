use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    #[error("Authentication required. Run 'belaf auth login'")]
    AuthenticationRequired,

    #[error("GitHub API error: {0}")]
    GitHubApi(String),

    #[error("Token storage error: {0}")]
    TokenStorage(String),

    #[error("Project already initialized. Run 'belaf init --force' to overwrite.")]
    AlreadyInitialized,

    #[error("Dialog error: {0}")]
    Dialog(#[from] dialoguer::Error),

    #[error("Project not initialized. Run 'belaf init' to get started.")]
    ProjectNotInitialized,

    #[error("API error: {0}")]
    Api(#[from] crate::core::api::ApiError),
}

pub type Result<T> = std::result::Result<T, CliError>;
