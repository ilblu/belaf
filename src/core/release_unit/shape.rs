//! Detector classification: 3 semantic classes that subsume the
//! flat `DetectorKind` enum from before 1.0.
//!
//! Every detector hit falls into exactly one of three classes and
//! consumers (snippet generation, wizard rendering, drift) dispatch
//! exhaustively via `match m.shape`. This makes the producing-side
//! and the consuming-side line up at the type level — adding a new
//! detector requires picking a class, which determines its wiring.
//!
//! - **Bundle**: a multi-manifest ReleaseUnit. Tauri triplet, hexagonal
//!   cargo service, JVM library. Bundles emit `[[release_unit]]` blocks
//!   and hide their inner manifests in the wizard.
//! - **Hint**: pure metadata that decorates a Standalone row. SDK
//!   cascade members, npm workspace members, single-project root,
//!   nested submodule. Hints are never togglable; they are annotations.
//! - **ExternallyManaged**: read-only paths that need
//!   `[allow_uncovered]` instead of a `[[release_unit]]`. Mobile apps
//!   primarily.

use crate::core::git::repository::RepoPathBuf;

/// One detector hit. The shape determines the class of treatment in
/// every downstream consumer (snippet generator, wizard, drift check).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorMatch {
    pub shape: DetectedShape,
    pub path: RepoPathBuf,
    pub note: Option<String>,
}

/// Three-class taxonomy. Every consumer must dispatch exhaustively.
/// The wizard's 3.0.x bug — Standalones hidden under `SdkCascadeMember`
/// hints — was a structural mistake at this layer; promoting the class
/// into the type makes that bug class unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedShape {
    /// Multi-manifest ReleaseUnit — emits a `[[release_unit]]` block
    /// and hides its inner manifests.
    Bundle(BundleKind),
    /// Pure metadata that decorates a Standalone row. Never togglable.
    Hint(HintKind),
    /// Read-only path that lands in `[allow_uncovered]`.
    ExternallyManaged(ExtKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleKind {
    Tauri { single_source: bool },
    HexagonalCargo { primary: HexagonalPrimary },
    JvmLibrary { version_source: JvmVersionSource },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintKind {
    SdkCascade,
    NpmWorkspace,
    SingleProject { ecosystem: SingleProjectEcosystem },
    NestedMonorepo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtKind {
    MobileIos,
    MobileAndroid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SingleProjectEcosystem {
    Cargo,
    Npm,
    Pypa,
    Go,
    Maven,
    Swift,
    Elixir,
}

impl std::fmt::Display for SingleProjectEcosystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Pypa => "pypa",
            Self::Go => "go",
            Self::Maven => "maven",
            Self::Swift => "swift",
            Self::Elixir => "elixir",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HexagonalPrimary {
    Bin,
    Lib,
    Workers,
    BaseName,
}

impl std::fmt::Display for HexagonalPrimary {
    /// Lowercase so error messages and config snippets read uniformly
    /// (`crates/bin/Cargo.toml`) rather than the PascalCase that the
    /// `Debug` derive would produce.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            HexagonalPrimary::Bin => "bin",
            HexagonalPrimary::Lib => "lib",
            HexagonalPrimary::Workers => "workers",
            HexagonalPrimary::BaseName => "basename",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JvmVersionSource {
    GradleProperties,
    BuildGradleKtsLiteral,
    PluginManaged,
}

impl DetectedShape {
    /// Bundles emit `[[release_unit]]` blocks and hide inner manifests.
    pub fn is_bundle(&self) -> bool {
        matches!(self, Self::Bundle(_))
    }

    /// Hints decorate Standalone rows; never togglable.
    pub fn is_hint(&self) -> bool {
        matches!(self, Self::Hint(_))
    }

    /// Externally-managed paths land in `[allow_uncovered]` — read-only.
    pub fn is_externally_managed(&self) -> bool {
        matches!(self, Self::ExternallyManaged(_))
    }
}
