//! Release automation utilities related to the GitHub service.

use anyhow::{anyhow, Context};
use clap::Parser;
use git_url_parse::types::provider::GenericProvider;
use octocrab::Octocrab;
use tracing::{debug, info};

use crate::core::release::{env::require_var, errors::Result, session::AppSession};

pub struct GitHubInformation {
    owner: String,
    repo: String,
    client: Octocrab,
}

impl GitHubInformation {
    pub fn new(sess: &AppSession) -> Result<Self> {
        Self::new_with_scopes(sess, &["repo"])
    }

    pub fn new_with_scopes(sess: &AppSession, required_scopes: &[&str]) -> Result<Self> {
        let is_ci = sess
            .execution_environment()
            .map(|env| matches!(env, crate::core::release::session::ExecutionEnvironment::Ci))
            .unwrap_or(false);

        let keyring_result = crate::core::auth::token::load_token();
        let env_result = require_var("GITHUB_TOKEN");

        let token = keyring_result
            .inspect_err(|e| {
                debug!("keyring token load failed: {}", e);
            })
            .ok()
            .or_else(|| {
                env_result
                    .as_ref()
                    .inspect_err(|e| {
                        debug!("GITHUB_TOKEN env var not found: {}", e);
                    })
                    .ok()
                    .cloned()
            })
            .ok_or_else(|| {
                if is_ci {
                    anyhow!(
                        "GitHub authentication required in CI. Set the GITHUB_TOKEN environment variable \
                        (typically via secrets.GITHUB_TOKEN in GitHub Actions)."
                    )
                } else {
                    anyhow!(
                        "GitHub authentication required. Run 'belaf login' to authenticate, \
                        or set the GITHUB_TOKEN environment variable."
                    )
                }
            })?;

        if !required_scopes.is_empty() {
            crate::core::auth::github::validate_token_scopes_blocking(&token, required_scopes)
                .context("GitHub token scope validation failed")?;
        }

        let upstream_url = sess.repo.upstream_url()?;
        info!("upstream url: {}", upstream_url);

        let upstream_url = git_url_parse::GitUrl::parse(&upstream_url)
            .map_err(|e| anyhow!("cannot parse upstream Git URL `{}`: {}", upstream_url, e))?;

        let provider: GenericProvider = upstream_url
            .provider_info()
            .map_err(|e| anyhow!("cannot extract provider info from Git URL: {}", e))?;

        let owner = provider.owner().to_string();
        let repo = provider.repo().to_string();

        let client = Octocrab::builder()
            .personal_token(token)
            .build()
            .context("failed to build GitHub client")?;

        Ok(GitHubInformation {
            owner,
            repo,
            client,
        })
    }

    pub fn create_pull_request(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<String> {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let client = self.client.clone();
        let title = title.to_string();
        let head = head.to_string();
        let base = base.to_string();
        let body = body.to_string();

        let future = async move {
            let pr = client
                .pulls(&owner, &repo)
                .create(&title, &head, &base)
                .body(&body)
                .send()
                .await
                .context("failed to create pull request")?;

            let html_url = pr
                .html_url
                .ok_or_else(|| anyhow!("PR response missing html_url"))?
                .to_string();

            info!("created pull request: {}", html_url);
            Ok(html_url)
        };

        match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
            Err(_) => {
                let rt =
                    tokio::runtime::Runtime::new().context("failed to create async runtime")?;
                rt.block_on(future)
            }
        }
    }
}

/// The `github` subcommands.
#[derive(Debug, Eq, PartialEq, Parser)]
pub enum GithubCommands {
    #[command(name = "_credential-helper", hide = true)]
    /// (hidden) github credential helper
    CredentialHelper(CredentialHelperCommand),

    #[command(name = "install-credential-helper")]
    /// Install Belaf as a Git "credential helper", using $GITHUB_TOKEN to log in
    InstallCredentialHelper(InstallCredentialHelperCommand),
}

#[derive(Debug, Eq, PartialEq, Parser)]
pub struct GithubCommand {
    #[command(subcommand)]
    command: GithubCommands,
}

impl GithubCommand {
    pub fn execute(self) -> Result<i32> {
        match self.command {
            GithubCommands::CredentialHelper(o) => o.execute(),
            GithubCommands::InstallCredentialHelper(o) => o.execute(),
        }
    }
}

/// hidden Git credential helper command
#[derive(Debug, Eq, PartialEq, Parser)]
pub struct CredentialHelperCommand {
    #[arg(help = "The operation")]
    operation: String,
}

impl CredentialHelperCommand {
    pub fn execute(self) -> Result<i32> {
        if self.operation != "get" {
            info!("ignoring Git credential operation `{}`", self.operation);
        } else {
            let token = require_var("GITHUB_TOKEN")?;
            println!("username=token");
            println!("password={token}");
        }

        Ok(0)
    }
}

/// Install as a Git credential helper
#[derive(Debug, Eq, PartialEq, Parser)]
pub struct InstallCredentialHelperCommand {}

impl InstallCredentialHelperCommand {
    pub fn execute(self) -> Result<i32> {
        let this_exe = std::env::current_exe()?;
        let this_exe = this_exe.to_str().ok_or_else(|| {
            anyhow!(
                "cannot install belaf as a Git \
                 credential helper because its executable path is not Unicode"
            )
        })?;
        let mut cfg = git2::Config::open_default().context("cannot open Git configuration")?;
        cfg.set_str(
            "credential.helper",
            &format!("{this_exe} github _credential-helper"),
        )
        .context("cannot update Git configuration setting `credential.helper`")?;
        Ok(0)
    }
}
