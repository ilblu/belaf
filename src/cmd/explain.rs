//! `belaf explain` — print attribution per ReleaseUnit.
//!
//! Phase K of `BELAF_MASTER_PLAN.md`. Reads the config file,
//! resolves all `[[release_unit]]` + `[[release_unit_glob]]` entries,
//! and prints provenance for each one (which detector / TOML line /
//! glob expansion produced it). Useful for debugging unexpected
//! configs.

use anyhow::{Context, Result};
use owo_colors::OwoColorize;

use crate::core::config::ConfigurationFile;
use crate::core::git::repository::Repository;
use crate::core::release_unit::{detector, resolver::resolve, ResolveOrigin, VersionSource};

pub fn run() -> Result<i32> {
    let repo = Repository::open_from_env()
        .context("belaf is not being run from a Git working directory")?;

    let mut cfg_path = repo.resolve_config_dir();
    cfg_path.push("config.toml");
    let cfg = ConfigurationFile::get(&cfg_path)
        .with_context(|| format!("failed to load config at {}", cfg_path.display()))?;

    let resolved = resolve(&repo, &cfg.release_units, &cfg.release_unit_globs)
        .map_err(|e| anyhow::anyhow!("release_unit resolution: {e}"))?;

    println!();
    println!("{}", "Belaf — config explain".bold());
    println!();

    if resolved.is_empty() {
        println!(
            "{}",
            "No [[release_unit]] / [[release_unit_glob]] entries in this repo.".yellow()
        );
        println!(
            "  (Auto-detected ecosystem-loader projects don't appear here — \
             only ReleaseUnits do.)"
        );
        return Ok(0);
    }

    println!(
        "{} {} ReleaseUnits",
        "Detected:".green().bold(),
        resolved.len()
    );
    println!();

    for r in &resolved {
        let origin_label = match &r.origin {
            ResolveOrigin::Explicit { config_index } => {
                format!("explicit [[release_unit]] #{}", config_index)
                    .cyan()
                    .to_string()
            }
            ResolveOrigin::Glob {
                glob_index,
                matched_path,
            } => format!(
                "glob [[release_unit_glob]] #{} matched {}",
                glob_index,
                matched_path.escaped()
            )
            .magenta()
            .to_string(),
            ResolveOrigin::Detected { detector } => {
                format!("auto-detected by `{detector}`").blue().to_string()
            }
        };

        let source_label = match &r.unit.source {
            VersionSource::Manifests(ms) => {
                if ms.len() == 1 {
                    format!("Manifests([{}])", ms[0].path.escaped())
                } else {
                    let paths: Vec<_> = ms.iter().map(|m| m.path.escaped().to_string()).collect();
                    format!("Manifests({} files: {})", ms.len(), paths.join(", "))
                }
            }
            VersionSource::External(ext) => format!("External(tool={})", ext.tool),
        };

        println!("  {} {}", "•".green(), r.unit.name.bold());
        println!("    origin    : {}", origin_label);
        println!("    ecosystem : {:?}", r.unit.ecosystem);
        println!("    source    : {}", source_label);
        if !r.unit.satellites.is_empty() {
            let sats: Vec<_> = r
                .unit
                .satellites
                .iter()
                .map(|s| s.escaped().to_string())
                .collect();
            println!("    satellites: {}", sats.join(", "));
        }
        if let Some(tf) = &r.unit.tag_format {
            println!("    tag_format: {}", tf);
        }
        if r.unit.visibility != crate::core::release_unit::Visibility::Public {
            println!("    visibility: {}", r.unit.visibility.wire_key().yellow());
        }
        if let Some(c) = &r.unit.cascade_from {
            println!("    cascade   : from `{}` ({:?})", c.source.cyan(), c.bump);
        }
        println!();
    }

    // Drift snapshot — useful in the same view.
    let drift = detector::detect_drift(&repo, &resolved, &cfg);
    if !drift.is_empty() {
        println!("{}", "⚠️  Drift detected:".yellow().bold());
        for hit in &drift.uncovered {
            println!("    - {}", hit.path.escaped());
        }
        println!();
    }

    if !cfg.ignore_paths.paths.is_empty() {
        println!(
            "{} {}",
            "[ignore_paths]:".dimmed(),
            cfg.ignore_paths.paths.join(", ")
        );
    }
    if !cfg.allow_uncovered.paths.is_empty() {
        println!(
            "{} {}",
            "[allow_uncovered]:".dimmed(),
            cfg.allow_uncovered.paths.join(", ")
        );
    }

    Ok(0)
}
