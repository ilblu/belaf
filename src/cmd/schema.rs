//! `belaf schema <name>` — print an embedded JSON Schema by name.
//!
//! Agents that produce or consume belaf artefacts (release manifests,
//! status payloads, etc.) can fetch the authoritative schema from the
//! binary without round-tripping to the dashboard or the github-app
//! repo.

use anyhow::{anyhow, Result};

use crate::core::config::syntax::ReleaseConfiguration;

const MANIFEST_SCHEMA: &str = include_str!("../../schemas/manifest.v1.schema.json");

/// One row of `belaf describe --json`'s `schemas` array. Kept here so
/// the schema list has a single source of truth.
pub const AVAILABLE_SCHEMAS: &[(&str, &str)] = &[
    (
        "manifest",
        "Belaf release manifest, v1 (JSON Schema Draft 2020-12)",
    ),
    (
        "config",
        "belaf/config.toml — the file `belaf init` writes and users edit",
    ),
];

/// The config schema, derived from the serde types that actually parse the
/// file rather than written by hand.
///
/// `manifest` is the format belaf *writes*; `config` is the one its users
/// write, and until now it was the one you could not ask the binary about.
/// An agent working in someone else's repo has the installed binary and
/// nothing else — not `docs/configuration.md`, not `src/core/config.rs` — so
/// "what is the exact spelling of `[ignore_paths]`?" had no answer that did
/// not involve guessing. Generating it from [`ReleaseConfiguration`] means it
/// cannot drift from the parser: a field added to the struct appears here on
/// the next build, and `deny_unknown_fields` carries through as
/// `additionalProperties: false`.
fn config_schema() -> Result<String> {
    let schema = schemars::schema_for!(ReleaseConfiguration);
    Ok(serde_json::to_string_pretty(&schema)?)
}

pub fn run(name: String) -> Result<i32> {
    let body = match name.as_str() {
        "manifest" => MANIFEST_SCHEMA.to_string(),
        "config" => config_schema()?,
        other => {
            let known: Vec<&str> = AVAILABLE_SCHEMAS.iter().map(|(n, _)| *n).collect();
            return Err(anyhow!(
                "unknown schema `{}` (available: {})",
                other,
                known.join(", ")
            ));
        }
    };
    println!("{}", body);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_advertised_schema_can_actually_be_printed() {
        for (name, _) in AVAILABLE_SCHEMAS {
            let out = match *name {
                "manifest" => MANIFEST_SCHEMA.to_string(),
                "config" => config_schema().expect("BUG: config schema should generate"),
                other => panic!("`{other}` is advertised but `run` cannot produce it"),
            };
            assert!(
                serde_json::from_str::<serde_json::Value>(&out).is_ok(),
                "schema `{name}` must be valid JSON"
            );
        }
    }

    /// The two questions the tool could not answer before, asked of the
    /// schema itself.
    #[test]
    fn the_config_schema_describes_the_tables_users_have_to_write() {
        let raw = config_schema().expect("BUG: config schema should generate");
        for table in [
            "ignore_paths",
            "group",
            "release_unit",
            "allow_uncovered",
            "cascade_inputs",
        ] {
            assert!(
                raw.contains(&format!("\"{table}\"")),
                "`[{table}]` must be discoverable from `belaf schema config`"
            );
        }
    }

    /// `ReleaseConfiguration` is `deny_unknown_fields`; a schema that did not
    /// say so would tell an agent that typos are acceptable.
    #[test]
    fn unknown_fields_are_rejected_in_the_schema_too() {
        let raw = config_schema().expect("BUG: config schema should generate");
        assert!(
            raw.contains("\"additionalProperties\": false"),
            "deny_unknown_fields must carry through to the schema"
        );
    }
}
