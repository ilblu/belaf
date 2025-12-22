pub mod cli;
pub mod error;

pub mod cmd {
    pub mod auth;
    pub mod changelog;
    pub mod completions;
    pub mod graph;
    pub mod init;
    pub mod prepare;
    pub mod status;
}

pub mod core {
    pub mod root;

    pub mod bump;
    pub mod config;
    pub mod embed;
    pub mod env;
    pub mod errors;
    pub mod graph;
    pub mod manifest;
    pub mod project;
    pub mod rewriters;
    pub mod session;
    pub mod version;
    pub mod workflow;

    pub mod auth {
        pub mod github;
        pub mod token;
    }

    pub mod git {
        pub mod branch;
        pub mod gitignore;
        pub mod repository;
        pub mod utils;
        pub mod validate;
    }

    pub mod ecosystem {
        pub mod cargo;
        #[cfg(feature = "csharp")]
        pub mod csproj;
        pub mod elixir;
        pub mod go;
        pub mod npm;
        pub mod pypa;
        pub mod swift;
        pub mod types;
    }

    pub mod github {
        pub mod client;
        pub mod pr;
    }

    pub mod changelog;

    pub mod ui;
}

pub mod utils {
    pub mod file_io;
    pub mod theme;
    pub mod version_check;
}

use anyhow::Result;
use cli::{AuthCommands, Cli, Commands};

pub async fn execute(cli: Cli) -> Result<()> {
    let command = cli.command.expect("Command must be present");
    match command {
        Commands::Auth(auth_cmd) => match auth_cmd {
            AuthCommands::Login { no_browser } => cmd::auth::login(no_browser).await,
            AuthCommands::Logout => cmd::auth::logout().await,
            AuthCommands::Status => cmd::auth::status().await,
        },
        Commands::Completions { shell } => {
            cmd::completions::generate(shell);
            Ok(())
        }
        Commands::Version => {
            cmd::completions::print_version();
            Ok(())
        }
        Commands::Init(args) => {
            let exit_code = cmd::init::run(args.force, args.upstream, args.ci, args.preset)?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
        Commands::Status(args) => {
            let exit_code = cmd::status::run(args.format, args.ci)?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
        Commands::Prepare(args) => {
            let exit_code = cmd::prepare::run(args.ci, args.project)?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
        Commands::Graph(args) => {
            let exit_code = cmd::graph::run(args.format, args.ci, args.web, args.out)?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
        Commands::Changelog(args) => {
            let exit_code = cmd::changelog::run(
                args.preview,
                args.stdout,
                args.project,
                args.output,
                args.unreleased,
                args.ci,
            )?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
    }
}
