//! Turning the `manifests = [...]` wire form into [`ManifestFile`] values.
//!
//! Two responsibilities: pick which of a glob's candidate paths actually
//! exists on disk, and validate that the declared `version_field` is one this
//! build knows and that it is compatible with the unit's `ecosystem` — the
//! `npm` + `cargo_toml` class of mistake, which otherwise fails much later
//! with a confusing "version not found in file".

use crate::core::git::repository::{RepoPathBuf, Repository};
use crate::core::release_unit::syntax::ManifestFileConfig;
use crate::core::release_unit::validator::ResolverError;
use crate::core::release_unit::{ManifestFile, VersionFieldSpec};
use crate::core::wire::known::Ecosystem;

use super::parse::{parse_ecosystem, parse_repo_path};

pub(super) fn pick_first_existing(
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

pub(super) fn build_manifests(
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

        let version_field = parse_version_field(unit_name, m, &manifest_eco)?;

        out.push(ManifestFile {
            path,
            ecosystem: manifest_eco,
            version_field,
        });
    }
    Ok(out)
}

pub(super) fn parse_version_field(
    unit_name: &str,
    cfg: &ManifestFileConfig,
    ecosystem: &Ecosystem,
) -> Result<VersionFieldSpec, ResolverError> {
    let spec = match cfg.version_field.as_str() {
        "cargo_toml" => VersionFieldSpec::CargoToml,
        "npm_package_json" => VersionFieldSpec::NpmPackageJson,
        "tauri_conf_json" => VersionFieldSpec::TauriConfJson,
        "gradle_properties" => VersionFieldSpec::GradleProperties,
        "pep_621" => VersionFieldSpec::Pep621,
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
                allowed: "cargo_toml, npm_package_json, tauri_conf_json, gradle_properties, pep_621, generic_regex",
            });
        }
    };

    // Edge case 5 — ecosystem ↔ version_field mismatch.
    validate_ecosystem_field_compat(unit_name, ecosystem, &cfg.version_field, &spec)?;

    Ok(spec)
}

pub(super) fn validate_ecosystem_field_compat(
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
        ("pypa", "pep_621") => true,
        ("external", _) => true,
        // Unknown ecosystems get a free pass (forward-compat).
        _ if matches!(ecosystem, Ecosystem::Unknown(_)) => true,
        // For known ecosystems, reject if ecosystem name and field key
        // are clearly mismatched (e.g. npm + cargo_toml).
        _ => !matches!(
            (ecosystem_str, version_field),
            ("npm", "cargo_toml")
                | ("npm", "gradle_properties")
                | ("npm", "tauri_conf_json")
                | ("npm", "pep_621")
                | ("cargo", "npm_package_json")
                | ("cargo", "gradle_properties")
                | ("cargo", "tauri_conf_json")
                | ("cargo", "pep_621")
                | ("jvm-library", "cargo_toml")
                | ("jvm-library", "npm_package_json")
                | ("jvm-library", "tauri_conf_json")
                | ("jvm-library", "pep_621")
        ),
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

pub fn default_version_field_for_ecosystem(ecosystem: &str) -> &'static str {
    match ecosystem {
        "cargo" => "cargo_toml",
        "npm" => "npm_package_json",
        "tauri" => "npm_package_json", // single-source default
        "jvm-library" => "gradle_properties",
        "pypa" => "pep_621",
        _ => "cargo_toml", // fallback; resolver validation catches mismatches
    }
}

/// Standard manifest filename for a given ecosystem identifier. Used
/// by `auto_detect` when emitting decorator blocks (e.g. cascade_from
/// overrides) for auto-discovered standalone units — the loader knows
/// the exact path internally but the wire-form decorator block needs
/// the path explicit. Returns `None` for unknown ecosystems.
pub fn default_manifest_filename_for_ecosystem(ecosystem: &str) -> Option<&'static str> {
    Some(match ecosystem {
        "cargo" => "Cargo.toml",
        "npm" => "package.json",
        "pypa" => "pyproject.toml",
        "go" => "go.mod",
        "maven" => "pom.xml",
        "swift" => "Package.swift",
        "elixir" => "mix.exs",
        "csproj" => "*.csproj",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
