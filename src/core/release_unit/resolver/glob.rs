//! Glob-form `[release_unit.<key>]` expansion: one config block becomes N
//! units, one per matching directory.
//!
//! Also holds the `{path}` / `{basename}` / `{parent}` template engine the
//! glob form uses for `name`, `manifests`, `fallback_manifests` and
//! `satellites`. The templates are deliberately tiny and closed — an unknown
//! `{var}` is an error, not a literal, so a typo surfaces at config-load
//! instead of producing a directory named `{basenam}`.

use crate::core::git::repository::{RepoPathBuf, Repository};
use crate::core::release_unit::syntax::{ManifestFileConfig, ManifestList, ReleaseUnitConfig};
use crate::core::release_unit::validator::ResolverError;
use crate::core::release_unit::{ReleaseUnit, ResolveOrigin, ResolvedReleaseUnit, VersionSource};

use super::manifests::{build_manifests, default_version_field_for_ecosystem, pick_first_existing};
use super::parse::{
    parse_baseline, parse_cascade_rule, parse_ecosystem, parse_kind, parse_repo_path,
    parse_visibility,
};

pub(super) fn expand_glob(
    repo: &Repository,
    glob_idx: usize,
    config_key: &str,
    cfg: &ReleaseUnitConfig,
    explicit_covered_paths: &[String],
) -> Result<Vec<ResolvedReleaseUnit>, ResolverError> {
    let glob_pattern = cfg
        .glob
        .as_ref()
        .expect("expand_glob called with non-glob entry");

    // Glob entries must use template-form `manifests = ["..."]` and the
    // unit-level `name` template; `external` is not supported because
    // each match would need its own command.
    if cfg.external.is_some() {
        return Err(ResolverError::GlobUnitHasExternal {
            config_key: config_key.to_string(),
        });
    }
    let templates: &Vec<String> = match &cfg.manifests {
        Some(ManifestList::Templates(t)) => t,
        Some(ManifestList::Explicit(_)) => {
            return Err(ResolverError::GlobUnitHasExplicitManifests {
                config_key: config_key.to_string(),
            });
        }
        None => {
            return Err(ResolverError::SourceNotSet {
                unit: config_key.to_string(),
            });
        }
    };
    let name_template =
        cfg.name
            .as_deref()
            .ok_or_else(|| ResolverError::GlobUnitMissingNameTemplate {
                config_key: config_key.to_string(),
            })?;

    // Pre-validate template syntax on the glob pattern (the pattern
    // itself is not template-substituted, but typos in template-style
    // braces would silently miss).
    validate_template_vars_known(glob_idx, glob_pattern)?;

    let workdir_repopath = RepoPathBuf::new(b"");
    let workdir = repo
        .resolve_workdir(&workdir_repopath)
        .canonicalize()
        .map_err(|e| ResolverError::InvalidPath {
            unit: format!("[release_unit.{config_key}]"),
            path: glob_pattern.clone(),
            reason: format!("repo workdir canonicalize failed: {e}"),
        })?;

    let pattern_abs = workdir.join(glob_pattern);
    let pattern_str = pattern_abs.to_string_lossy().to_string();

    let mut units = Vec::new();
    let entries = match glob::glob(&pattern_str) {
        Ok(e) => e,
        Err(err) => {
            return Err(ResolverError::InvalidPath {
                unit: format!("[release_unit.{config_key}]"),
                path: glob_pattern.clone(),
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
                    unit: format!("[release_unit.{config_key}]"),
                    path: entry.display().to_string(),
                    reason: format!("convert_path: {e}"),
                })?;

        // "Explicit wins" is checked here, before the manifest probe below,
        // not only on the units this function returns. A directory the user
        // declared explicitly — typically `kind = "ignore"` — carries none of
        // the manifests this glob expects, so it would be soft-skipped with a
        // loud warning and never reach the caller's shadow check at all. The
        // user already said what that path is; there is nothing to report.
        let matched_path = matched_repopath.escaped();
        let anchor_prefix = format!("{matched_path}/");
        if explicit_covered_paths
            .iter()
            .any(|p| p == &matched_path || p.starts_with(&anchor_prefix))
        {
            continue;
        }

        let ctx = TemplateCtx::from_matched_path(&matched_repopath);

        let unit_name = substitute(glob_idx, name_template, &ctx)?;
        let manifests_paths_templated: Vec<String> = templates
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

        // F9 — glob soft-skip: a directory that matches the glob pattern but
        // carries none of the expected manifests/fallbacks is simply not a unit
        // of this shape (e.g. `apps/services/e2e`, a flat test crate, matched by
        // `apps/services/*` whose template expects `crates/bin/Cargo.toml`).
        // Skip it instead of hard-erroring the whole resolve — but log loudly,
        // so a real service silently vanishing from releases stays observable.
        // Any *other* resolver error still propagates.
        let chosen_manifest = match pick_first_existing(
            &unit_name,
            &manifests_paths_templated,
            &fallback_paths_templated,
            repo,
        ) {
            Ok(m) => m,
            Err(ResolverError::AllManifestsAndFallbacksMissing { tried, .. }) => {
                tracing::warn!(
                    "release_unit `{config_key}`: glob match `{}` has none of the \
                     expected manifests (tried: {}) — skipping (not a unit of this shape)",
                    matched_repopath.escaped(),
                    tried.join(", "),
                );
                continue;
            }
            Err(e) => return Err(e),
        };

        let cfg_ecosystem =
            cfg.ecosystem
                .as_deref()
                .ok_or_else(|| ResolverError::SourceNotSet {
                    unit: config_key.to_string(),
                })?;
        let version_field_key = match &cfg.version_field {
            Some(s) => s.clone(),
            None => default_version_field_for_ecosystem(cfg_ecosystem).to_string(),
        };

        let manifests = build_manifests(
            &unit_name,
            cfg_ecosystem,
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
        let kind = parse_kind(&unit_name, cfg.kind.as_deref())?;
        let cascade_from = match &cfg.cascade_from {
            Some(c) => Some(parse_cascade_rule(&unit_name, c)?),
            None => None,
        };
        // A glob block's `baseline` applies to every unit it expands to.
        // That is a blunt instrument by construction — `belaf baseline
        // --fix` therefore never writes into a glob block; it writes a
        // per-unit override block instead.
        let baseline = parse_baseline(&unit_name, cfg.baseline.as_deref())?;

        let unit = ReleaseUnit {
            name: unit_name,
            ecosystem: parse_ecosystem(cfg_ecosystem),
            source: VersionSource::Manifests(manifests),
            satellites,
            tag_format: cfg.tag_format.clone(),
            visibility,
            cascade_from,
            kind,
            bump_override: cfg.bump.clone(),
            baseline,
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
}
