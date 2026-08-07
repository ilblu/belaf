//! Resolution pipeline: takes the TOML-parsed [`ReleaseUnitConfig`]
//! entries (one per `[release_unit.<name>]` block) and produces a
//! `Vec<ResolvedReleaseUnit>` ready for the rest of the release
//! pipeline.
//!
//! Each entry is either explicit (`glob` field unset) or glob-form
//! (`glob` field set). The dispatch is centralised in [`resolve`].
//!
//! Cascade cycle detection lives in the bump pass — see
//! [`crate::core::bump`].
//!
//! The stages live in submodules so this file stays the pipeline and
//! nothing else: [`glob`] expands glob-form blocks and owns the template
//! engine, [`manifests`] builds [`ManifestFile`]s and validates
//! `version_field`, [`parse`] holds the leaf-value parsers shared by every
//! path, and [`checks`] holds the whole-set validations.

use std::collections::HashMap;

use crate::core::config::NamedReleaseUnitConfig;
use crate::core::ecosystem::format_handler::DiscoveredUnit;
use crate::core::git::repository::{RepoPathBuf, Repository};

use super::syntax::{ManifestList, ReleaseUnitConfig};
use super::validator::ResolverError;
use super::{
    BaselineSpec, CascadeRule, ExternalVersioner, ManifestFile, ReleaseUnit, ResolveOrigin,
    ResolvedReleaseUnit, VersionFieldSpec, VersionSource, Visibility,
};

mod checks;
mod glob;
mod manifests;
mod parse;

pub use manifests::{default_manifest_filename_for_ecosystem, default_version_field_for_ecosystem};

use checks::{detect_name_collisions, detect_nested_bundles, unit_paths, validate_cascade_sources};
use glob::expand_glob;
use manifests::build_manifests;
use parse::{
    parse_baseline, parse_cascade_rule, parse_ecosystem, parse_kind, parse_repo_path,
    parse_visibility,
};

/// Output of [`resolve`]. Carries the fully-resolved units (explicit
/// blocks + glob expansions) and any **partial-override** specs that
/// still need to be matched against auto-detected units later in the
/// pipeline (see [`resolve_partial_against_discovered`]).
#[derive(Debug)]
pub struct ResolveOutput {
    pub resolved: Vec<ResolvedReleaseUnit>,
    pub partial_overrides: Vec<PartialOverrideSpec>,
}

/// A `[release_unit.<name>]` block with no `ecosystem` field — it
/// decorates an auto-detected unit with the same name. Validated at
/// [`resolve`] time (no structural fields, at least one override
/// field present); resolved against the discovered set later.
#[derive(Clone, Debug)]
pub struct PartialOverrideSpec {
    pub name: String,
    pub config_index: usize,
    pub tag_format: Option<String>,
    pub visibility: Option<Visibility>,
    pub satellites: Vec<RepoPathBuf>,
    pub cascade_from: Option<CascadeRule>,
    /// `kind` override (F1) — lets a bare `[release_unit.<name>] kind = "..."`
    /// reclassify an auto-detected unit as internal/ignore. `None` = leave the
    /// discovered unit's default (`Deploy`).
    pub kind: Option<crate::core::resolved_release_unit::UnitKind>,

    /// Per-unit bump-policy override (F11a) decorating an auto-detected unit.
    pub bump_override: Option<super::syntax::BumpOverrideConfig>,

    /// Per-unit history baseline decorating an auto-detected unit. This is
    /// the common shape: a freshly detected unit that has never been
    /// released gets a bare `[release_unit.<name>] baseline = "..."` block
    /// (which is exactly what `belaf baseline --fix` writes).
    pub baseline: Option<BaselineSpec>,
}

/// Public API: resolve the parsed config into a list of
/// `ResolvedReleaseUnit`s, validating along the way. Each input entry
/// is one of:
///
/// - **explicit** (no `glob`, has `ecosystem`) — converted directly
/// - **glob-form** (`glob` set) — expands to N units
/// - **partial override** (no `glob`, no `ecosystem`) — collected as
///   a [`PartialOverrideSpec`] in the returned [`ResolveOutput`]; the
///   session then matches it against the auto-detected unit set via
///   [`resolve_partial_against_discovered`].
pub fn resolve(
    repo: &Repository,
    units: &[NamedReleaseUnitConfig],
) -> Result<ResolveOutput, ResolverError> {
    let mut resolved: Vec<ResolvedReleaseUnit> = Vec::new();
    let mut partial_overrides: Vec<PartialOverrideSpec> = Vec::new();

    // Step 1: explicit entries — straight conversion. Partial-override
    // blocks (no `ecosystem`, not glob) are validated and collected
    // separately.
    for (idx, named) in units.iter().enumerate() {
        if named.config.is_glob() {
            continue;
        }
        if named.config.is_partial_override() {
            let spec = validate_partial_override(idx, &named.name, &named.config)?;
            partial_overrides.push(spec);
            continue;
        }
        let ecosystem_str = match named.config.ecosystem.as_deref() {
            Some(e) => e,
            // A manifest-less `paths = [...]` unit (F1) needs no ecosystem —
            // it is never tagged/released, so the value is irrelevant. Default
            // to "cargo" so downstream parsing has a concrete value.
            None if !named.config.paths.is_empty() => "cargo",
            None => {
                return Err(ResolverError::SourceNotSet {
                    unit: named.name.clone(),
                })
            }
        };
        let unit = convert_explicit(&named.name, ecosystem_str, &named.config, repo)?;
        resolved.push(ResolvedReleaseUnit {
            unit,
            origin: ResolveOrigin::Explicit { config_index: idx },
        });
    }

    // Step 2: collect every path already covered by an explicit unit so
    // we can apply "explicit wins, glob skips that path". A glob
    // expansion is shadowed if its matched directory is a parent of
    // any explicit-covered path (or equals one).
    let mut explicit_covered_paths: Vec<String> = Vec::new();
    for r in &resolved {
        for path in unit_paths(&r.unit) {
            explicit_covered_paths.push(path);
        }
    }

    // Step 3: glob expansion. Track (path, glob_idx) pairs to detect
    // two-globs-same-path and two-globs-same-name collisions.
    let mut glob_path_owners: HashMap<String, (usize, String)> = HashMap::new();
    let mut glob_name_owners: HashMap<String, (usize, String)> = HashMap::new();

    for (glob_idx, named) in units.iter().enumerate() {
        if !named.config.is_glob() {
            continue;
        }
        let glob_pattern = named
            .config
            .glob
            .as_ref()
            .expect("is_glob() implies glob set");
        for resolved_glob in expand_glob(
            repo,
            glob_idx,
            &named.name,
            &named.config,
            &explicit_covered_paths,
        )? {
            let unit_path = match &resolved_glob.origin {
                ResolveOrigin::Glob { matched_path, .. } => matched_path.escaped(),
                _ => unreachable!("expand_glob returns only Glob-origin units"),
            };

            // Explicit wins; skip silently. The glob's matched_path is
            // a directory; a sibling explicit `[release_unit.<name>]`
            // can either point to that directory directly OR to a
            // manifest/satellite inside it.
            let glob_anchor_prefix = format!("{unit_path}/");
            let covered = explicit_covered_paths
                .iter()
                .any(|p| p == &unit_path || p.starts_with(&glob_anchor_prefix));
            if covered {
                continue;
            }

            // Two globs matching the same path.
            if let Some((prev_idx, prev_glob)) = glob_path_owners.get(&unit_path) {
                if *prev_idx != glob_idx {
                    return Err(ResolverError::TwoGlobsSamePath {
                        path: unit_path,
                        glob_a: prev_glob.clone(),
                        glob_b: glob_pattern.clone(),
                    });
                }
            }
            glob_path_owners.insert(unit_path.clone(), (glob_idx, glob_pattern.clone()));

            // Two globs producing the same name from different paths.
            let unit_name = resolved_glob.unit.name.clone();
            if let Some((prev_idx, _prev_path)) = glob_name_owners.get(&unit_name) {
                if *prev_idx != glob_idx {
                    return Err(ResolverError::TwoGlobsSameName {
                        glob_a: *prev_idx,
                        glob_b: glob_idx,
                        name: unit_name,
                        path: unit_path,
                    });
                }
            } else {
                glob_name_owners.insert(unit_name, (glob_idx, unit_path));
            }

            resolved.push(resolved_glob);
        }
    }

    // Step 4: cross-cutting validations on the full resolved set.
    detect_name_collisions(&resolved)?;
    detect_nested_bundles(&resolved)?;
    validate_cascade_sources(&resolved)?;

    Ok(ResolveOutput {
        resolved,
        partial_overrides,
    })
}

// ===========================================================================
// Partial-override validation + late resolution against discovered units.
// ===========================================================================

/// Validate a partial-override block: it must not set any structural
/// field (`manifests`, `external`, `version_field`, `fallback_manifests`,
/// `name`) and must set at least one override field.
fn validate_partial_override(
    config_index: usize,
    name: &str,
    cfg: &ReleaseUnitConfig,
) -> Result<PartialOverrideSpec, ResolverError> {
    debug_assert!(cfg.ecosystem.is_none() && !cfg.is_glob());

    let structural_field: Option<&'static str> = if cfg.manifests.is_some() {
        Some("manifests")
    } else if cfg.external.is_some() {
        Some("external")
    } else if cfg.version_field.is_some() {
        Some("version_field")
    } else if !cfg.fallback_manifests.is_empty() {
        Some("fallback_manifests")
    } else if cfg.name.is_some() {
        Some("name")
    } else if !cfg.paths.is_empty() {
        Some("paths")
    } else {
        None
    };
    if let Some(field) = structural_field {
        return Err(ResolverError::PartialOverrideStructuralField {
            unit: name.to_string(),
            field,
        });
    }

    let visibility = parse_visibility(name, cfg.visibility.as_deref())?;
    let cascade_from = match &cfg.cascade_from {
        Some(c) => Some(parse_cascade_rule(name, c)?),
        None => None,
    };
    let satellites = cfg
        .satellites
        .iter()
        .map(|s| parse_repo_path(name, s))
        .collect::<Result<Vec<_>, _>>()?;

    let kind = parse_kind(name, cfg.kind.as_deref())?;
    let baseline = parse_baseline(name, cfg.baseline.as_deref())?;

    let has_any_override = cfg.tag_format.is_some()
        || cfg.visibility.is_some()
        || !cfg.satellites.is_empty()
        || cfg.cascade_from.is_some()
        || cfg.kind.is_some()
        || cfg.bump.is_some()
        || cfg.baseline.is_some();
    if !has_any_override {
        return Err(ResolverError::PartialOverrideEmpty {
            unit: name.to_string(),
        });
    }

    Ok(PartialOverrideSpec {
        name: name.to_string(),
        config_index,
        tag_format: cfg.tag_format.clone(),
        // `visibility: Option` so default-vs-set is distinguishable.
        // `parse_visibility` returns `Visibility::default()` when raw is
        // None, which we map back to `None` here.
        visibility: cfg.visibility.as_deref().map(|_| visibility),
        satellites,
        cascade_from,
        kind: cfg.kind.as_deref().map(|_| kind),
        bump_override: cfg.bump.clone(),
        baseline,
    })
}

/// Match each [`PartialOverrideSpec`] against the auto-detected unit
/// with the same name and synthesize a [`ResolvedReleaseUnit`] whose
/// override fields take effect at workflow time. The synthesized
/// unit's `source` is informational — graph registration goes through
/// the discovered unit's already-built rewriters; the session must
/// **not** call `add_configured_unit_to_graph` for these (origin is
/// `PartialOverride`, which is the signal).
pub fn resolve_partial_against_discovered(
    overrides: &[PartialOverrideSpec],
    discovered: &[DiscoveredUnit],
) -> Result<Vec<ResolvedReleaseUnit>, ResolverError> {
    let mut out = Vec::with_capacity(overrides.len());
    for spec in overrides {
        let matched = discovered
            .iter()
            .find(|d| d.qnames.first().is_some_and(|n| n == &spec.name))
            .ok_or_else(|| ResolverError::PartialOverrideNoMatch {
                unit: spec.name.clone(),
            })?;

        let ecosystem_str = matched
            .qnames
            .get(1)
            .map(String::as_str)
            .unwrap_or("external");
        let ecosystem = parse_ecosystem(ecosystem_str);
        let version_field_key = default_version_field_for_ecosystem(ecosystem_str);
        let version_field = match version_field_key {
            "cargo_toml" => VersionFieldSpec::CargoToml,
            "npm_package_json" => VersionFieldSpec::NpmPackageJson,
            "tauri_conf_json" => VersionFieldSpec::TauriConfJson,
            "gradle_properties" => VersionFieldSpec::GradleProperties,
            "pep_621" => VersionFieldSpec::Pep621,
            // Fallback — for ecosystems whose default key isn't a
            // first-class spec (e.g. unknown). Use a no-op regex so
            // the informational source is well-formed; it's never
            // actually read because graph registration uses the
            // discovered rewriters.
            _ => VersionFieldSpec::GenericRegex {
                pattern: "(.+)".to_string(),
                replace: "{version}".to_string(),
            },
        };
        let manifest = ManifestFile {
            path: matched.anchor_manifest.clone(),
            ecosystem: ecosystem.clone(),
            version_field,
        };

        let unit = ReleaseUnit {
            name: spec.name.clone(),
            ecosystem,
            source: VersionSource::Manifests(vec![manifest]),
            satellites: spec.satellites.clone(),
            tag_format: spec.tag_format.clone(),
            visibility: spec.visibility.unwrap_or_default(),
            cascade_from: spec.cascade_from.clone(),
            kind: spec.kind.unwrap_or_default(),
            bump_override: spec.bump_override.clone(),
            baseline: spec.baseline.clone(),
        };

        out.push(ResolvedReleaseUnit {
            unit,
            origin: ResolveOrigin::PartialOverride {
                config_index: spec.config_index,
            },
        });
    }
    Ok(out)
}

// ===========================================================================
// Explicit conversion
// ===========================================================================

fn convert_explicit(
    name: &str,
    ecosystem_str: &str,
    cfg: &ReleaseUnitConfig,
    repo: &Repository,
) -> Result<ReleaseUnit, ResolverError> {
    debug_assert!(
        !cfg.is_glob(),
        "convert_explicit only handles non-glob entries"
    );

    // `name` field forbidden on non-glob entries — TOML key drives the
    // name. Validator surfaces a clear error.
    if cfg.name.is_some() {
        return Err(ResolverError::ExplicitUnitHasNameTemplate {
            unit: name.to_string(),
        });
    }
    // Glob-only fields must be unset.
    if !cfg.fallback_manifests.is_empty() || cfg.version_field.is_some() {
        return Err(ResolverError::ExplicitUnitHasGlobOnlyField {
            unit: name.to_string(),
        });
    }

    let ecosystem = parse_ecosystem(ecosystem_str);
    let kind = parse_kind(name, cfg.kind.as_deref())?;

    // Source: a manifest-less `paths = [...]` unit (F1 — internal/ignore
    // cascade nodes), OR exactly one of manifests / external.
    let paths_set = !cfg.paths.is_empty();
    let manifests_set =
        matches!(cfg.manifests, Some(ManifestList::Explicit(ref m)) if !m.is_empty());
    let templates_set = matches!(cfg.manifests, Some(ManifestList::Templates(_)));
    let external_set = cfg.external.is_some();

    if templates_set {
        // Glob-form `manifests = ["..."]` not allowed in non-glob entry.
        return Err(ResolverError::ExplicitUnitHasGlobOnlyField {
            unit: name.to_string(),
        });
    }

    if paths_set {
        if manifests_set || external_set {
            return Err(ResolverError::PathsOnlyInvalid {
                unit: name.to_string(),
                reason: "`paths = [...]` is mutually exclusive with `manifests`/`external`",
            });
        }
        if kind == crate::core::resolved_release_unit::UnitKind::Deploy {
            return Err(ResolverError::PathsOnlyInvalid {
                unit: name.to_string(),
                reason: "`paths = [...]` (manifest-less) requires `kind = \"internal\"` or `\"ignore\"` \
                         — a `deploy` unit must declare a `manifests`/`external` version source",
            });
        }
    }

    let source = match (paths_set, manifests_set, external_set) {
        // Manifest-less paths-only unit: just records the owned directories.
        (true, _, _) => {
            let paths = cfg
                .paths
                .iter()
                .map(|p| parse_repo_path(name, p))
                .collect::<Result<Vec<_>, _>>()?;
            VersionSource::PathsOnly(paths)
        }
        (false, true, true) => {
            return Err(ResolverError::SourceBothSet {
                unit: name.to_string(),
            });
        }
        (false, false, false) => {
            return Err(ResolverError::SourceNotSet {
                unit: name.to_string(),
            });
        }
        (false, true, false) => {
            let Some(ManifestList::Explicit(manifests_cfg)) = &cfg.manifests else {
                unreachable!("manifests_set implies Explicit");
            };
            let manifests = build_manifests(
                name,
                ecosystem_str,
                manifests_cfg,
                repo,
                /* require_existence: */ true,
            )?;
            VersionSource::Manifests(manifests)
        }
        (false, false, true) => {
            let ext_cfg = cfg.external.as_ref().unwrap();
            let cwd = match &ext_cfg.cwd {
                Some(s) => Some(parse_repo_path(name, s)?),
                None => None,
            };
            VersionSource::External(ExternalVersioner {
                tool: ext_cfg.tool.clone(),
                read_command: ext_cfg.read_command.clone(),
                write_command: ext_cfg.write_command.clone(),
                cwd,
                timeout_sec: ext_cfg.timeout_sec,
                env: ext_cfg.env.clone(),
            })
        }
    };

    let satellites = cfg
        .satellites
        .iter()
        .map(|s| parse_repo_path(name, s))
        .collect::<Result<Vec<_>, _>>()?;

    let visibility = parse_visibility(name, cfg.visibility.as_deref())?;
    let cascade_from = match &cfg.cascade_from {
        Some(c) => Some(parse_cascade_rule(name, c)?),
        None => None,
    };
    let baseline = parse_baseline(name, cfg.baseline.as_deref())?;

    Ok(ReleaseUnit {
        name: name.to_string(),
        ecosystem,
        source,
        satellites,
        tag_format: cfg.tag_format.clone(),
        visibility,
        cascade_from,
        kind,
        bump_override: cfg.bump.clone(),
        baseline,
    })
}
