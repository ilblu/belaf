use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::time::sleep;

use crate::core::api::{ApiClient, ApiError, DeviceCodeResponse, StoredToken};
use crate::core::auth::token::{delete_token, load_token, save_token};

pub async fn run() -> Result<i32> {
    let client = ApiClient::new();

    let needs_auth = check_existing_auth(&client).await;

    if needs_auth {
        println!("{} Authenticating with belaf...\n", "🔐".bold());

        let device_codes = client.request_device_code().await?;

        println!(
            "Please visit: {}",
            device_codes.verification_uri.cyan().underline()
        );
        println!("Enter code:   {}\n", device_codes.user_code.yellow().bold());

        if open::that(&device_codes.verification_uri_complete).is_ok() {
            println!("{} Opening browser...", "🌐".bold());
        }

        let spinner = create_spinner("Waiting for authorization...");

        let token = poll_for_token(&client, &device_codes).await?;
        spinner.finish_and_clear();

        let stored_token = StoredToken::new(token, device_codes.expires_in.into());
        save_token(&stored_token)?;

        let user = client.get_user_info(&stored_token).await?;
        let display_name = user
            .username
            .as_deref()
            .or(user.name.as_deref())
            .unwrap_or("Unknown");
        println!(
            "\n{} Authenticated as: {} ({})",
            "✓".green(),
            display_name.cyan(),
            user.email.as_deref().unwrap_or("no email")
        );
    } else {
        let token = load_token()?.expect("Token must exist after auth check");
        let user = client.get_user_info(&token).await?;
        let display_name = user
            .username
            .as_deref()
            .or(user.name.as_deref())
            .unwrap_or("Unknown");
        println!(
            "{} Already authenticated as: {} ({})",
            "✓".green(),
            display_name.cyan(),
            user.email.as_deref().unwrap_or("no email")
        );
    }

    let (owner, repo_name) = detect_repository()?;
    let full_repo = format!("{}/{}", owner, repo_name);

    println!(
        "\n{} Checking repository: {}",
        "📦".bold(),
        full_repo.cyan()
    );

    let token = load_token()?.expect("Token must exist");
    let installation = client.check_installation(&token, &full_repo).await?;

    if installation.installed {
        println!("\n{} GitHub App already installed!", "✓".green());
        println!("  Repository: {}", full_repo.cyan());
        if let Some(id) = installation.installation_id {
            println!("  Installation ID: {}", id.to_string().dimmed());
        }
        println!(
            "\n{}",
            "You can now use belaf commands in this repository.".green()
        );
        return Ok(0);
    }

    let install_url = installation
        .install_url
        .unwrap_or_else(|| "https://github.com/apps/belaf-app/installations/new".to_string());

    println!(
        "\n{} GitHub App not installed. Opening installation page...",
        "⚠".yellow()
    );

    if open::that(&install_url).is_ok() {
        println!("{} Opening browser...", "🌐".bold());
    } else {
        println!("Please visit: {}", install_url.cyan().underline());
    }

    let spinner = create_spinner("Waiting for installation...");

    loop {
        sleep(Duration::from_secs(3)).await;

        match client.check_installation(&token, &full_repo).await {
            Ok(result) if result.installed => {
                spinner.finish_and_clear();
                println!("\n{} GitHub App installed successfully!", "✓".green());
                println!("  Repository: {}", full_repo.cyan());
                if let Some(id) = result.installation_id {
                    println!("  Installation ID: {}", id.to_string().dimmed());
                }
                println!(
                    "\n{}",
                    "You can now use belaf commands in this repository.".green()
                );
                return Ok(0);
            }
            Ok(_) => continue,
            Err(e) => {
                spinner.finish_and_clear();
                return Err(e.into());
            }
        }
    }
}

pub async fn logout() -> Result<i32> {
    delete_token()?;
    println!("{} Logged out successfully.", "✓".green());
    Ok(0)
}

pub async fn status() -> Result<i32> {
    let client = ApiClient::new();

    match load_token()? {
        Some(token) if !token.is_expired() => match client.get_user_info(&token).await {
            Ok(user) => {
                println!("{} Authenticated", "✓".green());
                println!(
                    "  User: {} ({})",
                    user.name.as_deref().unwrap_or("Unknown").cyan(),
                    user.email.as_deref().unwrap_or("no email")
                );
                if let Some(expires_at) = token.expires_at {
                    println!("  Expires: {}", expires_at.to_string().dimmed());
                }
                Ok(0)
            }
            Err(ApiError::Unauthorized) => {
                println!("{} Token expired or invalid", "✗".red());
                println!("  Run 'belaf install' to re-authenticate.");
                Ok(1)
            }
            Err(e) => Err(e.into()),
        },
        Some(_) => {
            println!("{} Token expired", "✗".red());
            println!("  Run 'belaf install' to re-authenticate.");
            Ok(1)
        }
        None => {
            println!("{} Not authenticated", "✗".red());
            println!("  Run 'belaf install' to get started.");
            Ok(1)
        }
    }
}

pub async fn whoami() -> Result<i32> {
    let client = ApiClient::new();

    match load_token()? {
        Some(token) if !token.is_expired() => match client.get_user_info(&token).await {
            Ok(user) => {
                println!("{}", user.name.as_deref().unwrap_or("Unknown"));
                Ok(0)
            }
            Err(ApiError::Unauthorized) => {
                eprintln!("{} Not authenticated", "error:".red());
                Ok(1)
            }
            Err(e) => Err(e.into()),
        },
        _ => {
            eprintln!("{} Not authenticated", "error:".red());
            Ok(1)
        }
    }
}

async fn check_existing_auth(client: &ApiClient) -> bool {
    match load_token() {
        Ok(Some(token)) if !token.is_expired() => {
            matches!(
                client.get_user_info(&token).await,
                Err(ApiError::Unauthorized)
            )
        }
        _ => true,
    }
}

async fn poll_for_token(
    client: &ApiClient,
    codes: &DeviceCodeResponse,
) -> Result<String, ApiError> {
    let mut interval = codes.interval.max(5);
    let deadline = Instant::now() + Duration::from_secs(codes.expires_in);

    loop {
        if Instant::now() > deadline {
            return Err(ApiError::DeviceCodeExpired);
        }

        sleep(Duration::from_secs(interval)).await;

        let response = client.poll_for_token(&codes.device_code).await?;

        if response.is_success() {
            return Ok(response.access_token.expect("Token must exist on success"));
        }

        match response.error_code() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval += 5;
                continue;
            }
            Some("expired_token") => return Err(ApiError::DeviceCodeExpired),
            Some("access_denied") => return Err(ApiError::DeviceCodeDenied),
            Some(e) => return Err(ApiError::InvalidResponse(e.to_string())),
            None => continue,
        }
    }
}

fn detect_repository() -> Result<(String, String)> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .context("Failed to run git command")?;

    if !output.status.success() {
        anyhow::bail!("No git remote 'origin' found. Are you in a git repository?");
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_github_remote(&url)
}

fn parse_github_remote(url: &str) -> Result<(String, String)> {
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let repo = rest.trim_end_matches(".git");
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() == 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
    }

    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let repo = rest.trim_end_matches(".git");
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() >= 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
    }

    anyhow::bail!("Could not parse GitHub remote URL: {}", url)
}

fn create_spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .expect("Invalid spinner template"),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(100));
    spinner
}
