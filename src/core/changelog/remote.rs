use std::time::Duration;

use dyn_clone::DynClone;
use http_cache_reqwest::{CACacheManager, Cache, CacheMode, HttpCache, HttpCacheOptions};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::contributor::RemoteContributor;
use super::error::{Error, Result};

pub const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
pub const REQUEST_TIMEOUT: u64 = 30;
pub const REQUEST_KEEP_ALIVE: u64 = 60;
pub const MAX_PAGE_SIZE: usize = 100;

pub trait RemoteCommit: DynClone + Send + Sync {
    fn id(&self) -> String;
    fn username(&self) -> Option<String>;
    fn timestamp(&self) -> Option<i64>;

    fn convert_to_unix_timestamp(&self, date: &str) -> i64 {
        OffsetDateTime::parse(date, &Rfc3339)
            .expect("failed to parse date")
            .unix_timestamp()
    }
}

dyn_clone::clone_trait_object!(RemoteCommit);

pub trait RemotePullRequest: DynClone + Send + Sync {
    fn number(&self) -> i64;
    fn title(&self) -> Option<String>;
    fn labels(&self) -> Vec<String>;
    fn merge_commit(&self) -> Option<String>;
}

dyn_clone::clone_trait_object!(RemotePullRequest);

pub type RemoteMetadata = (Vec<Box<dyn RemoteCommit>>, Vec<Box<dyn RemotePullRequest>>);

#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct RemoteReleaseMetadata {
    pub contributors: Vec<RemoteContributor>,
}

pub fn create_http_client(
    accept_header: &str,
    token: Option<&SecretString>,
) -> Result<ClientWithMiddleware> {
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        HeaderValue::from_str(accept_header)?,
    );
    if let Some(token) = token {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", token.expose_secret()).parse()?,
        );
    }
    headers.insert(reqwest::header::USER_AGENT, USER_AGENT.parse()?);

    let client = Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT))
        .tcp_keepalive(Duration::from_secs(REQUEST_KEEP_ALIVE))
        .default_headers(headers)
        .build()?;

    let cache_path = dirs::cache_dir()
        .ok_or_else(|| Error::ChangelogError("failed to find cache directory".to_string()))?
        .join(env!("CARGO_PKG_NAME"));

    let client = ClientBuilder::new(client)
        .with(Cache(HttpCache {
            mode: CacheMode::Default,
            manager: CACacheManager { path: cache_path },
            options: HttpCacheOptions::default(),
        }))
        .build();

    Ok(client)
}
