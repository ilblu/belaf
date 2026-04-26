//! Code-generated belaf CLI API client.
//!
//! This module is built at compile-time from `api-spec/openapi.cli.json` via
//! `build.rs` + progenitor. The actual `ApiClient` (in `client.rs`) keeps using
//! reqwest directly — only the wire types under `types::*` are consumed
//! elsewhere in the crate. The generated `Client` struct is unused but kept
//! around so future migration to the progenitor client is a drop-in.

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
    clippy::needless_pass_by_value
)]

include!(concat!(env!("OUT_DIR"), "/belaf_api_codegen.rs"));
