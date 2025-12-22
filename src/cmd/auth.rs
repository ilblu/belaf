use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::auth::{github, token};
use crate::utils::theme::{header, highlight, step_message, success_message, url};

pub async fn login(no_browser: bool) -> Result<()> {
    let client_id = "Ov23liuSrRXBZ7PDX61o";
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
    println!("{}", step_message("Validating permissions..."));

    let scopes = github::get_token_scopes(&access_token).await?;
    let missing_scopes: Vec<&str> = github::REQUIRED_SCOPES
        .iter()
        .filter(|&required| !scopes.iter().any(|s| s == *required))
        .copied()
        .collect();

    if !missing_scopes.is_empty() {
        println!();
        println!(
            "{} Authorization incomplete - missing required permissions!",
            "✗".red().bold()
        );
        println!();
        println!("  Required: {}", missing_scopes.join(", ").yellow());
        println!(
            "  Granted:  {}",
            if scopes.is_empty() {
                "(none)".to_string()
            } else {
                scopes.join(", ")
            }
            .dimmed()
        );
        println!();
        println!("This happens when you previously authorized belaf without granting");
        println!("repository access. To fix this:");
        println!();
        println!("  1. Revoke the existing authorization:");
        println!("     {}", github::get_revoke_url().cyan());
        println!();
        println!("  2. Run {} again", "belaf auth login".cyan());
        println!();
        println!(
            "  3. When prompted, make sure to {} repository access",
            "grant".green().bold()
        );
        println!();
        return Err(anyhow::anyhow!(
            "Token missing required scopes: {}",
            missing_scopes.join(", ")
        ));
    }

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

    if !scopes.is_empty() {
        println!("  Permissions: {}", scopes.join(", ").dimmed());
    }

    Ok(())
}

pub async fn logout() -> Result<()> {
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

pub async fn status() -> Result<()> {
    println!();
    println!("{}", "Authentication Status".bold());
    println!("{}", "=====================".dimmed());
    println!();
    println!("{}", "GitHub".bold());
    println!("{}", "------".dimmed());

    match token::load_token() {
        Ok(token_val) => match github::get_username(&token_val).await {
            Ok(username) => {
                println!("  {} Logged in as {}", "✓".green(), username.cyan());

                match github::get_token_scopes(&token_val).await {
                    Ok(scopes) => {
                        let has_repo = scopes.iter().any(|s| s == "repo");
                        if scopes.is_empty() {
                            println!(
                                "  {} No permissions granted - run {} to fix",
                                "!".yellow(),
                                "belaf auth login".cyan()
                            );
                        } else if !has_repo {
                            println!("  Permissions: {}", scopes.join(", ").dimmed());
                            println!(
                                "  {} Missing 'repo' permission - releases will fail",
                                "!".yellow()
                            );
                            println!("  Revoke at: {}", github::get_revoke_url().dimmed());
                        } else {
                            println!("  Permissions: {}", scopes.join(", ").dimmed());
                        }
                    }
                    Err(_) => {
                        println!("  {} Could not verify permissions", "!".yellow());
                    }
                }
            }
            Err(_) => {
                println!("  {} Token exists but may be invalid", "!".yellow());
                println!("  Run {} to re-authenticate", "belaf auth login".cyan());
            }
        },
        Err(_) => {
            println!("  {} Not logged in", "✗".red());
            println!("  Run {} to authenticate", "belaf auth login".cyan());
        }
    }

    Ok(())
}
