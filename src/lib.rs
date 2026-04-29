pub mod cli;
pub mod error;

pub mod cmd {
    pub mod changelog;
    pub mod completions;
    pub mod dashboard;
    pub mod explain;
    pub mod graph;
    pub mod init;
    pub mod install;
    pub mod prepare;
    pub mod status;
}

pub mod core {
    pub mod wire;

    pub mod bump;
    pub mod bump_source;
    pub mod cargo_lock;
    pub mod config;
    pub mod embed;
    pub mod env;
    pub mod errors;
    pub mod graph;
    pub mod group;
    pub mod manifest;
    pub mod project;
    pub mod release_unit;
    pub mod rewriters;
    pub mod session;
    pub mod tag_format;
    pub mod version;
    pub mod version_field;
    pub mod workflow;

    pub mod api;

    pub mod auth {
        pub mod token;
    }

    pub mod git {
        pub mod branch;
        pub mod gitignore;
        pub mod repository;
        pub mod url;
        pub mod utils;
        pub mod validate;
    }

    pub mod ecosystem {
        pub mod cargo;
        #[cfg(feature = "csharp")]
        pub mod csproj;
        pub mod elixir;
        pub mod go;
        pub mod maven;
        pub mod npm;
        pub mod pypa;
        pub mod registry;
        pub mod swift;
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
        Commands::Install => {
            let exit_code = cmd::install::run().await?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
        Commands::Auth(auth_cmd) => match auth_cmd {
            AuthCommands::Status => {
                let exit_code = cmd::install::status().await?;
                if exit_code != 0 {
                    std::process::exit(exit_code);
                }
                Ok(())
            }
            AuthCommands::Whoami => {
                let exit_code = cmd::install::whoami().await?;
                if exit_code != 0 {
                    std::process::exit(exit_code);
                }
                Ok(())
            }
            AuthCommands::Logout => {
                let exit_code = cmd::install::logout().await?;
                if exit_code != 0 {
                    std::process::exit(exit_code);
                }
                Ok(())
            }
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
            let exit_code = cmd::prepare::run(
                args.ci,
                args.project,
                args.bump_source,
                args.bump_source_cmd,
            )?;
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
        Commands::Explain => {
            let exit_code = cmd::explain::run()?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
    }
}
