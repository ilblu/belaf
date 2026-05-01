//! Wire-format types for the belaf release manifest (`schema_version: "3.0"`).
//!
//! This module is the boundary between the **JSON Schema** (the on-disk wire
//! format, owned by belaf, vendored by github-app) and the **domain model**
//! used inside the CLI.
//!
//! ## Layout
//!
//! - [`codegen`] — the `typify`-generated structs from
//!   `schemas/manifest.v3.0.schema.json`. Auto-generated; do not edit.
//!   All field types are 1:1 with the schema (newtypes for pattern-validated
//!   strings, plain `String` for free-form fields).
//! - [`known`] — hand-maintained whitelists for variant fields the schema
//!   intentionally leaves open (`ecosystem`, `bump_type`, status). Provides
//!   `Known | Unknown` discriminated unions so consumers can dispatch on
//!   recognised values without breaking when a new value appears on the wire.
//! - [`domain`] — ergonomic Rust API for the rest of the CLI: `Manifest`,
//!   `Release`, `Group`, builder methods, helpers. Consumes `codegen` types
//!   and the `known` enums.

pub mod codegen;
pub mod domain;
pub mod known;

pub use known::{
    BumpType, Ecosystem, KnownBumpType, KnownEcosystem, KnownReleaseStatus, ReleaseStatus,
    KNOWN_BUMP_TYPES, KNOWN_ECOSYSTEMS, KNOWN_RELEASE_STATUSES,
};
