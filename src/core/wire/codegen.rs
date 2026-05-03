//! typify-generated wire types from `schemas/manifest.v1.schema.json`.
//!
//! Do not edit manually — regenerate by changing the schema and rebuilding.
//! All consumers of these types should import from `core::wire::domain`,
//! not from here directly.

#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_qualifications,
    missing_docs,
    rustdoc::broken_intra_doc_links,
    rustdoc::bare_urls,
    clippy::all,
    clippy::pedantic,
    clippy::allow_attributes,
    clippy::needless_lifetimes,
    clippy::redundant_closure_for_method_calls,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::derive_partial_eq_without_eq,
    clippy::large_enum_variant
)]

include!(concat!(env!("OUT_DIR"), "/manifest_v1_codegen.rs"));
