// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Boostrapping Belaf on a preexisting repository.

use anyhow::{bail, Context};
use clap::Parser;
use std::{collections::HashMap, fs, io::Write};
use tracing::{error, info, warn};

use crate::atry;
use crate::core::{
    errors::{Error, Result},
    resolved_release_unit::DepRequirement,
    session::AppBuilder,
};

/// The `bootstrap` commands.
#[derive(Debug, Eq, PartialEq, Parser)]
pub struct BootstrapCommand {
    #[arg(
        short = 'f',
        long = "force",
        help = "Force operation even in unexpected conditions"
    )]
    force: bool,

    #[arg(
        short = 'u',
        long = "upstream",
        help = "The name of the Git upstream remote"
    )]
    upstream_name: Option<String>,

    #[arg(long = "preset", help = "Use a preset configuration template")]
    preset: Option<String>,
}

// `pub` only because the `tests/test_clikd_shape.rs` and
// `tests/test_explain_clikd.rs` integration tests call `auto_detect::run`
// directly to verify snippet emission against fixtures. Keeping it
// crate-private would force those tests to go through the wizard's
// runtime entry point, which needs a live tty.
pub mod auto_detect;
pub(crate) mod toml_util;
mod wizard;

pub fn run(
    force: bool,
    upstream: Option<String>,
    ci: bool,
    preset: Option<String>,
    auto_detect_flag: bool,
) -> Result<i32> {
    use crate::core::embed::EmbeddedPresets;
    use crate::core::ui::utils::is_interactive_terminal;

    if let Some(ref preset_name) = preset {
        let valid_presets = EmbeddedPresets::list_presets();
        if !valid_presets.contains(&preset_name.to_string()) {
            bail!(
                "Unknown preset '{}'. Valid presets: {}",
                preset_name,
                valid_presets.join(", ")
            );
        }
    }

    if !ci && is_interactive_terminal() {
        return wizard::run(force, upstream, preset);
    }

    let cmd = BootstrapCommand {
        force,
        upstream_name: upstream,
        preset,
    };
    let exit = cmd.execute(ci)?;

    let _ = auto_detect_flag; // accepted for backward compat; auto-detect is always on in --ci.
    if exit == 0 {
        let summary = run_auto_detect()?;
        if ci {
            emit_init_ci_status(&summary);
        }
    }
    Ok(exit)
}

/// Counters captured from the post-bootstrap auto-detect pass. Threaded
/// out of [`run_auto_detect`] so [`run`] can emit them in the `--ci`
/// JSON status without re-opening the session.
struct InitSummary {
    config_path: String,
    release_units_detected: usize,
    ecosystems: Vec<String>,
}

#[derive(serde::Serialize)]
struct InitCiStatus<'a> {
    /// Stable label. Currently always `initialized` — `already_initialized`
    /// would require detecting an existing config-file marker, which the
    /// bootstrap does not check today (it appends instead).
    status: &'static str,
    config_path: &'a str,
    release_units_detected: usize,
    ecosystems: &'a [String],
}

fn emit_init_ci_status(summary: &InitSummary) {
    let payload = InitCiStatus {
        status: "initialized",
        config_path: &summary.config_path,
        release_units_detected: summary.release_units_detected,
        ecosystems: &summary.ecosystems,
    };
    match serde_json::to_string_pretty(&payload) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error: failed to serialise --ci status: {e}"),
    }
}

fn run_auto_detect() -> Result<InitSummary> {
    let repo = crate::core::git::repository::Repository::open_from_env()
        .context("auto-detect: belaf is not in a Git working directory")?;

    let result = auto_detect::run(&repo);

    let mut cfg_path = repo.resolve_config_dir();
    cfg_path.push("config.toml");
    auto_detect::append_to_config(&cfg_path, &result.toml_snippet)
        .with_context(|| format!("auto-detect: failed to append to {}", cfg_path.display()))?;

    info!(
        "auto-detect: {} ReleaseUnit candidates ({} hexagonal cargo, {} tauri, {} jvm-library, {} sdk-cascade), {} mobile-app warnings → [allow_uncovered]",
        result.counters.total_release_unit_candidates(),
        result.counters.hexagonal_cargo,
        result.counters.tauri_single_source + result.counters.tauri_legacy,
        result.counters.jvm_library,
        result.counters.sdk_cascade_member,
        result.counters.total_mobile_warnings(),
    );

    // Re-walk the discovery surface for the JSON-status caller.
    // Cheap on small repos; on large monorepos this is the same walk
    // the session already does, but we deliberately don't reuse the
    // session here to keep the helper self-contained.
    use crate::core::ecosystem::format_handler::{
        FormatHandlerRegistry, WorkspaceDiscovererRegistry,
    };
    use crate::core::release_unit::discovery::discover_implicit_release_units;
    let handlers = FormatHandlerRegistry::with_defaults();
    let discoverers = WorkspaceDiscovererRegistry::with_defaults();
    let units =
        discover_implicit_release_units(&repo, &handlers, &discoverers, &[]).unwrap_or_default();
    let mut ecosystems: Vec<String> = units
        .iter()
        .filter_map(|u| u.qnames.get(1).cloned())
        .collect();
    ecosystems.sort();
    ecosystems.dedup();

    Ok(InitSummary {
        config_path: cfg_path.display().to_string(),
        release_units_detected: units.len(),
        ecosystems,
    })
}

impl BootstrapCommand {
    fn execute(self, ci: bool) -> Result<i32> {
        // In `--ci` mode, decorative human output goes to stderr so
        // stdout is reserved for the final JSON status line emitted by
        // [`emit_init_ci_status`]. Outside `--ci`, the same content
        // lands on stdout (today's behavior — preserved for human
        // users running `belaf init` non-interactively without `--ci`).
        macro_rules! out {
            () => { if ci { eprintln!() } else { println!() } };
            ($($arg:tt)*) => { if ci { eprintln!($($arg)*) } else { println!($($arg)*) } };
        }

        info!(
            "bootstrapping with belaf version {}",
            env!("CARGO_PKG_VERSION")
        );

        let mut repo = atry!(
            crate::core::git::repository::Repository::open_from_env();
            ["belaf is not being run from a Git working directory"]
            (note "run the bootstrap stage inside the Git work tree that you wish to bootstrap")
        );

        let upstream_url = atry!(
            repo.bootstrap_upstream(self.upstream_name.as_ref().map(|s| s.as_ref()));
            ["belaf cannot identify the Git upstream URL"]
            (note "use the `--upstream` option to manually identify the upstream Git remote")
        );

        info!("the Git upstream URL is: {}", upstream_url);

        if let Some(dirty) = atry!(
            repo.check_if_dirty(&[]);
            ["failed to check the repository for modified files"]
        ) {
            warn!(
                "bootstrapping with uncommitted changes in the repository (e.g.: `{}`)",
                dirty.escaped()
            );
            if !self.force {
                let mut report = crate::core::errors::AnnotatedReport::default();
                report.set_message("refusing to proceed".to_string());
                report.add_note(
                    "pass `--force` to override, or commit/stash your changes first".to_string(),
                );
                return Err(Error::new(report));
            }
        }

        {
            let embedded_config = match &self.preset {
                Some(preset_name) => {
                    info!("using preset configuration: {}", preset_name);
                    atry!(
                        crate::core::embed::EmbeddedPresets::get_preset_string(preset_name);
                        ["could not load preset configuration '{}'. Available presets: {}",
                         preset_name,
                         crate::core::embed::EmbeddedPresets::list_presets().join(", ")]
                    )
                }
                None => {
                    atry!(
                        crate::core::embed::EmbeddedConfig::get_config_string();
                        ["could not load embedded default configuration"]
                        (note "this is a packaging bug — please report at https://github.com/ilblu/belaf/issues")
                    )
                }
            };
            let cfg_text = embedded_config.replace(
                "upstream_urls = []",
                &format!("upstream_urls = [\"{}\"]", upstream_url),
            );

            let mut cfg_path = repo.resolve_config_dir();
            atry!(
                fs::create_dir_all(&cfg_path);
                ["could not create belaf configuration directory `{}`", cfg_path.display()]
            );

            cfg_path.push("config.toml");
            info!("writing belaf configuration file `{}`", cfg_path.display());

            let f = match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&cfg_path)
            {
                Ok(f) => Some(f),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        warn!(
                            "belaf configuration file `{}` already exists; not modifying it",
                            cfg_path.display()
                        );
                        None
                    } else {
                        return Err(Error::new(e).context(format!(
                            "failed to open belaf configuration file `{}` for writing",
                            cfg_path.display()
                        )));
                    }
                }
            };

            if let Some(mut f) = f {
                atry!(
                    f.write_all(cfg_text.as_bytes());
                    ["could not write belaf configuration file `{}`", cfg_path.display()]
                );
            }
        }

        let mut sess = AppBuilder::new()?.with_progress(true).initialize()?;

        let mut seen_any = false;

        for ident in sess.graph().toposorted() {
            let unit = sess.graph().lookup(ident);

            if !seen_any {
                info!("belaf detected the following projects in the repo:");
                out!();
                seen_any = true;
            }

            let loc_desc = {
                let p = unit.prefix();

                if p.is_empty() {
                    "the root directory".to_owned()
                } else {
                    format!("`{}`", p.escaped())
                }
            };

            out!(
                "    {} @ {} in {}",
                unit.user_facing_name,
                unit.version,
                loc_desc
            );
        }

        if seen_any {
            out!();
            info!("consult the documentation if these results are unexpected");
            info!("autodetection letting you down? file an issue: https://github.com/ilblu/belaf/issues/new");
        } else {
            error!("belaf failed to discover any projects in the repo");
            error!("autodetection letting you down? file an issue: https://github.com/ilblu/belaf/issues/new");
            return Ok(1);
        }

        // Convert each project's internal_deps from `Commit(...)` to
        // `Manual(version)` so the rewrite step has concrete versions
        // to write into the manifest files.
        let mut versions = HashMap::new();
        let topo_ids: Vec<_> = sess.graph().toposorted().collect();
        for ident in topo_ids {
            let unit = sess.graph_mut().lookup_mut(ident);
            versions.insert(unit.ident(), unit.version.clone());
            for dep in &mut unit.internal_deps[..] {
                dep.belaf_requirement = DepRequirement::Manual(versions[&dep.ident].to_string());
            }
        }

        info!("updating project meta-files with developer versions");

        let changes = match sess.rewrite() {
            Ok(c) => c,
            Err(e) => {
                error!("rewrite error: {:?}", e);
                return Err(e.context("there was a problem updating the project files"));
            }
        };

        let mut seen_any = false;

        for path in changes.paths() {
            if !seen_any {
                info!("modified:");
                out!();
                seen_any = true;
            }

            out!("    {}", path.escaped());
        }

        if seen_any {
            out!();
        } else {
            info!("... no files modified. This might be OK.")
        }

        let topo_ids: Vec<_> = sess.graph().toposorted().collect();
        for ident in topo_ids {
            let unit = sess.graph_mut().lookup_mut(ident);
            for dep in &mut unit.internal_deps[..] {
                dep.belaf_requirement = DepRequirement::Manual(dep.literal.clone());
            }
        }

        atry!(
            sess.rewrite_belaf_requirements();
            ["there was a problem adding dependency metadata to the project files"]
        );

        info!("creating baseline tag for commit tracking");
        atry!(
            repo.create_baseline_tag();
            ["failed to create baseline tag"]
            (note "ensure your Git config has both `user.email` and `user.name` set")
        );

        info!("modifications complete!");
        out!();
        info!("Review changes, add `belaf/` to the repository, and commit.");
        info!("Then try `belaf status` for a history summary");
        info!("   (Note: commit tracking starts from package-specific tags or 'belaf-baseline')");
        info!("Then begin modifying your CI/CD pipeline to use the `belaf release` commands");
        Ok(0)
    }
}
