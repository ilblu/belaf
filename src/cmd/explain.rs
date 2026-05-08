//! `belaf explain` — print attribution per ReleaseUnit.
//!
//! Reads the config file, resolves all `[release_unit.<name>]`
//! entries (explicit + glob-form), and prints provenance for each
//! one (detector / TOML key / glob expansion).
//!
//! `--format=json` emits a structured payload consumed by the
//! github-app dashboard. The JSON shape is a stable contract —
//! additions land as new optional fields, removals require a major
//! bump.

use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use serde::Serialize;

use crate::cli::ExplainOutputFormat;
use crate::core::config::ConfigurationFile;
use crate::core::git::repository::Repository;
use crate::core::release_unit::{detector, resolver::resolve, ResolveOrigin, VersionSource};

#[derive(Serialize)]
struct ExplainPayload {
    units: Vec<ExplainUnit>,
    drift: ExplainDrift,
    ignore_paths: Vec<String>,
    allow_uncovered: Vec<String>,
}

#[derive(Serialize)]
struct ExplainUnit {
    name: String,
    ecosystem: String,
    origin: ExplainOrigin,
    source: ExplainSource,
    satellites: Vec<String>,
    tag_format: Option<String>,
    visibility: String,
    cascade_from: Option<ExplainCascade>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ExplainOrigin {
    Explicit {
        config_index: usize,
    },
    Glob {
        glob_index: usize,
        matched_path: String,
    },
    PartialOverride {
        config_index: usize,
    },
    Detected {
        detector: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ExplainSource {
    Manifests { paths: Vec<String> },
    External { tool: String },
}

#[derive(Serialize)]
struct ExplainCascade {
    source: String,
    bump: String,
}

#[derive(Serialize, Default)]
struct ExplainDrift {
    uncovered_paths: Vec<String>,
}

pub fn run(format: Option<ExplainOutputFormat>) -> Result<i32> {
    let repo = Repository::open_from_env()
        .context("belaf is not being run from a Git working directory")?;

    let mut cfg_path = repo.resolve_config_dir();
    cfg_path.push("config.toml");
    let cfg = ConfigurationFile::get(&cfg_path)
        .with_context(|| format!("failed to load config at {}", cfg_path.display()))?;

    // `belaf explain` only shows resolved units sourced from the config
    // (explicit + glob). Partial overrides require a discovery pass to
    // synthesize their resolved forms; that's session-level work and
    // would change this command's semantics. We surface only what the
    // resolver could produce statically — the partial-override count
    // is reported separately.
    let resolve_output = resolve(&repo, &cfg.release_units)
        .map_err(|e| anyhow::anyhow!("release_unit resolution: {e}"))?;
    let resolved = resolve_output.resolved;

    let json_mode = matches!(format, Some(ExplainOutputFormat::Json));

    if json_mode {
        let payload = build_json_payload(&repo, &resolved, &cfg);
        let json = serde_json::to_string_pretty(&payload).context("serialise explain payload")?;
        println!("{}", json);
        return Ok(0);
    }

    println!();
    println!("{}", "Belaf — config explain".bold());
    println!();

    if resolved.is_empty() {
        println!(
            "{}",
            "No [release_unit.<name>] entries in this repo.".yellow()
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
                format!("explicit [release_unit] #{}", config_index)
                    .cyan()
                    .to_string()
            }
            ResolveOrigin::Glob {
                glob_index,
                matched_path,
            } => format!(
                "glob [release_unit] #{} matched {}",
                glob_index,
                matched_path.escaped()
            )
            .magenta()
            .to_string(),
            ResolveOrigin::PartialOverride { config_index } => format!(
                "partial-override [release_unit] #{} (decorates auto-detected)",
                config_index
            )
            .yellow()
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

fn build_json_payload(
    repo: &Repository,
    resolved: &[crate::core::release_unit::ResolvedReleaseUnit],
    cfg: &ConfigurationFile,
) -> ExplainPayload {
    let units = resolved
        .iter()
        .map(|r| ExplainUnit {
            name: r.unit.name.clone(),
            ecosystem: format!("{:?}", r.unit.ecosystem),
            origin: match &r.origin {
                ResolveOrigin::Explicit { config_index } => ExplainOrigin::Explicit {
                    config_index: *config_index,
                },
                ResolveOrigin::Glob {
                    glob_index,
                    matched_path,
                } => ExplainOrigin::Glob {
                    glob_index: *glob_index,
                    matched_path: matched_path.escaped().to_string(),
                },
                ResolveOrigin::PartialOverride { config_index } => ExplainOrigin::PartialOverride {
                    config_index: *config_index,
                },
                ResolveOrigin::Detected { detector } => ExplainOrigin::Detected {
                    detector: detector.to_string(),
                },
            },
            source: match &r.unit.source {
                VersionSource::Manifests(ms) => ExplainSource::Manifests {
                    paths: ms.iter().map(|m| m.path.escaped().to_string()).collect(),
                },
                VersionSource::External(ext) => ExplainSource::External {
                    tool: ext.tool.clone(),
                },
            },
            satellites: r
                .unit
                .satellites
                .iter()
                .map(|s| s.escaped().to_string())
                .collect(),
            tag_format: r.unit.tag_format.clone(),
            visibility: r.unit.visibility.wire_key().to_string(),
            cascade_from: r.unit.cascade_from.as_ref().map(|c| ExplainCascade {
                source: c.source.clone(),
                bump: format!("{:?}", c.bump),
            }),
        })
        .collect();

    let drift_report = detector::detect_drift(repo, resolved, cfg);
    let drift = ExplainDrift {
        uncovered_paths: drift_report
            .uncovered
            .iter()
            .map(|h| h.path.escaped().to_string())
            .collect(),
    };

    ExplainPayload {
        units,
        drift,
        ignore_paths: cfg.ignore_paths.paths.clone(),
        allow_uncovered: cfg.allow_uncovered.paths.clone(),
    }
}
