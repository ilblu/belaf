//! Resolution pipeline: takes the TOML-parsed
//! [`ExplicitReleaseUnitConfig`] / [`GlobReleaseUnitConfig`] entries and
//! produces a `Vec<ResolvedReleaseUnit>` ready for the rest of the
//! release pipeline.
//!
//! Phase B of `BELAF_MASTER_PLAN.md`. Handles:
//!
//! - Glob expansion via the `glob` crate
//! - Template substitution (`{path}`, `{basename}`, `{parent}`)
//! - `fallback_manifests` first-existing-wins resolution
//! - Validation per Part VI's 21 edge cases
//! - Conflict resolution per §2.6 (explicit > glob, two-globs-same-path
//!   error, nested-bundle error, name collision error)
//!
//! Cascade cycle detection (edge case 19) lives in the bump pass, not
//! here — see [`crate::core::bump`] (Phase G).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use crate::core::git::repository::{RepoPathBuf, Repository};
use crate::core::wire::known::Ecosystem;

use super::syntax::{
    CascadeRuleConfig, ExplicitReleaseUnitConfig, GlobReleaseUnitConfig, ManifestFileConfig,
};
use super::validator::ResolverError;
use super::{
    CascadeBumpStrategy, CascadeRule, ExternalVersioner, ManifestFile, ReleaseUnit, ResolveOrigin,
    ResolvedReleaseUnit, VersionFieldSpec, VersionSource, Visibility,
};

/// Public API: resolve the parsed config into a list of
/// `ResolvedReleaseUnit`s, validating along the way.
pub fn resolve(
    repo: &Repository,
    explicit: &[ExplicitReleaseUnitConfig],
    globs: &[GlobReleaseUnitConfig],
) -> Result<Vec<ResolvedReleaseUnit>, ResolverError> {
    let mut resolved: Vec<ResolvedReleaseUnit> = Vec::new();

    // Step 1: explicit entries — straight conversion.
    for (idx, cfg) in explicit.iter().enumerate() {
        let unit = convert_explicit(cfg, repo)?;
        resolved.push(ResolvedReleaseUnit {
            unit,
            origin: ResolveOrigin::Explicit { config_index: idx },
        });
    }

    // Step 2: collect every path already covered by an explicit unit so
    // we can apply edge case 7 (explicit wins, glob skips that path).
    // We collect the *manifest file paths* and *satellite paths* of every
    // explicit unit; a glob expansion is shadowed if its matched
    // directory is a parent of any of those paths (or equals one).
    let mut explicit_covered_paths: Vec<String> = Vec::new();
    for r in &resolved {
        for path in unit_paths(&r.unit) {
            explicit_covered_paths.push(path);
        }
    }

    // Step 3: glob expansion. Each glob may produce N units, one per
    // matching directory. Track which (path, glob_idx) pairs already
    // emitted to detect edge case 8 (two globs same path) and edge
    // case 21 (two globs produce same name).
    let mut glob_path_owners: HashMap<String, (usize, String)> = HashMap::new();
    let mut glob_name_owners: HashMap<String, (usize, String)> = HashMap::new();

    for (glob_idx, glob_cfg) in globs.iter().enumerate() {
        for resolved_glob in expand_glob(repo, glob_idx, glob_cfg)? {
            let unit_path = match &resolved_glob.origin {
                ResolveOrigin::Glob { matched_path, .. } => matched_path.escaped(),
                _ => unreachable!("expand_glob returns only Glob-origin units"),
            };

            // Edge case 7 — explicit wins; skip silently. The glob's
            // matched_path is a directory; a sibling explicit
            // [[release_unit]] can either point to that directory
            // directly OR to a manifest/satellite inside it.
            let glob_anchor_prefix = format!("{unit_path}/");
            let covered = explicit_covered_paths
                .iter()
                .any(|p| p == &unit_path || p.starts_with(&glob_anchor_prefix));
            if covered {
                continue;
            }

            // Edge case 8 — two globs match same path.
            if let Some((prev_idx, prev_glob)) = glob_path_owners.get(&unit_path) {
                if *prev_idx != glob_idx {
                    return Err(ResolverError::TwoGlobsSamePath {
                        path: unit_path,
                        glob_a: prev_glob.clone(),
                        glob_b: glob_cfg.glob.clone(),
                    });
                }
            }
            glob_path_owners.insert(unit_path.clone(), (glob_idx, glob_cfg.glob.clone()));

            // Edge case "two globs produce same name from different
            // paths" (a sub-case of edge 20). Detect across globs.
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

    Ok(resolved)
}

// ===========================================================================
// Explicit conversion
// ===========================================================================

fn convert_explicit(
    cfg: &ExplicitReleaseUnitConfig,
    repo: &Repository,
) -> Result<ReleaseUnit, ResolverError> {
    let ecosystem = parse_ecosystem(&cfg.ecosystem);

    // Source: exactly one of manifests / external must be set.
    let manifests_set = !cfg.source.manifests.is_empty();
    let external_set = cfg.source.external.is_some();

    let source = match (manifests_set, external_set) {
        (true, true) => {
            return Err(ResolverError::SourceBothSet {
                unit: cfg.name.clone(),
            })
        }
        (false, false) => {
            return Err(ResolverError::SourceNotSet {
                unit: cfg.name.clone(),
            })
        }
        (true, false) => {
            let manifests = build_manifests(
                &cfg.name,
                &cfg.ecosystem,
                &cfg.source.manifests,
                repo,
                /* require_existence: */ true,
            )?;
            VersionSource::Manifests(manifests)
        }
        (false, true) => {
            let ext_cfg = cfg.source.external.as_ref().unwrap();
            let cwd = match &ext_cfg.cwd {
                Some(s) => Some(parse_repo_path(&cfg.name, s)?),
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
        .map(|s| parse_repo_path(&cfg.name, s))
        .collect::<Result<Vec<_>, _>>()?;

    let visibility = parse_visibility(&cfg.name, cfg.visibility.as_deref())?;
    let cascade_from = match &cfg.cascade_from {
        Some(c) => Some(parse_cascade_rule(&cfg.name, c)?),
        None => None,
    };

    Ok(ReleaseUnit {
        name: cfg.name.clone(),
        ecosystem,
        source,
        satellites,
        tag_format: cfg.tag_format.clone(),
        visibility,
        cascade_from,
    })
}

// ===========================================================================
// Glob expansion
// ===========================================================================

fn expand_glob(
    repo: &Repository,
    glob_idx: usize,
    cfg: &GlobReleaseUnitConfig,
) -> Result<Vec<ResolvedReleaseUnit>, ResolverError> {
    // Templates we accept anywhere — pre-validate the glob pattern
    // itself for unknown vars. (The `glob` field itself is NOT
    // template-substituted — it's the source of `{path}` for the rest.)
    validate_template_vars_known(glob_idx, &cfg.glob)?;

    let workdir_repopath = RepoPathBuf::new(b"");
    let workdir = repo
        .resolve_workdir(&workdir_repopath)
        .canonicalize()
        .map_err(|e| ResolverError::InvalidPath {
            unit: format!("[[release_unit_glob #{glob_idx}]]"),
            path: cfg.glob.clone(),
            reason: format!("repo workdir canonicalize failed: {e}"),
        })?;

    let pattern_abs = workdir.join(&cfg.glob);
    let pattern_str = pattern_abs.to_string_lossy().to_string();

    let mut units = Vec::new();
    let entries = match glob::glob(&pattern_str) {
        Ok(e) => e,
        Err(err) => {
            return Err(ResolverError::InvalidPath {
                unit: format!("[[release_unit_glob #{glob_idx}]]"),
                path: cfg.glob.clone(),
                reason: format!("invalid glob pattern: {err}"),
            });
        }
    };

    for entry in entries.flatten() {
        if !entry.is_dir() {
            // Glob form expands to directories only.
            continue;
        }

        let matched_repopath =
            repo.convert_path(&entry)
                .map_err(|e| ResolverError::InvalidPath {
                    unit: format!("[[release_unit_glob #{glob_idx}]]"),
                    path: entry.display().to_string(),
                    reason: format!("convert_path: {e}"),
                })?;

        let ctx = TemplateCtx::from_matched_path(&matched_repopath);

        // Apply templates to every templated field.
        let unit_name = substitute(glob_idx, &cfg.name, &ctx)?;
        let manifests_paths_templated: Vec<String> = cfg
            .manifests
            .iter()
            .map(|m| substitute(glob_idx, m, &ctx))
            .collect::<Result<Vec<_>, _>>()?;
        let fallback_paths_templated: Vec<String> = cfg
            .fallback_manifests
            .iter()
            .map(|m| substitute(glob_idx, m, &ctx))
            .collect::<Result<Vec<_>, _>>()?;
        let satellites_templated: Vec<String> = cfg
            .satellites
            .iter()
            .map(|s| substitute(glob_idx, s, &ctx))
            .collect::<Result<Vec<_>, _>>()?;

        // Resolve manifests via first-existing-wins.
        let chosen_manifest = pick_first_existing(
            &unit_name,
            &manifests_paths_templated,
            &fallback_paths_templated,
            repo,
        )?;

        // Build ManifestFile entries. For glob form, version_field is
        // either explicitly set or derived from the ecosystem.
        let version_field_key = match &cfg.version_field {
            Some(s) => s.clone(),
            None => default_version_field_for_ecosystem(&cfg.ecosystem).to_string(),
        };

        let manifests = build_manifests(
            &unit_name,
            &cfg.ecosystem,
            &[ManifestFileConfig {
                path: chosen_manifest,
                ecosystem: None,
                version_field: version_field_key,
                regex_pattern: None,
                regex_replace: None,
            }],
            repo,
            /* require_existence: */ false, // pick_first_existing already checked
        )?;

        let satellites = satellites_templated
            .iter()
            .map(|s| parse_repo_path(&unit_name, s))
            .collect::<Result<Vec<_>, _>>()?;

        let visibility = parse_visibility(&unit_name, cfg.visibility.as_deref())?;
        let cascade_from = match &cfg.cascade_from {
            Some(c) => Some(parse_cascade_rule(&unit_name, c)?),
            None => None,
        };

        let unit = ReleaseUnit {
            name: unit_name,
            ecosystem: parse_ecosystem(&cfg.ecosystem),
            source: VersionSource::Manifests(manifests),
            satellites,
            tag_format: cfg.tag_format.clone(),
            visibility,
            cascade_from,
        };

        units.push(ResolvedReleaseUnit {
            unit,
            origin: ResolveOrigin::Glob {
                glob_index: glob_idx,
                matched_path: matched_repopath,
            },
        });
    }

    Ok(units)
}

// ===========================================================================
// Template substitution
// ===========================================================================

struct TemplateCtx {
    path: String,
    basename: String,
    parent: String,
}

impl TemplateCtx {
    fn from_matched_path(p: &RepoPathBuf) -> Self {
        let path_str = p.escaped().to_string();
        let path_buf = std::path::PathBuf::from(&path_str);
        let basename = path_buf
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let parent = path_buf
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        Self {
            path: path_str,
            basename,
            parent,
        }
    }
}

fn substitute(glob_idx: usize, template: &str, ctx: &TemplateCtx) -> Result<String, ResolverError> {
    let mut out = template.to_string();
    out = out.replace("{path}", &ctx.path);
    out = out.replace("{basename}", &ctx.basename);
    out = out.replace("{parent}", &ctx.parent);

    // Detect leftover `{...}` placeholders.
    if let Some(start) = out.find('{') {
        if let Some(end_off) = out[start..].find('}') {
            let var = &out[start + 1..start + end_off];
            return Err(ResolverError::UnknownTemplateVar {
                glob_index: glob_idx,
                var: var.to_string(),
            });
        }
        return Err(ResolverError::TemplateNotFullySubstituted {
            glob_index: glob_idx,
            template: template.to_string(),
            result: out,
        });
    }
    Ok(out)
}

/// Pre-flight: scan a glob pattern for `{...}` and reject any unknown
/// vars (helps surface typos before glob expansion runs).
fn validate_template_vars_known(glob_idx: usize, raw: &str) -> Result<(), ResolverError> {
    let mut rest = raw;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| ResolverError::TemplateNotFullySubstituted {
                glob_index: glob_idx,
                template: raw.to_string(),
                result: raw.to_string(),
            })?;
        let var = &after[..end];
        if !matches!(var, "path" | "basename" | "parent") {
            return Err(ResolverError::UnknownTemplateVar {
                glob_index: glob_idx,
                var: var.to_string(),
            });
        }
        rest = &after[end + 1..];
    }
    Ok(())
}

// ===========================================================================
// Manifest path resolution & ManifestFile construction
// ===========================================================================

fn pick_first_existing(
    unit_name: &str,
    primary: &[String],
    fallback: &[String],
    repo: &Repository,
) -> Result<String, ResolverError> {
    let mut tried = Vec::new();
    for paths in [primary, fallback] {
        for p in paths {
            tried.push(p.clone());
            let buf = RepoPathBuf::new(p.as_bytes());
            let abs = repo.resolve_workdir(&buf);
            if abs.exists() {
                return Ok(p.clone());
            }
        }
    }
    Err(ResolverError::AllManifestsAndFallbacksMissing {
        unit: unit_name.to_string(),
        tried,
    })
}

fn build_manifests(
    unit_name: &str,
    unit_ecosystem: &str,
    cfg_manifests: &[ManifestFileConfig],
    repo: &Repository,
    require_existence: bool,
) -> Result<Vec<ManifestFile>, ResolverError> {
    let mut out = Vec::new();
    for m in cfg_manifests {
        let path = parse_repo_path(unit_name, &m.path)?;

        if require_existence {
            let abs = repo.resolve_workdir(&path);
            if !abs.exists() {
                return Err(ResolverError::PathDoesNotExist {
                    unit: unit_name.to_string(),
                    path: m.path.clone(),
                });
            }
        }

        let manifest_eco = match &m.ecosystem {
            Some(e) => parse_ecosystem(e),
            None => parse_ecosystem(unit_ecosystem),
        };

        let version_field = parse_version_field(unit_name, &m, &manifest_eco)?;

        out.push(ManifestFile {
            path,
            ecosystem: manifest_eco,
            version_field,
        });
    }
    Ok(out)
}

fn parse_version_field(
    unit_name: &str,
    cfg: &ManifestFileConfig,
    ecosystem: &Ecosystem,
) -> Result<VersionFieldSpec, ResolverError> {
    let spec = match cfg.version_field.as_str() {
        "cargo_toml" => VersionFieldSpec::CargoToml,
        "npm_package_json" => VersionFieldSpec::NpmPackageJson,
        "tauri_conf_json" => VersionFieldSpec::TauriConfJson,
        "gradle_properties" => VersionFieldSpec::GradleProperties,
        "generic_regex" => {
            let pattern = cfg.regex_pattern.clone().ok_or_else(|| {
                ResolverError::GenericRegexMissingPatternOrReplace {
                    unit: unit_name.to_string(),
                }
            })?;
            let replace = cfg.regex_replace.clone().ok_or_else(|| {
                ResolverError::GenericRegexMissingPatternOrReplace {
                    unit: unit_name.to_string(),
                }
            })?;
            // Validate exactly one capture group.
            let r = regex::Regex::new(&pattern).map_err(|e| ResolverError::InvalidPath {
                unit: unit_name.to_string(),
                path: pattern.clone(),
                reason: format!("regex compile: {e}"),
            })?;
            let captures = r.captures_len() - 1; // captures_len includes the whole-match group
            if captures != 1 {
                return Err(ResolverError::GenericRegexCaptureCount {
                    unit: unit_name.to_string(),
                    pattern,
                    found: captures,
                });
            }
            VersionFieldSpec::GenericRegex { pattern, replace }
        }
        other => {
            return Err(ResolverError::UnknownEnumValue {
                unit: unit_name.to_string(),
                field: "version_field",
                value: other.to_string(),
                allowed: "cargo_toml, npm_package_json, tauri_conf_json, gradle_properties, generic_regex",
            });
        }
    };

    // Edge case 5 — ecosystem ↔ version_field mismatch.
    validate_ecosystem_field_compat(unit_name, ecosystem, &cfg.version_field, &spec)?;

    Ok(spec)
}

fn validate_ecosystem_field_compat(
    unit_name: &str,
    ecosystem: &Ecosystem,
    version_field: &str,
    _spec: &VersionFieldSpec,
) -> Result<(), ResolverError> {
    let ecosystem_str = match ecosystem {
        Ecosystem::Known(k) => k.as_str(),
        Ecosystem::Unknown(s) => s.as_str(),
    };

    // GenericRegex is the escape hatch — accepts any ecosystem.
    if version_field == "generic_regex" {
        return Ok(());
    }

    let compat = match (ecosystem_str, version_field) {
        ("cargo", "cargo_toml") => true,
        ("npm", "npm_package_json") => true,
        ("tauri", "npm_package_json") => true, // single-source Tauri uses package.json
        ("tauri", "cargo_toml") => true,       // legacy multi-file
        ("tauri", "tauri_conf_json") => true,  // legacy multi-file
        // jvm-library uses gradle_properties; "external" is permissive.
        // Anything else with the matching key is allowed; only mismatches
        // we can name with confidence are rejected.
        ("jvm-library", "gradle_properties") => true,
        ("external", _) => true,
        // Unknown ecosystems get a free pass (forward-compat).
        _ if matches!(ecosystem, Ecosystem::Unknown(_)) => true,
        // For known ecosystems, reject if ecosystem name and field key
        // are clearly mismatched (e.g. npm + cargo_toml).
        _ => match (ecosystem_str, version_field) {
            ("npm", "cargo_toml")
            | ("npm", "gradle_properties")
            | ("npm", "tauri_conf_json")
            | ("cargo", "npm_package_json")
            | ("cargo", "gradle_properties")
            | ("cargo", "tauri_conf_json")
            | ("jvm-library", "cargo_toml")
            | ("jvm-library", "npm_package_json")
            | ("jvm-library", "tauri_conf_json") => false,
            _ => true,
        },
    };

    if !compat {
        let hint = match (ecosystem_str, version_field) {
            ("npm", "cargo_toml") => "did you mean ecosystem=\"cargo\"?",
            ("cargo", "npm_package_json") => "did you mean ecosystem=\"npm\"?",
            _ => "use the matching version_field for this ecosystem",
        };
        return Err(ResolverError::EcosystemMismatchVersionField {
            unit: unit_name.to_string(),
            ecosystem: ecosystem_str.to_string(),
            version_field: version_field.to_string(),
            hint: hint.to_string(),
        });
    }
    Ok(())
}

fn default_version_field_for_ecosystem(ecosystem: &str) -> &'static str {
    match ecosystem {
        "cargo" => "cargo_toml",
        "npm" => "npm_package_json",
        "tauri" => "npm_package_json", // single-source default
        "jvm-library" => "gradle_properties",
        _ => "cargo_toml", // fallback; resolver validation catches mismatches
    }
}

// ===========================================================================
// Misc parsing helpers
// ===========================================================================

fn parse_ecosystem(s: &str) -> Ecosystem {
    Ecosystem::classify(s)
}

fn parse_visibility(unit_name: &str, raw: Option<&str>) -> Result<Visibility, ResolverError> {
    match raw {
        None => Ok(Visibility::default()),
        Some(s) => Visibility::from_wire(s).ok_or_else(|| ResolverError::UnknownEnumValue {
            unit: unit_name.to_string(),
            field: "visibility",
            value: s.to_string(),
            allowed: "public, internal, hidden",
        }),
    }
}

fn parse_cascade_rule(
    unit_name: &str,
    c: &CascadeRuleConfig,
) -> Result<CascadeRule, ResolverError> {
    let bump = match c.bump.as_str() {
        "mirror" => CascadeBumpStrategy::Mirror,
        "floor_patch" => CascadeBumpStrategy::FloorPatch,
        "floor_minor" => CascadeBumpStrategy::FloorMinor,
        "floor_major" => CascadeBumpStrategy::FloorMajor,
        other => {
            return Err(ResolverError::UnknownCascadeBumpStrategy {
                unit: unit_name.to_string(),
                strategy: other.to_string(),
            });
        }
    };
    Ok(CascadeRule {
        source: c.source.clone(),
        bump,
    })
}

fn parse_repo_path(unit_name: &str, s: &str) -> Result<RepoPathBuf, ResolverError> {
    if Path::new(s).is_absolute() {
        return Err(ResolverError::InvalidPath {
            unit: unit_name.to_string(),
            path: s.to_string(),
            reason: "must be repo-relative, not absolute".to_string(),
        });
    }
    Ok(RepoPathBuf::new(s.as_bytes()))
}

// ===========================================================================
// Cross-cutting validations
// ===========================================================================

fn unit_paths(unit: &ReleaseUnit) -> Vec<String> {
    let mut paths = Vec::new();
    if let VersionSource::Manifests(ms) = &unit.source {
        for m in ms {
            paths.push(m.path.escaped().to_string());
        }
    }
    for s in &unit.satellites {
        paths.push(s.escaped().to_string());
    }
    paths
}

fn detect_name_collisions(units: &[ResolvedReleaseUnit]) -> Result<(), ResolverError> {
    let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in units {
        let name = r.unit.name.clone();
        let path_label = match &r.origin {
            ResolveOrigin::Explicit { config_index } => {
                format!("[[release_unit]] #{config_index}")
            }
            ResolveOrigin::Glob { matched_path, .. } => matched_path.escaped().to_string(),
            ResolveOrigin::Detected { detector } => format!("detector {detector}"),
        };
        by_name.entry(name).or_default().push(path_label);
    }
    for (name, paths) in by_name {
        if paths.len() > 1 {
            return Err(ResolverError::NameCollision { name, paths });
        }
    }
    Ok(())
}

fn detect_nested_bundles(units: &[ResolvedReleaseUnit]) -> Result<(), ResolverError> {
    // A bundle's "anchor" path is the dirname of its first manifest
    // (for Manifests source) or its first satellite (External).
    fn anchor(u: &ReleaseUnit) -> Option<String> {
        if let VersionSource::Manifests(ms) = &u.source {
            if let Some(first) = ms.first() {
                let s = first.path.escaped().to_string();
                let parent = std::path::Path::new(&s)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                return Some(if parent.is_empty() {
                    "".to_string()
                } else {
                    parent
                });
            }
        }
        u.satellites.first().map(|p| p.escaped().to_string())
    }

    // Edge case 10: anchor == "" means root → reject.
    for r in units {
        if let Some(a) = anchor(&r.unit) {
            if a.is_empty() {
                return Err(ResolverError::BundlePathIsRepoRoot {
                    unit: r.unit.name.clone(),
                });
            }
        }
    }

    // Edge case 9: one anchor is a strict prefix of another's.
    let anchored: Vec<(String, String)> = units
        .iter()
        .filter_map(|r| anchor(&r.unit).map(|a| (r.unit.name.clone(), a)))
        .collect();

    for i in 0..anchored.len() {
        for j in 0..anchored.len() {
            if i == j {
                continue;
            }
            let (outer, outer_path) = &anchored[i];
            let (inner, inner_path) = &anchored[j];
            // strict prefix: inner_path starts with `outer_path/`
            let outer_prefixed = format!("{outer_path}/");
            if inner_path.starts_with(&outer_prefixed) {
                return Err(ResolverError::NestedBundlePath {
                    outer: outer.clone(),
                    outer_path: outer_path.clone(),
                    inner: inner.clone(),
                    inner_path: inner_path.clone(),
                });
            }
        }
    }
    Ok(())
}

fn validate_cascade_sources(units: &[ResolvedReleaseUnit]) -> Result<(), ResolverError> {
    let names: BTreeSet<String> = units.iter().map(|u| u.unit.name.clone()).collect();
    for r in units {
        if let Some(c) = &r.unit.cascade_from {
            if !names.contains(&c.source) {
                return Err(ResolverError::CascadeSourceUnknown {
                    unit: r.unit.name.clone(),
                    cascade_source: c.source.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quick template-substitution test that doesn't need a Repository.
    #[test]
    fn substitute_replaces_known_vars() {
        let ctx = TemplateCtx {
            path: "apps/services/aura".into(),
            basename: "aura".into(),
            parent: "services".into(),
        };
        let out = substitute(0, "{path}/crates/bin/Cargo.toml", &ctx).unwrap();
        assert_eq!(out, "apps/services/aura/crates/bin/Cargo.toml");

        let out2 = substitute(0, "{parent}-{basename}", &ctx).unwrap();
        assert_eq!(out2, "services-aura");
    }

    #[test]
    fn substitute_rejects_unknown_var() {
        let ctx = TemplateCtx {
            path: "x".into(),
            basename: "y".into(),
            parent: "z".into(),
        };
        let err = substitute(3, "{path}/{unknown}", &ctx).unwrap_err();
        assert_eq!(err.rule(), "unknown_template_var");
    }

    #[test]
    fn validate_template_vars_pre_flight_catches_typos() {
        let err = validate_template_vars_known(0, "{basenam}").unwrap_err();
        assert_eq!(err.rule(), "unknown_template_var");

        // Known vars pass.
        validate_template_vars_known(0, "{path}/crates/{basename}/Cargo.toml").unwrap();
    }

    #[test]
    fn template_ctx_extracts_basename_and_parent() {
        let p = RepoPathBuf::new(b"apps/services/aura");
        let ctx = TemplateCtx::from_matched_path(&p);
        assert_eq!(ctx.path, "apps/services/aura");
        assert_eq!(ctx.basename, "aura");
        assert_eq!(ctx.parent, "services");
    }

    #[test]
    fn ecosystem_field_compat_npm_with_cargo_toml_rejects() {
        let unit = "x";
        let eco = Ecosystem::classify("npm");
        let err =
            validate_ecosystem_field_compat(unit, &eco, "cargo_toml", &VersionFieldSpec::CargoToml)
                .unwrap_err();
        assert_eq!(err.rule(), "ecosystem_mismatch_version_field");
    }

    #[test]
    fn ecosystem_field_compat_unknown_ecosystem_passes() {
        // Forward-compat: unknown ecosystem accepts any version_field.
        let eco = Ecosystem::classify("brand-new-eco");
        validate_ecosystem_field_compat("x", &eco, "cargo_toml", &VersionFieldSpec::CargoToml)
            .unwrap();
    }

    #[test]
    fn ecosystem_field_compat_external_passes_anything() {
        let eco = Ecosystem::classify("external");
        validate_ecosystem_field_compat(
            "x",
            &eco,
            "gradle_properties",
            &VersionFieldSpec::GradleProperties,
        )
        .unwrap();
    }

    #[test]
    fn parse_visibility_known_and_unknown() {
        assert_eq!(parse_visibility("x", None).unwrap(), Visibility::Public);
        assert_eq!(
            parse_visibility("x", Some("hidden")).unwrap(),
            Visibility::Hidden
        );
        let err = parse_visibility("x", Some("invisible")).unwrap_err();
        assert_eq!(err.rule(), "unknown_enum_value");
    }

    #[test]
    fn parse_cascade_strategy_all_keys() {
        let cases = [
            ("mirror", CascadeBumpStrategy::Mirror),
            ("floor_patch", CascadeBumpStrategy::FloorPatch),
            ("floor_minor", CascadeBumpStrategy::FloorMinor),
            ("floor_major", CascadeBumpStrategy::FloorMajor),
        ];
        for (key, expected) in cases {
            let r = parse_cascade_rule(
                "x",
                &CascadeRuleConfig {
                    source: "src".into(),
                    bump: key.into(),
                },
            )
            .unwrap();
            assert_eq!(r.bump, expected);
        }

        let err = parse_cascade_rule(
            "x",
            &CascadeRuleConfig {
                source: "src".into(),
                bump: "explode".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err.rule(), "unknown_cascade_bump_strategy");
    }

    #[test]
    fn parse_repo_path_rejects_absolute() {
        let err = parse_repo_path("x", "/absolute/path").unwrap_err();
        assert_eq!(err.rule(), "invalid_path");
    }

    #[test]
    fn detect_nested_bundles_flat_set_passes() {
        // Build two non-nested anchored units.
        let make = |name: &str, manifest_path: &str| ResolvedReleaseUnit {
            unit: ReleaseUnit {
                name: name.to_string(),
                ecosystem: Ecosystem::classify("cargo"),
                source: VersionSource::Manifests(vec![ManifestFile {
                    path: RepoPathBuf::new(manifest_path.as_bytes()),
                    ecosystem: Ecosystem::classify("cargo"),
                    version_field: VersionFieldSpec::CargoToml,
                }]),
                satellites: vec![],
                tag_format: None,
                visibility: Visibility::Public,
                cascade_from: None,
            },
            origin: ResolveOrigin::Explicit { config_index: 0 },
        };

        let units = vec![
            make("aura", "apps/services/aura/crates/bin/Cargo.toml"),
            make("ekko", "apps/services/ekko/crates/bin/Cargo.toml"),
        ];
        detect_nested_bundles(&units).unwrap();
    }

    #[test]
    fn detect_nested_bundles_strict_prefix_rejects() {
        let make = |name: &str, manifest_path: &str| ResolvedReleaseUnit {
            unit: ReleaseUnit {
                name: name.to_string(),
                ecosystem: Ecosystem::classify("cargo"),
                source: VersionSource::Manifests(vec![ManifestFile {
                    path: RepoPathBuf::new(manifest_path.as_bytes()),
                    ecosystem: Ecosystem::classify("cargo"),
                    version_field: VersionFieldSpec::CargoToml,
                }]),
                satellites: vec![],
                tag_format: None,
                visibility: Visibility::Public,
                cascade_from: None,
            },
            origin: ResolveOrigin::Explicit { config_index: 0 },
        };

        let units = vec![
            make("outer", "apps/services/Cargo.toml"),
            make("inner", "apps/services/aura/Cargo.toml"),
        ];
        let err = detect_nested_bundles(&units).unwrap_err();
        assert_eq!(err.rule(), "nested_bundle_path");
    }

    #[test]
    fn detect_name_collisions_two_explicit_same_name() {
        let make = |name: &str| ResolvedReleaseUnit {
            unit: ReleaseUnit {
                name: name.to_string(),
                ecosystem: Ecosystem::classify("cargo"),
                source: VersionSource::Manifests(vec![]),
                satellites: vec![],
                tag_format: None,
                visibility: Visibility::Public,
                cascade_from: None,
            },
            origin: ResolveOrigin::Explicit { config_index: 0 },
        };
        let units = vec![make("aura"), make("aura")];
        let err = detect_name_collisions(&units).unwrap_err();
        assert_eq!(err.rule(), "name_collision");
    }

    #[test]
    fn validate_cascade_sources_unknown_source() {
        let unit = ResolvedReleaseUnit {
            unit: ReleaseUnit {
                name: "sdk-kotlin".into(),
                ecosystem: Ecosystem::classify("jvm-library"),
                source: VersionSource::Manifests(vec![]),
                satellites: vec![],
                tag_format: None,
                visibility: Visibility::Public,
                cascade_from: Some(CascadeRule {
                    source: "ghost-schema".into(),
                    bump: CascadeBumpStrategy::FloorMinor,
                }),
            },
            origin: ResolveOrigin::Explicit { config_index: 0 },
        };
        let err = validate_cascade_sources(&[unit]).unwrap_err();
        assert_eq!(err.rule(), "cascade_source_unknown");
    }
}
