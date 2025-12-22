use async_stream::stream as async_stream;
use futures::{stream, Stream, StreamExt};
use reqwest_middleware::ClientWithMiddleware;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::error::Result;
use super::remote::{create_http_client, RemoteCommit, RemotePullRequest, MAX_PAGE_SIZE};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubCommit {
    pub sha: String,
    pub author: Option<GitHubCommitAuthor>,
    pub commit: Option<GitHubCommitDetails>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubCommitDetails {
    pub author: GitHubCommitDetailsAuthor,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubCommitDetailsAuthor {
    pub date: String,
}

impl RemoteCommit for GitHubCommit {
    fn id(&self) -> String {
        self.sha.clone()
    }

    fn username(&self) -> Option<String> {
        self.author.clone().and_then(|v| v.login)
    }

    fn timestamp(&self) -> Option<i64> {
        self.commit
            .clone()
            .map(|f| self.convert_to_unix_timestamp(f.author.date.as_str()))
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubCommitAuthor {
    pub login: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestLabel {
    pub name: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubPullRequest {
    pub number: i64,
    pub title: Option<String>,
    pub merge_commit_sha: Option<String>,
    pub labels: Vec<PullRequestLabel>,
}

impl RemotePullRequest for GitHubPullRequest {
    fn number(&self) -> i64 {
        self.number
    }

    fn title(&self) -> Option<String> {
        self.title.clone()
    }

    fn labels(&self) -> Vec<String> {
        self.labels.iter().map(|v| v.name.clone()).collect()
    }

    fn merge_commit(&self) -> Option<String> {
        self.merge_commit_sha.clone()
    }
}

#[derive(Debug, Clone)]
pub struct GitHubClient {
    owner: String,
    repo: String,
    client: ClientWithMiddleware,
    api_url: String,
}

impl GitHubClient {
    pub fn new(owner: String, repo: String, token: Option<SecretString>) -> Result<Self> {
        let client = create_http_client("application/vnd.github+json", token.as_ref())?;

        let api_url = std::env::var("GITHUB_API_URL")
            .unwrap_or_else(|_| "https://api.github.com".to_string());

        Ok(Self {
            owner,
            repo,
            client,
            api_url,
        })
    }

    fn commits_url(&self, ref_name: Option<&str>, page: i32) -> String {
        let mut url = format!(
            "{}/repos/{}/{}/commits?per_page={}&page={}",
            self.api_url, self.owner, self.repo, MAX_PAGE_SIZE, page
        );

        if let Some(ref_name) = ref_name {
            url.push_str(&format!("&sha={}", ref_name));
        }

        url
    }

    fn pull_requests_url(&self, page: i32) -> String {
        format!(
            "{}/repos/{}/{}/pulls?per_page={}&page={}&state=closed",
            self.api_url, self.owner, self.repo, MAX_PAGE_SIZE, page
        )
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        log::debug!("Sending request to: {url}");
        let response = self.client.get(url).send().await?.error_for_status()?;
        let text = response.text().await?;
        log::trace!("Response: {:?}", text);
        Ok(serde_json::from_str::<T>(&text)?)
    }

    pub async fn get_commits(&self, ref_name: Option<&str>) -> Result<Vec<Box<dyn RemoteCommit>>> {
        use futures::TryStreamExt;
        self.get_commit_stream(ref_name).try_collect().await
    }

    pub async fn get_pull_requests(&self) -> Result<Vec<Box<dyn RemotePullRequest>>> {
        use futures::TryStreamExt;
        self.get_pull_request_stream().try_collect().await
    }

    fn get_commit_stream<'a>(
        &'a self,
        ref_name: Option<&str>,
    ) -> impl Stream<Item = Result<Box<dyn RemoteCommit>>> + 'a {
        let ref_name = ref_name.map(|s| s.to_string());
        async_stream! {
            let page_stream = stream::iter(0..)
                .map(|page| {
                    let ref_name = ref_name.clone();
                    async move {
                        let url = self.commits_url(ref_name.as_deref(), page);
                        self.get_json::<Vec<GitHubCommit>>(&url).await
                    }
                })
                .buffered(10);

            let mut page_stream = Box::pin(page_stream);

            while let Some(page_result) = page_stream.next().await {
                match page_result {
                    Ok(commits) => {
                        if commits.is_empty() {
                            break;
                        }

                        for commit in commits {
                            yield Ok(Box::new(commit) as Box<dyn RemoteCommit>);
                        }
                    }
                    Err(e) => {
                        yield Err(e);
                        break;
                    }
                }
            }
        }
    }

    fn get_pull_request_stream<'a>(
        &'a self,
    ) -> impl Stream<Item = Result<Box<dyn RemotePullRequest>>> + 'a {
        async_stream! {
            let page_stream = stream::iter(0..)
                .map(|page| async move {
                    let url = self.pull_requests_url(page);
                    self.get_json::<Vec<GitHubPullRequest>>(&url).await
                })
                .buffered(5);

            let mut page_stream = Box::pin(page_stream);

            while let Some(page_result) = page_stream.next().await {
                match page_result {
                    Ok(prs) => {
                        if prs.is_empty() {
                            break;
                        }

                        for pr in prs {
                            yield Ok(Box::new(pr) as Box<dyn RemotePullRequest>);
                        }
                    }
                    Err(e) => {
                        yield Err(e);
                        break;
                    }
                }
            }
        }
    }
}
