use crate::cli::Cli;
use clap::CommandFactory;
use clap_complete::{generate as gen_completions, Shell};

pub fn generate(shell: Shell) {
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    gen_completions(shell, &mut cmd, bin_name, &mut std::io::stdout());
}

pub fn print_version() {
    let version = env!("CARGO_PKG_VERSION");
    println!("belaf {}", version);
}
