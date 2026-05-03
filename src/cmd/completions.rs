use crate::cli::Cli;
use clap::CommandFactory;
use clap_complete::{generate as gen_completions, Shell};
use owo_colors::OwoColorize;

pub fn generate(shell: Shell) {
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    gen_completions(shell, &mut cmd, bin_name, &mut std::io::stdout());
}

pub fn print_version() {
    let version = env!("CARGO_PKG_VERSION");
    let description = env!("CARGO_PKG_DESCRIPTION");
    let repository = env!("CARGO_PKG_REPOSITORY");
    let target = env!("TARGET");
    let rustc_version = env!("RUSTC_VERSION");

    println!("{} {}", "belaf".cyan().bold(), version.green().bold());
    println!("{}", description.dimmed());
    println!();
    println!("{:<12} {}", "Repository:".dimmed(), repository);
    println!("{:<12} {}", "Target:".dimmed(), target);
    println!("{:<12} {}", "Rustc:".dimmed(), rustc_version);
}
