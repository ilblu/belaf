//! `belaf schema <name>` — print an embedded JSON Schema by name.
//!
//! Agents that produce or consume belaf artefacts (release manifests,
//! status payloads, etc.) can fetch the authoritative schema from the
//! binary without round-tripping to the dashboard or the github-app
//! repo.

use anyhow::{anyhow, Result};

const MANIFEST_SCHEMA: &str = include_str!("../../schemas/manifest.v1.schema.json");

/// One row of `belaf describe --json`'s `schemas` array. Kept here so
/// the schema list has a single source of truth.
pub const AVAILABLE_SCHEMAS: &[(&str, &str)] = &[(
    "manifest",
    "Belaf release manifest, v1 (JSON Schema Draft 2020-12)",
)];

pub fn run(name: String) -> Result<i32> {
    let body = match name.as_str() {
        "manifest" => MANIFEST_SCHEMA,
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
