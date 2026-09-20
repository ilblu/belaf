//! Detector classification: 4 semantic classes that subsume the
//! flat `DetectorKind` enum from before 1.0.
//!
//! Every detector hit falls into exactly one of four classes and
//! consumers (snippet generation, wizard rendering, drift) dispatch
//! exhaustively via `match m.shape`. This makes the producing-side
//! and the consuming-side line up at the type level — adding a new
//! detector requires picking a class, which determines its wiring.
//!
//! - **Bundle**: a multi-manifest ReleaseUnit. Tauri triplet, hexagonal
//!   cargo service, JVM library. Bundles emit `[release_unit.<name>]` blocks
//!   and hide their inner manifests in the wizard.
//! - **Hint**: pure metadata that decorates a Standalone row. SDK
//!   cascade members, npm workspace members, single-project root,
//!   nested submodule. Hints are never togglable; they are annotations.
//! - **ExternallyManaged**: read-only paths that need
//!   `[allow_uncovered]` instead of a `[release_unit.<name>]`. Mobile apps
//!   primarily.
//! - **NeedsDecision**: a release artifact belaf recognises but cannot
//!   classify on its own. Emits **nothing** into config — not a
//!   `[release_unit.<name>]` block, and deliberately not an
//!   `[allow_uncovered]` entry either — and stays a drift signal, so
//!   `prepare` aborts and names the path until a human resolves it.
//!
//! That last class exists because the other three have no room for
//! "I do not know". Before it, the JVM detector's final branch was a
//! bare `else` that read "a Gradle build script with no version I can
//! write must belong to someone else's release tooling" and wrote an
//! `[allow_uncovered]` line accordingly. For a project that simply
//! keeps its version somewhere the rewriter cannot reach, that line is
//! the worst possible outcome: the artifact is silently dropped from
//! every release, and the one mechanism that would have reported it —
//! the drift detector — is switched off by the same line. A guess that
//! disables its own alarm is worse than an abort, so unclassifiable
//! hits now get a class that reports instead of guessing.

use crate::core::git::repository::RepoPathBuf;

/// One detector hit. The shape determines the class of treatment in
/// every downstream consumer (snippet generator, wizard, drift check).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorMatch {
    pub shape: DetectedShape,
    pub path: RepoPathBuf,
    pub note: Option<String>,
}

/// Four-class taxonomy. Every consumer must dispatch exhaustively.
/// The wizard's 3.0.x bug — Standalones hidden under `SdkCascadeMember`
/// hints — was a structural mistake at this layer; promoting the class
/// into the type makes that bug class unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedShape {
    /// Multi-manifest ReleaseUnit — emits a `[release_unit.<name>]` block
    /// and hides its inner manifests.
    Bundle(BundleKind),
    /// Pure metadata that decorates a Standalone row. Never togglable.
    Hint(HintKind),
    /// Read-only path that lands in `[allow_uncovered]`.
    ExternallyManaged(ExtKind),
    /// Recognised release artifact that belaf cannot classify. Emits
    /// no config at all and keeps signalling drift until a human
    /// resolves it. See the module docs for why this is a class of its
    /// own rather than a flavour of [`Self::ExternallyManaged`].
    NeedsDecision(DecisionKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleKind {
    Tauri {
        single_source: bool,
        /// Repo-relative directory of the cargo workspace that owns the
        /// app's `src-tauri` crate, when that workspace versions every
        /// member together (`[workspace.package].version` + every member
        /// inheriting it). Empty string for a workspace at the repo root.
        ///
        /// When set, the workspace and the app are one release, not two:
        /// the crate reads its version from the root key, so bumping the
        /// app *is* bumping the workspace. The emitted block has to claim
        /// the root manifest, or the cargo loader discovers the same
        /// version a second time and the repo ends up with `cargo:<name>`
        /// beside `tauri:<name>`.
        shared_workspace: Option<String>,
    },
    HexagonalCargo {
        primary: HexagonalPrimary,
    },
    JvmLibrary {
        version_source: JvmVersionSource,
    },
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
    /// JVM project where versioning is owned by a Gradle plugin
    /// (axion-release, nebula-release, app-versioning, etc.). The
    /// plugin is the user's release tool of choice; belaf treats it
    /// as out-of-scope and lands the path in `[allow_uncovered]` —
    /// same mental model as Mobile (Fastlane/Bitrise).
    ///
    /// Only ever produced by a *positive* match on a known plugin id
    /// (see `bundle::jvm_library::VERSIONING_PLUGIN_IDS`). "No version
    /// I can write" on its own is [`DecisionKind::JvmVersionUnwritable`],
    /// not this.
    JvmPluginManaged,
}

/// Why a hit needs a human. Each variant owns the remediation text
/// shown in the drift error and the init wizard, because the useful
/// instruction differs per cause — and a generic "add it to
/// `[allow_uncovered]`" is exactly the advice that must not be given
/// here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionKind {
    /// A JVM project whose version belaf can neither read nor write:
    /// no `version=` line in `gradle.properties`, no line-anchored
    /// `version = "…"` in the build script, and no known versioning
    /// plugin applied that would legitimately own the version instead.
    JvmVersionUnwritable {
        build_file: GradleBuildFile,
        reason: JvmUnwritableReason,
    },
}

/// Which Gradle build script the JVM detector read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradleBuildFile {
    Kts,
    Groovy,
}

impl GradleBuildFile {
    pub fn filename(self) -> &'static str {
        match self {
            Self::Kts => "build.gradle.kts",
            Self::Groovy => "build.gradle",
        }
    }
}

impl std::fmt::Display for GradleBuildFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.filename())
    }
}

/// The two ways a Gradle version can be out of the rewriter's reach.
/// They are worth telling apart: one is a one-line move, the other is
/// a genuine "where does this version even come from" question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JvmUnwritableReason {
    /// A `version = "…"` assignment exists but is nested inside a
    /// block (`publishing { … }`, `allprojects { … }`, `subprojects
    /// { … }`), so the line-anchored rewriter cannot address it.
    /// `line` is 1-indexed, for a message the user can jump to.
    NotAtLineStart { line: usize },
    /// No version assignment found at all — the version is computed,
    /// inherited from an outer build, or simply absent.
    Absent,
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
    /// A line-anchored `version = "…"` in the project's build script.
    /// Carries which script it is: a Groovy project's version lives in
    /// `build.gradle`, and emitting a `build.gradle.kts` manifest path
    /// for it would point the rewriter at a file that does not exist.
    BuildScriptLiteral {
        build_file: GradleBuildFile,
    },
}

impl DetectedShape {
    /// Bundles emit `[release_unit.<name>]` blocks and hide inner manifests.
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

    /// Unclassifiable hits: no config emitted, drift keeps reporting.
    pub fn is_needs_decision(&self) -> bool {
        matches!(self, Self::NeedsDecision(_))
    }
}
