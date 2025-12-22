use anyhow::Result;
use clap::Parser;
use owo_colors::OwoColorize;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = belaf::cli::Cli::parse();
    init_logging(cli.verbose);

    if cli.no_color {
        owo_colors::set_override(false);
    }

    if cli.version {
        belaf::cmd::completions::print_version();
        belaf::utils::version_check::check_for_updates(env!("CARGO_PKG_VERSION"), true);
        return Ok(());
    }

    if let Some(command) = cli.command {
        let is_completions = matches!(command, belaf::cli::Commands::Completions { .. });

        let should_pre_execute = !matches!(command, belaf::cli::Commands::Completions { .. });

        if should_pre_execute {
            belaf::core::root::pre_execute();
        }

        let res = belaf::execute(belaf::cli::Cli {
            verbose: cli.verbose,
            no_color: cli.no_color,
            version: false,
            command: Some(command),
        })
        .await;

        if !is_completions {
            belaf::utils::version_check::check_for_updates(env!("CARGO_PKG_VERSION"), false);
        }

        if let Err(e) = res {
            print_error(&e);
            std::process::exit(1);
        }
    } else {
        match belaf::cmd::dashboard::run() {
            Ok(action) => {
                use belaf::cmd::dashboard::DashboardAction;
                match action {
                    DashboardAction::Prepare => {
                        belaf::core::root::pre_execute();
                        let exit_code = belaf::cmd::prepare::run(false, None)?;
                        if exit_code != 0 {
                            std::process::exit(exit_code);
                        }
                    }
                    DashboardAction::Status => {
                        belaf::core::root::pre_execute();
                        let exit_code = belaf::cmd::status::run(None, false)?;
                        if exit_code != 0 {
                            std::process::exit(exit_code);
                        }
                    }
                    DashboardAction::Graph => {
                        belaf::core::root::pre_execute();
                        let exit_code = belaf::cmd::graph::run(None, false, false, None)?;
                        if exit_code != 0 {
                            std::process::exit(exit_code);
                        }
                    }
                    DashboardAction::Changelog => {
                        belaf::core::root::pre_execute();
                        let exit_code = belaf::cmd::changelog::run(false, false, None, None, false, false)?;
                        if exit_code != 0 {
                            std::process::exit(exit_code);
                        }
                    }
                    DashboardAction::Init => {
                        belaf::core::root::pre_execute();
                        let exit_code = belaf::cmd::init::run(false, None, false, None)?;
                        if exit_code != 0 {
                            std::process::exit(exit_code);
                        }
                    }
                    DashboardAction::Web => {
                        let url = std::env::var("BELAF_WEB_URL")
                            .unwrap_or_else(|_| "https://belaf.dev/dashboard".to_string());
                        if let Err(e) = open::that(&url) {
                            eprintln!("Failed to open browser: {}", e);
                            std::process::exit(1);
                        }
                    }
                    DashboardAction::Help => {
                        belaf::cli::Cli::parse_from(["belaf", "--help"]);
                    }
                    DashboardAction::Quit | DashboardAction::None => {}
                }
            }
            Err(e) => {
                print_error(&e);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn print_error(error: &anyhow::Error) {
    eprintln!("{} {}", "Error:".red().bold(), error);
}

fn init_logging(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(level))
        .init();
}
