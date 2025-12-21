use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "belaf",
    about = "Release management CLI for monorepos",
    long_about = "A powerful CLI tool for semantic versioning and release management.\nSupports Rust, Node.js, Python, Go, Elixir, Swift, and C# projects.",
    version,
    after_help = "For detailed command help, run: belaf <COMMAND> --help"
)]
#[command(disable_version_flag = true)]
pub struct Cli {
    #[arg(
        short,
        long,
        action = clap::ArgAction::Count,
        global = true,
        help = "Increase logging verbosity (-v, -vv, -vvv)"
    )]
    pub verbose: u8,

    #[arg(long, global = true, help = "Disable colored output")]
    pub no_color: bool,

    #[arg(short = 'V', long, help = "Print version information")]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(subcommand, about = "Authentication management")]
    Auth(AuthCommands),

    #[command(about = "Generate shell completions")]
    Completions {
        #[arg(value_enum, help = "Shell type to generate completions for")]
        shell: clap_complete::Shell,
    },

    #[command(about = "Print version information")]
    Version,

    #[command(
        about = "Initialize release management",
        long_about = "Initialize release management in your repository.\n\nThis command:\n  • Detects all projects (Rust, Node.js, Python, Go, Elixir, Swift, C#)\n  • Creates .belaf/release.toml configuration\n  • Analyzes project dependencies and builds dependency graph\n  • Sets up changelog tracking\n\nRequires a clean Git working directory unless --force is used."
    )]
    Init(InitArgs),

    #[command(
        about = "Show release status and changelog",
        long_about = "Display current release status and preview upcoming changes.\n\nShows:\n  • Projects with uncommitted changes\n  • Projects ready for release\n  • Dependency order for releases\n  • Preview of changelog entries based on Git commits\n\nUse this before 'prepare' to verify what will be released."
    )]
    Status(StatusArgs),

    #[command(
        about = "Prepare a release (bump versions)",
        long_about = "Prepare a new release by bumping versions and updating changelogs.\n\nBump types:\n  • major: Breaking changes (1.0.0 → 2.0.0)\n  • minor: New features (1.0.0 → 1.1.0)\n  • patch: Bug fixes (1.0.0 → 1.0.1)\n  • auto: Automatic bump based on conventional commits\n  • manual: Interactive TUI wizard with suggestions\n\nThis command:\n  • Creates a release branch\n  • Updates version numbers in all affected project files\n  • Generates/updates CHANGELOG.md for each project\n  • Creates a release manifest\n  • Commits, pushes, and creates a Pull Request\n\nModes:\n  • TUI mode (default): Interactive 4-step wizard with auto-suggestions\n  • Auto mode (--no-tui): Automatic bump based on conventional commits\n  • CI mode (--ci): Full automation with PR creation"
    )]
    Prepare(PrepareArgs),

    #[command(
        about = "Show project dependency graph",
        long_about = "Display the project dependency graph.\n\nInteractive TUI mode (default):\n  • Navigate through projects with arrow keys\n  • View dependency details\n  • Visual dependency tree\n\nBrowser mode (--web):\n  • Interactive Cytoscape.js graph\n  • Multiple layouts (Hierarchy, Force, Circle)\n  • Search, zoom, export PNG\n\nNon-interactive mode (--no-tui):\n  • ASCII art graph\n  • DOT format for Graphviz\n  • JSON for programmatic use"
    )]
    Graph(GraphArgs),

    #[command(
        about = "Generate changelog from commits",
        long_about = "Generate changelog entries based on conventional commits.\n\nThis command generates changelogs without the full release workflow.\nUseful for previewing changes or generating changelogs as a separate step.\n\nModes:\n  • Default: Write changelog files to disk\n  • Preview (--preview): Show changelog without writing files\n  • Stdout (--stdout): Output to stdout instead of files\n\nExamples:\n  belaf changelog                    # Generate all changelogs\n  belaf changelog --preview          # Preview without writing\n  belaf changelog --project mylib    # Only for specific project\n  belaf changelog --stdout           # Output to terminal"
    )]
    Changelog(ChangelogArgs),
}

#[derive(Subcommand)]
pub enum AuthCommands {
    #[command(
        about = "Authenticate with GitHub",
        long_about = "Authenticate with GitHub for PR creation and release management.\n\nCredentials are stored securely in the system keychain."
    )]
    Login {
        #[arg(long, help = "Skip opening browser and show URL only")]
        no_browser: bool,
    },

    #[command(about = "Sign out from GitHub")]
    Logout,

    #[command(about = "Show authentication status")]
    Status,
}

#[derive(Args)]
pub struct InitArgs {
    #[arg(short, long, help = "Force operation even in unexpected conditions")]
    pub force: bool,

    #[arg(short, long, help = "The name of the Git upstream remote")]
    pub upstream: Option<String>,

    #[arg(long, help = "Skip interactive TUI, use automatic detection")]
    pub no_tui: bool,
}

#[derive(Args)]
pub struct StatusArgs {
    #[arg(
        short,
        long,
        value_enum,
        default_value = "table",
        help = "Output format"
    )]
    pub format: Option<ReleaseOutputFormat>,

    #[arg(long, help = "Force text mode even in TTY")]
    pub no_tui: bool,
}

#[derive(Args)]
pub struct PrepareArgs {
    #[arg(
        long,
        help = "CI/CD mode: auto-bump, changelog, commit, push, and PR creation"
    )]
    pub ci: bool,

    #[arg(
        short,
        long,
        value_delimiter = ',',
        help = "Override bump for specific projects (e.g., gate:major,core:minor)"
    )]
    pub project: Option<Vec<String>>,
}

#[derive(Args)]
pub struct GraphArgs {
    #[arg(short, long, value_enum, help = "Output format (only with --no-tui)")]
    pub format: Option<GraphOutputFormat>,

    #[arg(long, help = "Skip interactive TUI, output static graph")]
    pub no_tui: bool,

    #[arg(long, short, help = "Open interactive graph in web browser")]
    pub web: bool,

    #[arg(long, short, help = "Save HTML graph to file (implies --web)")]
    pub out: Option<String>,
}

#[derive(Args)]
pub struct ChangelogArgs {
    #[arg(long, help = "Preview changelog without writing files")]
    pub preview: bool,

    #[arg(long, help = "Output changelog to stdout instead of files")]
    pub stdout: bool,

    #[arg(short, long, help = "Generate changelog only for specific project")]
    pub project: Option<String>,

    #[arg(short, long, help = "Custom output file path (overrides config)")]
    pub output: Option<String>,

    #[arg(long, help = "Include unreleased changes (no version tag)")]
    pub unreleased: bool,
}

#[derive(Clone, ValueEnum)]
pub enum GraphOutputFormat {
    #[value(help = "ASCII art graph")]
    Ascii,
    #[value(help = "DOT format (for Graphviz)")]
    Dot,
    #[value(help = "JSON format")]
    Json,
}

#[derive(Clone, ValueEnum)]
pub enum ReleaseOutputFormat {
    #[value(help = "Interactive table (TUI mode)")]
    Table,
    #[value(help = "Plain text output")]
    Text,
    #[value(help = "JSON output")]
    Json,
}
