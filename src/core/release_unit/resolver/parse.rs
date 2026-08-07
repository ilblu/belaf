//! Leaf-value parsers for the scalar fields on a `[release_unit.<name>]`
//! block: `ecosystem`, `visibility`, `kind`, `baseline`, `cascade_from`, and
//! repo-relative paths.
//!
//! Each one is total over its input and produces a typed value or a named
//! [`ResolverError`] — no `unwrap_or_default` swallowing a typo. They are
//! shared by all three resolution paths (explicit block, glob expansion,
//! partial override), which is why they live apart from any of them.

use std::path::Path;

use crate::core::git::repository::RepoPathBuf;
use crate::core::release_unit::syntax::CascadeRuleConfig;
use crate::core::release_unit::validator::ResolverError;
use crate::core::release_unit::{BaselineSpec, CascadeBumpStrategy, CascadeRule, Visibility};
use crate::core::wire::known::Ecosystem;

pub(super) fn parse_ecosystem(s: &str) -> Ecosystem {
    Ecosystem::classify(s)
}

pub(super) fn parse_visibility(
    unit_name: &str,
    raw: Option<&str>,
) -> Result<Visibility, ResolverError> {
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

/// Parse the `kind = "deploy" | "internal" | "ignore"` field (F1). Defaults
/// to `Deploy` when unset (back-compat).
pub(super) fn parse_kind(
    unit_name: &str,
    raw: Option<&str>,
) -> Result<crate::core::resolved_release_unit::UnitKind, ResolverError> {
    use crate::core::resolved_release_unit::UnitKind;
    match raw {
        None => Ok(UnitKind::default()),
        Some(s) => match s.to_lowercase().as_str() {
            "deploy" => Ok(UnitKind::Deploy),
            "internal" => Ok(UnitKind::Internal),
            "ignore" => Ok(UnitKind::Ignore),
            _ => Err(ResolverError::UnknownEnumValue {
                unit: unit_name.to_string(),
                field: "kind",
                value: s.to_string(),
                allowed: "deploy, internal, ignore",
            }),
        },
    }
}

/// Parse the `baseline = "first-release" | "<commit-ish>"` field.
///
/// Deliberately permissive about the commit form: the string is handed to
/// git at history-analysis time, so short shas, full shas, tags and branch
/// names all work and an unresolvable value fails there with the repo in
/// hand (this function has no repo). The only thing rejected here is an
/// empty value, which can only ever be a mistake.
pub(super) fn parse_baseline(
    unit_name: &str,
    raw: Option<&str>,
) -> Result<Option<BaselineSpec>, ResolverError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ResolverError::BaselineEmpty {
            unit: unit_name.to_string(),
        });
    }
    Ok(Some(if trimmed == BaselineSpec::FIRST_RELEASE {
        BaselineSpec::FirstRelease
    } else {
        BaselineSpec::Commit(trimmed.to_string())
    }))
}

pub(super) fn parse_cascade_rule(
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

pub(super) fn parse_repo_path(unit_name: &str, s: &str) -> Result<RepoPathBuf, ResolverError> {
    if Path::new(s).is_absolute() {
        return Err(ResolverError::InvalidPath {
            unit: unit_name.to_string(),
            path: s.to_string(),
            reason: "must be repo-relative, not absolute".to_string(),
        });
    }
    Ok(RepoPathBuf::new(s.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_baseline_keyword_sha_and_empty() {
        assert_eq!(parse_baseline("x", None).unwrap(), None);
        assert_eq!(
            parse_baseline("x", Some("first-release")).unwrap(),
            Some(BaselineSpec::FirstRelease)
        );
        // Surrounding whitespace is trimmed, not treated as part of the sha.
        assert_eq!(
            parse_baseline("x", Some("  first-release ")).unwrap(),
            Some(BaselineSpec::FirstRelease)
        );
        assert_eq!(
            parse_baseline("x", Some("8eb3e3cf78ac6e")).unwrap(),
            Some(BaselineSpec::Commit("8eb3e3cf78ac6e".to_string()))
        );
        let err = parse_baseline("x", Some("   ")).unwrap_err();
        assert_eq!(err.rule(), "baseline_empty");
    }

    #[test]
    fn baseline_spec_round_trips_through_its_wire_value() {
        for raw in ["first-release", "8eb3e3cf78ac6e", "v1.2.3"] {
            let spec = parse_baseline("x", Some(raw)).unwrap().unwrap();
            assert_eq!(spec.wire_value(), raw);
        }
    }

    #[test]
    fn parse_repo_path_rejects_absolute() {
        let err = parse_repo_path("x", "/absolute/path").unwrap_err();
        assert_eq!(err.rule(), "invalid_path");
    }
}
