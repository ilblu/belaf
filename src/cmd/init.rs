// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Boostrapping Belaf on a preexisting repository.

use anyhow::bail;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io::Write};
use tracing::{error, info, warn};

use crate::atry;
use crate::core::release::{
    errors::{Error, Result},
    project::DepRequirement,
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
}

mod wizard;

pub fn run(force: bool, upstream: Option<String>, no_tui: bool) -> Result<i32> {
    use crate::core::ui::utils::is_interactive_terminal;

    if !no_tui && is_interactive_terminal() {
        return wizard::run(force, upstream);
    }

    let cmd = BootstrapCommand {
        force,
        upstream_name: upstream,
    };
    cmd.execute()
}

impl BootstrapCommand {
    fn execute(self) -> Result<i32> {
        info!(
            "bootstrapping with belaf version {}",
            env!("CARGO_PKG_VERSION")
        );

        let mut repo = atry!(
            crate::core::release::repository::Repository::open_from_env();
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
                bail!("refusing to proceed (use `--force` to override)");
            }
        }

        {
            let embedded_config = atry!(
                crate::core::release::embed::EmbeddedConfig::get_config_string();
                ["could not load embedded default configuration"]
            );
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

        let mut sess = atry!(
            AppBuilder::new()?.with_progress(true).initialize();
            ["could not initialize app and project graph"]
        );

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
