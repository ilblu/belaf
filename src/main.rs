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
        print_version_info();
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
        eprintln!("Error: No command provided. Use --help for usage information.");
        std::process::exit(1);
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

fn print_version_info() {
    let version = env!("CARGO_PKG_VERSION");
    let description = env!("CARGO_PKG_DESCRIPTION");
    let repository = env!("CARGO_PKG_REPOSITORY");
    let target = env!("TARGET");
    let rustc_version = env!("RUSTC_VERSION");

    println!(
        "{} {}",
        "belaf".cyan().bold(),
        version.green().bold()
    );
    println!("{}", description.dimmed());
    println!();
    println!("{:<12} {}", "Repository:".dimmed(), repository);
    println!("{:<12} {}", "Target:".dimmed(), target);
    println!("{:<12} {}", "Rustc:".dimmed(), rustc_version);
}
