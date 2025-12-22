use thiserror::Error;

pub type Result<T> = std::result::Result<T, self::Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),

    #[error("Git error: {0}")]
    GitError(#[from] git2::Error),

    #[error("Commit does not match conventional format: {0}")]
    ParseError(#[from] git_conventional::Error),

    #[error("Grouping error: {0}")]
    GroupError(String),

    #[error("Field error: {0}")]
    FieldError(String),

    #[error("Template parse error:\n{0}")]
    TemplateParseError(String),

    #[error("Template render error:\n{0}")]
    TemplateRenderError(String),

    #[error("Template error: {0}")]
    TemplateError(#[from] tera::Error),

    #[error("Regex error: {0}")]
    RegexError(#[from] regex::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Semver parse error: {0}")]
    SemverError(#[from] semver::Error),

    #[error("HTTP request error: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("HTTP middleware error: {0}")]
    MiddlewareError(#[from] reqwest_middleware::Error),

    #[error("HTTP header error: {0}")]
    HeaderError(#[from] reqwest::header::InvalidHeaderValue),

    #[error("API error: {0}")]
    ApiError(#[from] crate::core::api::ApiError),

    #[error("Remote not configured")]
    RemoteNotConfigured,

    #[error("Date parse error: {0}")]
    DateParseError(String),

    #[error("Changelog error: {0}")]
    ChangelogError(String),

    #[error("Command error: {0}")]
    CommandError(String),

    #[error("System time error: {0}")]
    SystemTimeError(#[from] std::time::SystemTimeError),

    #[error("Integer parse error: {0}")]
    IntParseError(#[from] std::num::TryFromIntError),

    #[error("Pattern error: {0}")]
    PatternError(#[from] glob::PatternError),

    #[error("Requiring all commits be conventional but found {0} unconventional commits")]
    UnconventionalCommitsError(i32),

    #[error("Found {0} unmatched commit(s)")]
    UnmatchedCommitsError(i32),
}
