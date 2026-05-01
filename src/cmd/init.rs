// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Boostrapping Belaf on a preexisting repository.

use anyhow::{bail, Context};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io::Write};
use tracing::{error, info, warn};

use crate::atry;
use crate::core::{
    errors::{Error, Result},
    resolved_release_unit::DepRequirement,
    session::AppBuilder,
};

/// The toplevel bootstrap state structure.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BootstrapConfiguration {
    pub project: Vec<BootstrapProjectInfo>,
}

/// Bootstrap info for a project.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BootstrapProjectInfo {
    pub qnames: Vec<String>,
    pub version: String,
    pub release_commit: Option<String>,
}

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
mod toml_util;
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
    let exit = cmd.execute()?;

    // 3.0/Wave 1d: auto-detect is the default behaviour in --ci mode
    // (it always was for interactive wizards). The legacy
    // `--auto-detect` opt-in flag is preserved on the CLI surface for
    // backward compat but no longer gates anything — the run-detectors
    // path is on by default, and `--no-auto-detect` is the opt-out
    // (TODO: clap flag rename in a follow-up; for now `auto_detect_flag`
    // is read but always treated as on).
    let _ = auto_detect_flag; // kept for future opt-out semantics
    if exit == 0 {
        run_auto_detect()?;
    }
    Ok(exit)
}

fn run_auto_detect() -> Result<()> {
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

    Ok(())
}

impl BootstrapCommand {
    fn execute(self) -> Result<i32> {
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
            let proj = sess.graph().lookup(ident);

            if !seen_any {
                info!("belaf detected the following projects in the repo:");
                println!();
                seen_any = true;
            }

            let loc_desc = {
                let p = proj.prefix();

                if p.is_empty() {
                    "the root directory".to_owned()
                } else {
                    format!("`{}`", p.escaped())
                }
            };

            println!(
                "    {} @ {} in {}",
                proj.user_facing_name, proj.version, loc_desc
            );
        }

        if seen_any {
            println!();
            info!("consult the documentation if these results are unexpected");
            info!("autodetection letting you down? file an issue: https://github.com/ilblu/belaf/issues/new");
        } else {
            error!("belaf failed to discover any projects in the repo");
            error!("autodetection letting you down? file an issue: https://github.com/ilblu/belaf/issues/new");
            return Ok(1);
        }

        let mut bs_cfg = BootstrapConfiguration::default();
        let mut versions = HashMap::new();

        let topo_ids: Vec<_> = sess.graph().toposorted().collect();
        for ident in topo_ids {
            let proj = sess.graph_mut().lookup_mut(ident);
            bs_cfg.project.push(BootstrapProjectInfo {
                qnames: proj.qualified_names().to_owned(),
                version: proj.version.to_string(),
                release_commit: None,
            });

            versions.insert(proj.ident(), proj.version.clone());

            for dep in &mut proj.internal_deps[..] {
                dep.belaf_requirement = DepRequirement::Manual(versions[&dep.ident].to_string());
            }
        }

        let bs_text = atry!(
            toml::to_string_pretty(&bs_cfg);
            ["could not serialize bootstrap data into TOML format"]
        );

        {
            let mut bs_path = repo.resolve_config_dir();
            bs_path.push("bootstrap.toml");
            info!("writing versioning bootstrap file `{}`", bs_path.display());

            let f = match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&bs_path)
            {
                Ok(f) => Some(f),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        warn!(
                            "belaf bootstrap file `{}` already exists; not modifying it",
                            bs_path.display()
                        );
                        None
                    } else {
                        return Err(Error::new(e).context(format!(
                            "failed to open belaf bootstrap file `{}` for writing",
                            bs_path.display()
                        )));
                    }
                }
            };

            if let Some(mut f) = f {
                atry!(
                    f.write_all(bs_text.as_bytes());
                    ["could not write bootstrap file `{}`", bs_path.display()]
                );
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
                println!();
                seen_any = true;
            }

            println!("    {}", path.escaped());
        }

        if seen_any {
            println!();
        } else {
            info!("... no files modified. This might be OK.")
        }

        let topo_ids: Vec<_> = sess.graph().toposorted().collect();
        for ident in topo_ids {
            let proj = sess.graph_mut().lookup_mut(ident);
            for dep in &mut proj.internal_deps[..] {
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
        println!();
        info!("Review changes, add `belaf/` to the repository, and commit.");
        info!("Then try `belaf status` for a history summary");
        info!("   (Note: commit tracking starts from package-specific tags or 'belaf-baseline')");
        info!("Then begin modifying your CI/CD pipeline to use the `belaf release` commands");
        Ok(0)
    }
}
