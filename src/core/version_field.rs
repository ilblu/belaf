//! Per-`VersionFieldSpec` read + write implementations.
//!
//! Each sub-module implements two functions:
//!
//! - `read(path: &Path) -> Result<String>` — extract the current
//!   version string from the file, idempotent
//! - `write(path: &Path, new_version: &str) -> Result<()>` — patch
//!   the file in place to point at `new_version`, preserving
//!   formatting (comments, ordering, indentation) wherever feasible
//!
//! The top-level [`read`] / [`write`] dispatchers map a
//! [`VersionFieldSpec`] to the matching sub-module call.
//!
//! Phase D of `BELAF_MASTER_PLAN.md`.

use std::path::Path;

use thiserror::Error;

use crate::core::release_unit::VersionFieldSpec;

pub mod cargo_toml;
pub mod generic_regex;
pub mod gradle_properties;
pub mod npm_json;
pub mod tauri_conf;

#[derive(Debug, Error)]
pub enum VersionFieldError {
    #[error("I/O error reading/writing `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("`{path}` could not be parsed as {kind}: {reason}")]
    ParseError {
        path: String,
        kind: &'static str,
        reason: String,
    },

    #[error("`{path}` has no version field (looked for {looked_for})")]
    VersionFieldMissing {
        path: String,
        looked_for: &'static str,
    },

    #[error("`{path}` has malformed version `{value}`: {reason}")]
    MalformedVersion {
        path: String,
        value: String,
        reason: String,
    },

    #[error("regex compile error for `{pattern}`: {source}")]
    RegexCompile {
        pattern: String,
        #[source]
        source: regex::Error,
    },
}

pub type Result<T> = std::result::Result<T, VersionFieldError>;

/// Read the current version string from `path` according to `spec`.
pub fn read(spec: &VersionFieldSpec, path: &Path) -> Result<String> {
    match spec {
        VersionFieldSpec::CargoToml => cargo_toml::read(path),
        VersionFieldSpec::NpmPackageJson => npm_json::read(path),
        VersionFieldSpec::TauriConfJson => tauri_conf::read(path),
        VersionFieldSpec::GradleProperties => gradle_properties::read(path),
        VersionFieldSpec::GenericRegex {
            pattern,
            replace: _,
        } => generic_regex::read(path, pattern),
    }
}

/// Write `new_version` to `path` according to `spec`. Idempotent —
/// if the file is already at `new_version`, this is a no-op (no
/// disk write).
pub fn write(spec: &VersionFieldSpec, path: &Path, new_version: &str) -> Result<()> {
    match spec {
        VersionFieldSpec::CargoToml => cargo_toml::write(path, new_version),
        VersionFieldSpec::NpmPackageJson => npm_json::write(path, new_version),
        VersionFieldSpec::TauriConfJson => tauri_conf::write(path, new_version),
        VersionFieldSpec::GradleProperties => gradle_properties::write(path, new_version),
        VersionFieldSpec::GenericRegex { pattern, replace } => {
            generic_regex::write(path, pattern, replace, new_version)
        }
    }
}
