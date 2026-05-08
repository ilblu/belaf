//! `belaf describe --json` — emit a machine-readable map of the CLI
//! surface so AI agents can discover what belaf can do without
//! parsing 12 `--help` outputs.
//!
//! The output is the canonical answer to "what is belaf and how do I
//! drive it from a script?" — every command, every argument, the
//! relevant environment variables, the stable exit codes, and the
//! list of embedded JSON schemas.
//!
//! Walks `clap::Command` for the command/argument tree (so it can
//! never drift from what the binary actually accepts) and pairs that
//! with hand-curated workflow examples that a static walk cannot
//! produce.

use anyhow::{Context, Result};
use clap::CommandFactory;
use serde::Serialize;

use crate::cli::Cli;
use crate::cmd::schema::AVAILABLE_SCHEMAS;
use crate::core::exit_code::ExitCode;

#[derive(Serialize)]
struct DescribeOutput {
    name: String,
    version: String,
    about: String,
    long_about: Option<String>,
    commands: Vec<CommandDoc>,
    env_vars: Vec<EnvVarDoc>,
    exit_codes: Vec<ExitCodeDoc>,
    schemas: Vec<SchemaDoc>,
    example_workflows: Vec<WorkflowDoc>,
}

#[derive(Serialize)]
struct CommandDoc {
    name: String,
    about: Option<String>,
    long_about: Option<String>,
    /// Full path from the root, e.g. `["auth", "status"]` for
    /// `belaf auth status`. Empty for the root.
    path: Vec<String>,
    args: Vec<ArgDoc>,
    subcommands: Vec<CommandDoc>,
    /// Convenience flags for agents:
    ci_supported: bool,
    format_supported: bool,
}

#[derive(Serialize)]
struct ArgDoc {
    name: String,
    short: Option<char>,
    long: Option<String>,
    help: Option<String>,
    required: bool,
    takes_value: bool,
    default: Option<String>,
    /// For value-enum args, the allowed values.
    possible_values: Vec<String>,
}

#[derive(Serialize)]
struct EnvVarDoc {
    name: &'static str,
    purpose: &'static str,
}

#[derive(Serialize)]
struct ExitCodeDoc {
    code: i32,
    label: &'static str,
    description: &'static str,
}

#[derive(Serialize)]
struct SchemaDoc {
    name: &'static str,
    description: &'static str,
}

#[derive(Serialize)]
struct WorkflowDoc {
    name: &'static str,
    description: &'static str,
    steps: &'static [&'static str],
}

const ENV_VARS: &[EnvVarDoc] = &[
    EnvVarDoc {
        name: "BELAF_API_URL",
        purpose: "Override the belaf API endpoint (default https://api.belaf.dev). Used by auth, update check, and the install command.",
    },
    EnvVarDoc {
        name: "BELAF_WEB_URL",
        purpose: "Override the dashboard URL opened from TUI menus.",
    },
    EnvVarDoc {
        name: "BELAF_NO_KEYRING",
        purpose: "Set to `1` to disable the OS keyring. Required in headless / test environments where the keyring crate hangs.",
    },
    EnvVarDoc {
        name: "RUST_LOG",
        purpose: "Standard tracing filter. CLI verbosity flags (-v / -vv / -vvv) override this.",
    },
    EnvVarDoc {
        name: "CI",
        purpose: "Auto-detected. When set (and on common CI providers), prompts are suppressed and `--ci` mode is implied for some commands.",
    },
    EnvVarDoc {
        name: "GITHUB_ACTIONS",
        purpose: "Auto-detected (alongside GITLAB_CI etc). Same effect as CI.",
    },
    EnvVarDoc {
        name: "ACTIONS_ID_TOKEN_REQUEST_URL",
        purpose: "Set automatically by GitHub Actions when the job has `permissions: id-token: write`. belaf falls back to OIDC token exchange via /api/cli/auth/oidc/exchange when the keyring is empty.",
    },
    EnvVarDoc {
        name: "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
        purpose: "Companion to ACTIONS_ID_TOKEN_REQUEST_URL — the OIDC bearer token GitHub Actions injects into the job.",
    },
];

const WORKFLOWS: &[WorkflowDoc] = &[
    WorkflowDoc {
        name: "ci-release",
        description:
            "Non-interactive release flow for CI. Runs prepare in --ci mode; the GitHub App finalises tags and Releases when the resulting PR merges.",
        steps: &[
            "belaf prepare --ci",
            "# (resulting PR is reviewed and merged manually or by automation)",
        ],
    },
    WorkflowDoc {
        name: "inspect-config",
        description:
            "Understand which release units belaf sees in the current repository, which detector or config block produced each, and surface drift.",
        steps: &[
            "belaf explain --format=json",
            "belaf graph --format=json",
        ],
    },
    WorkflowDoc {
        name: "preview-changelog",
        description:
            "Preview the changelog that `belaf prepare` would emit for the next release without writing files or opening a PR.",
        steps: &[
            "belaf changelog --preview",
        ],
    },
    WorkflowDoc {
        name: "agent-discovery",
        description:
            "First call an AI agent should make on a fresh repo to understand the belaf surface.",
        steps: &[
            "belaf describe --json",
            "belaf schema manifest  # if you plan to consume manifests",
        ],
    },
];

pub fn run(text: bool) -> Result<i32> {
    let cmd = Cli::command();
    let output = build_output(&cmd);

    if text {
        render_text(&output);
    } else {
        let json = serde_json::to_string_pretty(&output).context("serialise describe payload")?;
        println!("{}", json);
    }

    Ok(0)
}

fn build_output(root: &clap::Command) -> DescribeOutput {
    DescribeOutput {
        name: root.get_name().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        about: root.get_about().map(|s| s.to_string()).unwrap_or_default(),
        long_about: root.get_long_about().map(|s| s.to_string()),
        commands: collect_subcommands(root, &[]),
        env_vars: ENV_VARS
            .iter()
            .map(|e| EnvVarDoc {
                name: e.name,
                purpose: e.purpose,
            })
            .collect(),
        exit_codes: ExitCode::all()
            .iter()
            .map(|c| ExitCodeDoc {
                code: (*c).into(),
                label: c.label(),
                description: c.description(),
            })
            .collect(),
        schemas: AVAILABLE_SCHEMAS
            .iter()
            .map(|(name, description)| SchemaDoc { name, description })
            .collect(),
        example_workflows: WORKFLOWS
            .iter()
            .map(|w| WorkflowDoc {
                name: w.name,
                description: w.description,
                steps: w.steps,
            })
            .collect(),
    }
}

fn collect_subcommands(parent: &clap::Command, parent_path: &[String]) -> Vec<CommandDoc> {
    parent
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
        .map(|c| document_command(c, parent_path))
        .collect()
}

fn document_command(cmd: &clap::Command, parent_path: &[String]) -> CommandDoc {
    let name = cmd.get_name().to_string();
    let mut path: Vec<String> = parent_path.to_vec();
    path.push(name.clone());

    let args: Vec<ArgDoc> = cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set())
        .map(|a| ArgDoc {
            name: a.get_id().to_string(),
            short: a.get_short(),
            long: a.get_long().map(|s| s.to_string()),
            help: a.get_help().map(|s| s.to_string()),
            required: a.is_required_set(),
            takes_value: matches!(
                a.get_action(),
                clap::ArgAction::Set | clap::ArgAction::Append
            ),
            default: a
                .get_default_values()
                .first()
                .map(|s| s.to_string_lossy().to_string()),
            possible_values: a
                .get_possible_values()
                .iter()
                .map(|p| p.get_name().to_string())
                .collect(),
        })
        .collect();

    let ci_supported = args.iter().any(|a| a.long.as_deref() == Some("ci"));
    let format_supported = args.iter().any(|a| a.long.as_deref() == Some("format"));

    let subcommands = collect_subcommands(cmd, &path);

    CommandDoc {
        name,
        about: cmd.get_about().map(|s| s.to_string()),
        long_about: cmd.get_long_about().map(|s| s.to_string()),
        path,
        args,
        subcommands,
        ci_supported,
        format_supported,
    }
}

fn render_text(out: &DescribeOutput) {
    println!("{} {}", out.name, out.version);
    println!("{}", out.about);
    println!();

    println!("COMMANDS");
    for c in &out.commands {
        render_command_text(c, 0);
    }
    println!();

    println!("EXIT CODES");
    for ec in &out.exit_codes {
        println!("  {:>2}  {:<14}  {}", ec.code, ec.label, ec.description);
    }
    println!();

    println!("ENV VARS");
    for ev in &out.env_vars {
        println!("  {}", ev.name);
        println!("      {}", ev.purpose);
    }
    println!();

    println!("EMBEDDED SCHEMAS");
    for s in &out.schemas {
        println!("  {:<10} {}", s.name, s.description);
    }
    println!("  (run `belaf schema <name>` to print one)");
    println!();

    println!("EXAMPLE WORKFLOWS");
    for w in &out.example_workflows {
        println!("  {}: {}", w.name, w.description);
        for s in w.steps {
            println!("    $ {}", s);
        }
    }
}

fn render_command_text(cmd: &CommandDoc, depth: usize) {
    let indent = "  ".repeat(depth + 1);
    println!(
        "{}{}{}{}",
        indent,
        cmd.name,
        if cmd.ci_supported { "  [--ci]" } else { "" },
        if cmd.format_supported {
            "  [--format=json]"
        } else {
            ""
        },
    );
    if let Some(about) = &cmd.about {
        println!("{}    {}", indent, about);
    }
    for sub in &cmd.subcommands {
        render_command_text(sub, depth + 1);
    }
}
