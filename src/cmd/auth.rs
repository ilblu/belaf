use anyhow::{Context, Result};
use dialoguer::{theme::ColorfulTheme, Select};
use owo_colors::OwoColorize;
use std::io::{self, Write};

use crate::core::ai::client::AnthropicClient;
use crate::core::ai::credentials::{
    delete_credentials, load_credentials, now_unix, store_credentials, ClaudeCredential,
};
use crate::core::ai::oauth::OAuthFlow;
use crate::core::auth::{github, token};
use crate::utils::theme::{header, highlight, step_message, success_message, url};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Service {
    GitHub,
    Anthropic,
    Both,
}

pub async fn login(github: bool, anthropic: bool, all: bool, no_browser: bool) -> Result<()> {
    let service = if all || (github && anthropic) {
        Service::Both
    } else if github {
        Service::GitHub
    } else if anthropic {
        Service::Anthropic
    } else {
        select_service_for_login()?
    };

    match service {
        Service::GitHub => login_github(no_browser).await,
        Service::Anthropic => login_anthropic().await,
        Service::Both => {
            login_github(no_browser).await?;
            println!();
            login_anthropic().await
        }
    }
}

pub async fn logout(github: bool, anthropic: bool, all: bool) -> Result<()> {
    let service = if all || (github && anthropic) {
        Service::Both
    } else if github {
        Service::GitHub
    } else if anthropic {
        Service::Anthropic
    } else {
        select_service_for_logout()?
    };

    match service {
        Service::GitHub => logout_github().await,
        Service::Anthropic => logout_anthropic().await,
        Service::Both => {
            logout_github().await?;
            logout_anthropic().await
        }
    }
}

pub async fn status() -> Result<()> {
    println!();
    println!("{}", "Authentication Status".bold());
    println!("{}", "=====================".dimmed());

    println!();
    println!("{}", "GitHub".bold());
    println!("{}", "------".dimmed());
    github_status().await?;

    println!();
    println!("{}", "Anthropic (Claude)".bold());
    println!("{}", "------------------".dimmed());
    anthropic_status().await?;

    Ok(())
}

fn select_service_for_login() -> Result<Service> {
    println!();
    println!("{}", "Authentication".bold());
    println!("{}", "==============".dimmed());
    println!();

    let options = vec![
        "GitHub        (for releases & pull requests)",
        "Anthropic     (for AI-powered changelogs)",
        "Both          (GitHub + Anthropic)",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Which service would you like to connect?")
        .items(&options)
        .default(0)
        .interact()?;

    Ok(match selection {
        0 => Service::GitHub,
        1 => Service::Anthropic,
        2 => Service::Both,
        _ => unreachable!(),
    })
}

fn select_service_for_logout() -> Result<Service> {
    println!();

    let options = vec![
        "GitHub        (sign out from GitHub)",
        "Anthropic     (sign out from Claude)",
        "Both          (sign out from all)",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Which service would you like to sign out from?")
        .items(&options)
        .default(0)
        .interact()?;

    Ok(match selection {
        0 => Service::GitHub,
        1 => Service::Anthropic,
        2 => Service::Both,
        _ => unreachable!(),
    })
}

async fn login_github(no_browser: bool) -> Result<()> {
    let client_id = "Ov23liNPpcjTMYaP841Y";
    let device_codes = github::request_device_code(client_id).await?;

    println!("{}", header("GitHub Authentication"));

    if !no_browser {
        println!("\n{}", step_message("Opening browser to:"));
        println!("  {}", url(&device_codes.verification_uri));

        if let Err(e) = open::that(&device_codes.verification_uri) {
            eprintln!("{} Failed to open browser: {}", "!".yellow(), e);
            println!("{}", step_message("Please open the URL manually"));
        }
    } else {
        println!("\n{}", step_message("Please visit:"));
        println!("  {}", url(&device_codes.verification_uri));
    }

    println!("\n{}", step_message("Enter code:"));
    println!("  {}", device_codes.user_code.cyan().bold());
    println!();

    println!("{} Waiting for authorization...", "→".cyan());

    let access_token = device_codes.poll_for_token().await?;

    println!("{} Authorized!", "✓".green());
    println!("{}", step_message("Getting user info..."));
    let username = github::get_username(&access_token).await?;

    token::save_token(&access_token)?;

    println!(
        "\n{}",
        success_message(&format!(
            "Successfully authenticated as {}",
            highlight(&username)
        ))
    );

    Ok(())
}

async fn login_anthropic() -> Result<()> {
    if let Some(existing) = load_credentials()? {
        println!(
            "{} Already logged in with {}",
            "!".yellow(),
            existing.credential_type()
        );
        print!("Do you want to re-authenticate? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Login cancelled.");
            return Ok(());
        }
    }

    println!();
    println!("{}", "Anthropic Authentication".bold());
    println!("{}", "========================".dimmed());
    println!();

    let options = vec!["Claude Max/Pro Subscription (OAuth)", "Anthropic API Key"];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("How would you like to authenticate?")
        .items(&options)
        .default(0)
        .interact()?;

    match selection {
        0 => login_anthropic_oauth().await,
        1 => login_anthropic_api_key().await,
        _ => unreachable!(),
    }
}

async fn login_anthropic_oauth() -> Result<()> {
    println!();
    println!("This will authenticate belaf with your Claude Max/Pro subscription.");
    println!();

    let flow = OAuthFlow::new();
    let auth_url = flow.authorization_url();

    println!("{} Opening browser for authentication...", "→".cyan());
    println!();

    if open::that(&auth_url).is_err() {
        println!("{} Could not open browser automatically.", "!".yellow());
        println!();
        println!("Please open this URL manually:");
        println!("{}", auth_url.dimmed());
    }

    println!();
    println!(
        "{} After logging in, you will see a page with an authorization code.",
        "→".cyan()
    );
    println!();
    print!("{} Paste the authorization code here: ", "?".green());
    io::stdout().flush()?;

    let mut code = String::new();
    io::stdin().read_line(&mut code)?;
    let code = code.trim();

    if code.is_empty() {
        anyhow::bail!("No authorization code provided");
    }

    println!();
    println!("{} Exchanging code for tokens...", "→".cyan());

    let tokens = flow.exchange_code(code).await?;

    let credential = ClaudeCredential::OAuthToken {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: now_unix() + tokens.expires_in,
    };

    store_credentials(&credential)?;

    println!();
    println!(
        "{} Successfully logged in with Claude Max/Pro subscription!",
        "✓".green().bold()
    );
    println!();
    println!(
        "You can now use {} to generate AI-powered changelogs.",
        "belaf prepare".cyan()
    );

    Ok(())
}

async fn login_anthropic_api_key() -> Result<()> {
    println!();
    println!("Enter your Anthropic API key.");
    println!(
        "{}",
        "Get one at: https://console.anthropic.com/settings/keys".dimmed()
    );
    println!();

    print!("{} API Key: ", "?".green());
    io::stdout().flush()?;

    let mut api_key = String::new();
    io::stdin().read_line(&mut api_key)?;
    let api_key = api_key.trim();

    if api_key.is_empty() {
        anyhow::bail!("No API key provided");
    }

    if !api_key.starts_with("sk-ant-") {
        println!(
            "{} Warning: API key doesn't start with 'sk-ant-'. Are you sure it's correct?",
            "!".yellow()
        );
        print!("Continue anyway? [y/N] ");
        io::stdout().flush()?;

        let mut confirm = String::new();
        io::stdin().read_line(&mut confirm)?;
        if !confirm.trim().eq_ignore_ascii_case("y") {
            println!("Login cancelled.");
            return Ok(());
        }
    }

    let credential = ClaudeCredential::ApiKey(api_key.to_string());
    store_credentials(&credential)?;

    println!();
    println!("{} Successfully saved API key!", "✓".green().bold());
    println!();
    println!(
        "You can now use {} to generate AI-powered changelogs.",
        "belaf prepare".cyan()
    );

    Ok(())
}

async fn logout_github() -> Result<()> {
    match token::load_token() {
        Ok(_) => {
            token::delete_token()?;
            println!("{} Signed out from GitHub.", "✓".green());
        }
        Err(_) => {
            println!("{} Not logged in to GitHub.", "!".yellow());
        }
    }
    Ok(())
}

async fn logout_anthropic() -> Result<()> {
    match load_credentials()? {
        Some(creds) => {
            delete_credentials()?;
            println!(
                "{} Signed out from {} credentials.",
                "✓".green(),
                creds.credential_type()
            );
        }
        None => {
            println!("{} Not logged in to Anthropic.", "!".yellow());
        }
    }
    Ok(())
}

async fn github_status() -> Result<()> {
    match token::load_token() {
        Ok(token_val) => match github::get_username(&token_val).await {
            Ok(username) => {
                println!("  {} Logged in as {}", "✓".green(), username.cyan());
            }
            Err(_) => {
                println!("  {} Token exists but may be invalid", "!".yellow());
                println!(
                    "  Run {} to re-authenticate",
                    "belaf auth login --github".cyan()
                );
            }
        },
        Err(_) => {
            println!("  {} Not logged in", "✗".red());
            println!(
                "  Run {} to authenticate",
                "belaf auth login --github".cyan()
            );
        }
    }
    Ok(())
}

async fn anthropic_status() -> Result<()> {
    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        let masked = if api_key.len() > 8 {
            format!("{}...{}", &api_key[..4], &api_key[api_key.len() - 4..])
        } else {
            "****".to_string()
        };
        println!("  {} Using API Key from environment", "✓".green());
        println!("    Key: {}", masked.dimmed());
        println!(
            "    {}",
            "Note: ANTHROPIC_API_KEY takes priority over OAuth login.".dimmed()
        );
        return Ok(());
    }

    match load_credentials()? {
        Some(creds) => {
            println!("  {} Logged in", "✓".green());
            println!("    Type: {}", creds.credential_type());

            if let ClaudeCredential::OAuthToken { expires_at, .. } = &creds {
                let now = now_unix();
                if *expires_at > now {
                    let remaining = expires_at - now;
                    let hours = remaining / 3600;
                    let minutes = (remaining % 3600) / 60;
                    println!("    Expires in: {}h {}m", hours, minutes);
                } else {
                    println!("    Status: {} (will auto-refresh)", "Expired".yellow());
                }
            }
        }
        None => {
            println!("  {} Not logged in", "✗".red());
            println!(
                "  Run {} to authenticate",
                "belaf auth login --anthropic".cyan()
            );
            println!(
                "  Or set {} environment variable",
                "ANTHROPIC_API_KEY".cyan()
            );
        }
    }

    Ok(())
}

pub async fn test_anthropic() -> Result<()> {
    println!();
    println!("{}", "Claude AI Connection Test".bold());
    println!("{}", "=========================".dimmed());
    println!();

    println!("{} Checking credentials...", "→".cyan());

    let creds = load_credentials()?;
    let cred_source = if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        "ANTHROPIC_API_KEY environment variable"
    } else if creds.is_some() {
        creds
            .as_ref()
            .map(|c| c.credential_type())
            .unwrap_or("Unknown")
    } else {
        println!("{} No credentials found", "✗".red());
        println!();
        println!(
            "Run {} to authenticate first.",
            "belaf auth login --anthropic".cyan()
        );
        return Ok(());
    };

    println!("  Credential source: {}", cred_source.dimmed());
    println!();

    println!("{} Initializing API client...", "→".cyan());

    let client = AnthropicClient::new()
        .await
        .context("failed to initialize Anthropic client")?;

    println!("  Model: {}", "claude-sonnet-4-5-20250929".dimmed());
    println!();

    println!("{} Sending test request...", "→".cyan());

    let start = std::time::Instant::now();
    let response = client
        .complete(
            "You are a helpful assistant. Respond with exactly one short sentence.",
            "Say hello and confirm you're working.",
        )
        .await
        .context("API request failed")?;
    let elapsed = start.elapsed();

    println!();
    println!("{} API connection successful!", "✓".green().bold());
    println!();
    println!("  Response time: {:?}", elapsed);
    println!("  Response: {}", response.trim().dimmed());
    println!();
    println!(
        "{}",
        "AI changelog generation is ready to use with 'belaf prepare'.".green()
    );

    Ok(())
}
