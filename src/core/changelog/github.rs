use async_stream::stream as async_stream;
use futures::Stream;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::error::Result;
use super::remote::{RemoteCommit, RemotePullRequest, MAX_PAGE_SIZE};
use crate::core::api::{ApiClient, ApiCommit, ApiPullRequest, StoredToken};

impl RemoteCommit for ApiCommit {
    fn id(&self) -> String {
        self.sha.clone()
    }

    fn username(&self) -> Option<String> {
        self.author.as_ref().and_then(|a| a.login.clone())
    }

    fn timestamp(&self) -> Option<i64> {
        self.timestamp.as_ref().map(|ts| {
            OffsetDateTime::parse(ts, &Rfc3339)
                .map(|dt| dt.unix_timestamp())
                .unwrap_or(0)
        })
    }
}

impl RemotePullRequest for ApiPullRequest {
    fn number(&self) -> i64 {
        self.number
    }

    fn title(&self) -> Option<String> {
        self.title.clone()
    }

    fn labels(&self) -> Vec<String> {
        self.labels.clone()
    }

    fn merge_commit(&self) -> Option<String> {
        self.merge_commit_sha.clone()
    }
}

#[derive(Debug, Clone)]
pub struct GitHubClient {
    owner: String,
    repo: String,
    client: ApiClient,
    token: StoredToken,
}

impl GitHubClient {
    pub fn new(owner: String, repo: String, token: StoredToken) -> Result<Self> {
        Ok(Self {
            owner,
            repo,
            client: ApiClient::new(),
            token,
        })
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
        let owner = self.owner.clone();
        let repo = self.repo.clone();

        async_stream! {
            let mut page = 1u32;
            let per_page = MAX_PAGE_SIZE as u32;

            loop {
                let commits_result = self.client
                    .get_commits(&self.token, &owner, &repo, ref_name.as_deref(), page, per_page)
                    .await;

                match commits_result {
                    Ok(commits) => {
                        if commits.is_empty() {
                            break;
                        }

                        for commit in commits {
                            yield Ok(Box::new(commit) as Box<dyn RemoteCommit>);
                        }

                        page += 1;
                    }
                    Err(e) => {
                        yield Err(e.into());
                        break;
                    }
                }
            }
        }
    }

    fn get_pull_request_stream<'a>(
        &'a self,
    ) -> impl Stream<Item = Result<Box<dyn RemotePullRequest>>> + 'a {
        let owner = self.owner.clone();
        let repo = self.repo.clone();

        async_stream! {
            let mut page = 1u32;
            let per_page = MAX_PAGE_SIZE as u32;

            loop {
                let prs_result = self.client
                    .get_pull_requests(&self.token, &owner, &repo, page, per_page)
                    .await;

                match prs_result {
                    Ok(prs) => {
                        if prs.is_empty() {
                            break;
                        }

                        for pr in prs {
                            yield Ok(Box::new(pr) as Box<dyn RemotePullRequest>);
                        }

                        page += 1;
                    }
                    Err(e) => {
                        yield Err(e.into());
                        break;
                    }
                }
            }
        }
    }
}
