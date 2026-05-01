// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Error types raised by graph construction and lookup.
//!
//! Extracted from the parent `graph` module during the 3.0 refactor so
//! that builder/query/iterator concerns each live in their own file.

use thiserror::Error as ThisError;

/// An error returned when an input has requested a project with a certain name,
/// and it just doesn't exist.
#[derive(Debug, ThisError)]
#[error("no such project with the name `{0}`")]
pub struct NoSuchProjectError(pub String);

/// An error returned when the internal project graph has a dependency cycle.
/// The inner value is the user-facing name of a project involved in the cycle.
#[derive(Debug, ThisError)]
#[error("detected an internal dependency cycle associated with project {0}")]
pub struct DependencyCycleError(pub String);

/// An error returned when it is impossible to come up with distinct names for
/// two projects. This "should never happen", but ... The inner value is the
/// clashing name.
#[derive(Debug, ThisError)]
#[error("multiple projects with same name `{0}`")]
pub struct NamingClashError(pub String);
