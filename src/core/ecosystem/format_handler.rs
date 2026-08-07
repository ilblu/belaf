//! `FormatHandler` and `WorkspaceDiscoverer` traits — the pull-API
//! that replaced the legacy `Ecosystem` push-API.
//!
//! Three responsibilities, three places:
//!
//! - [`FormatHandler`] — file-format I/O for one canonical manifest.
//!   Decides whether a path *is* one of "its" manifests
//!   ([`FormatHandler::is_manifest_file`]), parses a version out of
//!   that file's content ([`FormatHandler::parse_version`]), and
//!   builds a [`Rewriter`] for releasing it.
//!
//! - [`WorkspaceDiscoverer`] — multi-package discovery for ecosystems
//!   that have a workspace concept (cargo metadata, npm `workspaces`
//!   field, maven `<modules>`). Single-package ecosystems
//!   (go/swift/elixir/pypa/csproj) don't implement this.
//!
//! - [`crate::core::release_unit::discovery`] — the orchestrator. Walks
//!   the repo, dispatches each manifest path to a `WorkspaceDiscoverer`
//!   if one claims it, otherwise to the matching `FormatHandler`'s
//!   default single-file discovery.

use crate::core::{
    errors::Result,
    git::repository::{RepoPath, RepoPathBuf, Repository},
    release_unit::VersionFieldSpec,
    resolved_release_unit::{DepRequirement, ReleaseUnitId},
    rewriters::Rewriter,
    version::Version,
};

// ---------------------------------------------------------------------------
// FormatHandler — file-format I/O.
// ---------------------------------------------------------------------------

/// Per-ecosystem file-format handler. Stateless.
pub trait FormatHandler: Send + Sync + std::fmt::Debug {
    /// Stable wire-format name (e.g. `"cargo"`, `"npm"`). Must match
    /// the string used as `qualified_names()[1]` and as the
    /// `ecosystem` field in the manifest.
    fn name(&self) -> &'static str;

    /// Human-facing label for CLI / PR-body output.
    fn display_name(&self) -> &'static str;

    /// True if `path` is one of the canonical manifest files this
    /// handler claims responsibility for. Used by the repo-walking
    /// orchestrator to dispatch each scanned path to at most one
    /// handler.
    fn is_manifest_file(&self, path: &RepoPath) -> bool;

    /// Extract the version string from one canonical manifest's
    /// content. Pure: no Repo coupling, no I/O. Drives both single-
    /// file discovery and the configured-RU initial-version read.
    fn parse_version(&self, content: &str) -> Result<String>;

    /// The [`VersionFieldSpec`] whose `read`/`write` round-trip
    /// matches this handler's canonical manifest. Configured
    /// `[release_unit.X]` blocks fall back to this when no per-
    /// manifest `version_field` is declared.
    fn default_version_field(&self) -> VersionFieldSpec;

    /// Default tag template for releases in this ecosystem.
    fn tag_format_default(&self) -> &'static str {
        "{name}-v{version}"
    }

    /// Whitelist of template variables this ecosystem accepts in
    /// `tag_format`.
    fn tag_template_vars(&self) -> &'static [&'static str] {
        &["name", "version", "ecosystem"]
    }

    /// Construct a [`Rewriter`] that updates `manifest_path` when
    /// the unit at `unit_id` is bumped. Override for ecosystems that
    /// also need to rewrite internal-dep version requirements (cargo,
    /// npm) or coordinate satellite files.
    fn make_rewriter(
        &self,
        unit_id: ReleaseUnitId,
        manifest_path: RepoPathBuf,
    ) -> Box<dyn Rewriter>;

    /// Build a [`DiscoveredUnit`] for one canonical manifest in
    /// single-package mode (no workspace context). Returns `Ok(None)`
    /// when the path is recognised by `is_manifest_file` but doesn't
    /// describe an emittable unit on its own — virtual cargo
    /// workspace roots (no `[package]`), pypa-style nested
    /// `setup.cfg` without metadata, etc. The orchestrator treats
    /// `Ok(None)` as a clean skip.
    fn discover_single(
        &self,
        repo: &Repository,
        manifest_path: &RepoPath,
    ) -> Result<Option<DiscoveredUnit>>;
}

// ---------------------------------------------------------------------------
// WorkspaceDiscoverer — multi-package walks. Only cargo/npm/maven.
// ---------------------------------------------------------------------------

/// Per-ecosystem workspace walker. Implementers are the three
/// ecosystems with native workspace protocols: cargo (metadata),
/// npm (`workspaces` field), maven (`<modules>`). Stateless.
pub trait WorkspaceDiscoverer: Send + Sync + std::fmt::Debug {
    /// Discoverer's stable label for diagnostics.
    fn name(&self) -> &'static str;

    /// True if `manifest_path` is the *root* of a workspace this
    /// discoverer recognises. Implementations may need to read the
    /// file's content to decide (`Cargo.toml` with a `[workspace]`
    /// table, `package.json` with a `workspaces` field).
    fn claims(&self, repo: &Repository, manifest_path: &RepoPath) -> bool;

    /// Discover every unit reachable from `root_path`.
    ///
    /// A workspace protocol enumerates *every* member, including the ones a
    /// `[release_unit.X]` block already covers — the orchestrator's path-level
    /// skip-list cannot filter those, because they arrive from one call on the
    /// workspace root rather than as separate scanned paths. `ownership` is how
    /// an implementation tells the two apart: a member it claims must not be
    /// emitted as a unit of its own (that would put a second graph node on the
    /// same directory), but its dependency edges still belong to the claiming
    /// unit and are returned as [`DiscoveryBatch::claimed_deps`].
    fn discover(
        &self,
        repo: &Repository,
        root_path: &RepoPath,
        ownership: &UnitOwnership,
    ) -> Result<DiscoveryBatch>;
}

// ---------------------------------------------------------------------------
// UnitOwnership — which repo paths a configured `[release_unit.X]` covers.
// ---------------------------------------------------------------------------

/// Who owns a repo path, as far as discovery is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathOwner<'a> {
    /// A configured `[release_unit.<name>]` block covers this path. The
    /// payload is that block's unit name — the name its graph node carries,
    /// which is what a dependency edge onto it must target.
    Unit(&'a str),

    /// An `[ignore_paths]` entry covers this path. Nothing here is a unit and
    /// nothing here is a valid dependency target.
    Ignored,
}

/// The repo paths already spoken for before auto-discovery runs: every
/// configured unit's manifest directories, satellites and literal `paths`,
/// plus the `[ignore_paths]` list.
///
/// Doubles as the scan skip-list (via [`Self::paths`]) and as the
/// package→unit map workspace discoverers use to redirect dependency edges
/// onto the owning unit.
#[derive(Debug, Default, Clone)]
pub struct UnitOwnership {
    /// Sorted longest-path-first so [`Self::owner_of`] resolves the most
    /// specific claim: a satellite nested inside another unit's tree belongs
    /// to the satellite's owner, not the outer one.
    claims: Vec<(RepoPathBuf, Option<String>)>,
}

impl UnitOwnership {
    /// Build from `(path, owner)` pairs. `None` marks an ignored path.
    pub fn new(claims: impl IntoIterator<Item = (RepoPathBuf, Option<String>)>) -> Self {
        let mut claims: Vec<_> = claims.into_iter().collect();
        claims.sort_by(|a, b| {
            let (a_bytes, b_bytes): (&[u8], &[u8]) = (a.0.as_ref(), b.0.as_ref());
            b_bytes.len().cmp(&a_bytes.len()).then(a_bytes.cmp(b_bytes))
        });
        Self { claims }
    }

    /// The owner of `path`, where "owner" means a claim that is `path` itself
    /// or one of its ancestor directories. `None` when nothing claims it.
    pub fn owner_of(&self, path: &RepoPath) -> Option<PathOwner<'_>> {
        self.claims
            .iter()
            .find(|(claim, _)| is_path_inside_any(path, std::slice::from_ref(claim)))
            .map(|(_, owner)| match owner {
                Some(name) => PathOwner::Unit(name),
                None => PathOwner::Ignored,
            })
    }

    /// Every claimed path — the skip-list the repo walk filters against.
    pub fn paths(&self) -> impl Iterator<Item = &RepoPathBuf> {
        self.claims.iter().map(|(p, _)| p)
    }

    pub fn is_empty(&self) -> bool {
        self.claims.is_empty()
    }
}

// ---------------------------------------------------------------------------
// DiscoveryBatch — what one discovery pass produced.
// ---------------------------------------------------------------------------

/// The output of a discovery pass: units discovery owns outright, plus
/// dependency edges whose *depender* is a configured unit that is already a
/// graph node by the time discovery runs.
#[derive(Debug, Default)]
pub struct DiscoveryBatch {
    pub units: Vec<DiscoveredUnit>,
    pub claimed_deps: Vec<ClaimedDep>,
}

impl DiscoveryBatch {
    /// Convenience for the single-manifest paths that never produce claimed
    /// edges.
    pub fn from_units(units: Vec<DiscoveredUnit>) -> Self {
        Self {
            units,
            claimed_deps: Vec::new(),
        }
    }

    pub fn extend(&mut self, other: DiscoveryBatch) {
        self.units.extend(other.units);
        self.claimed_deps.extend(other.claimed_deps);
    }
}

/// A dependency edge belonging to a configured `[release_unit.X]`.
#[derive(Debug, Clone)]
pub struct ClaimedDep {
    /// Name of the configured unit the edge hangs off.
    pub depender_unit: String,

    /// Ecosystem the edge was read from — the discoverer's
    /// [`WorkspaceDiscoverer::name`]. Used to bind `dep.target_package_name`
    /// to the right node when two ecosystems host a same-named package.
    pub ecosystem: &'static str,

    pub dep: RawInternalDep,
}

// ---------------------------------------------------------------------------
// DiscoveredUnit — what discovery emits, before graph IDs are assigned.
// ---------------------------------------------------------------------------

/// Build a [`Rewriter`] for a unit once its `ReleaseUnitId` is known.
pub type RewriterFactory = Box<dyn FnOnce(ReleaseUnitId) -> Box<dyn Rewriter> + Send>;

pub struct DiscoveredUnit {
    /// Qualified names; `qnames[0]` is the package-manager-native
    /// name, `qnames[1]` is the ecosystem (must equal the
    /// `FormatHandler::name()`).
    pub qnames: Vec<String>,
    /// Current version read from the manifest.
    pub version: Version,
    /// The directory the unit lives in (parent of `anchor_manifest`).
    pub prefix: RepoPathBuf,
    /// The canonical manifest path (the file `is_manifest_file`
    /// matched). Used for tag-format precedence.
    pub anchor_manifest: RepoPathBuf,
    /// Closures that produce the unit's rewriters once its
    /// `ReleaseUnitId` is known. Multi-rewriter cases (pypa
    /// `annotated_files`, csproj `AssemblyInfo.cs`, multi-manifest
    /// bundles) push more than one.
    pub rewriter_factories: Vec<RewriterFactory>,
    /// Internal-dependency edges this unit declares. The graph
    /// builder resolves `target_package_name` against the global
    /// name-to-id map after every unit is registered.
    pub internal_deps: Vec<RawInternalDep>,
}

impl std::fmt::Debug for DiscoveredUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("DiscoveredUnit")
            .field("qnames", &self.qnames)
            .field("version", &self.version)
            .field("prefix", &self.prefix)
            .field("anchor_manifest", &self.anchor_manifest)
            .field("rewriter_count", &self.rewriter_factories.len())
            .field("internal_deps", &self.internal_deps)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct RawInternalDep {
    pub target_package_name: String,
    pub literal: String,
    pub requirement: DepRequirement,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `true` if `path` equals or is inside (strict child of)
/// any path in `skip_list`.
pub fn is_path_inside_any(path: &RepoPath, skip_list: &[RepoPathBuf]) -> bool {
    let path_bytes = path.as_ref();
    skip_list.iter().any(|s| {
        let s_bytes: &[u8] = s.as_ref();
        if path_bytes == s_bytes {
            return true;
        }
        if path_bytes.len() > s_bytes.len()
            && path_bytes.starts_with(s_bytes)
            && path_bytes[s_bytes.len()] == b'/'
        {
            return true;
        }
        false
    })
}

/// Parse a version string with the right type for an ecosystem
/// (PEP 440 for pypa, semver for the rest).
pub fn parse_version_string(version_str: &str, ecosystem_name: &str) -> Result<Version> {
    let trimmed = version_str.trim();
    if ecosystem_name == "pypa" {
        Ok(Version::Pep440(
            trimmed
                .parse()
                .map_err(|e| anyhow::anyhow!("not PEP 440: {e}"))?,
        ))
    } else {
        Ok(Version::Semver(
            semver::Version::parse(trimmed).map_err(|e| anyhow::anyhow!("not semver: {e}"))?,
        ))
    }
}

// ---------------------------------------------------------------------------
// Registries.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct FormatHandlerRegistry {
    handlers: Vec<Box<dyn FormatHandler>>,
}

impl FormatHandlerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        r.register(Box::new(super::cargo::CargoLoader));
        r.register(Box::new(super::npm::NpmLoader));
        r.register(Box::new(super::go::GoLoader));
        r.register(Box::new(super::swift::SwiftLoader));
        r.register(Box::new(super::elixir::ElixirLoader));
        r.register(Box::new(super::maven::MavenLoader));
        r.register(Box::new(super::pypa::PypaLoader));
        #[cfg(feature = "csharp")]
        r.register(Box::new(super::csproj::CsProjLoader));
        r
    }

    pub fn register(&mut self, handler: Box<dyn FormatHandler>) {
        self.handlers.push(handler);
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.handlers.iter().map(|h| h.name()).collect()
    }

    pub fn lookup(&self, name: &str) -> Option<&dyn FormatHandler> {
        self.handlers
            .iter()
            .find(|h| h.name() == name)
            .map(|h| h.as_ref())
    }

    /// Find the handler that claims `path` via
    /// [`FormatHandler::is_manifest_file`]. First-match wins; if
    /// multiple handlers could claim the same path the registration
    /// order in `with_defaults` decides.
    pub fn handler_for(&self, path: &RepoPath) -> Option<&dyn FormatHandler> {
        self.handlers
            .iter()
            .find(|h| h.is_manifest_file(path))
            .map(|h| h.as_ref())
    }

    pub fn handlers(&self) -> impl Iterator<Item = &dyn FormatHandler> {
        self.handlers.iter().map(|h| h.as_ref())
    }
}

#[derive(Debug, Default)]
pub struct WorkspaceDiscovererRegistry {
    discoverers: Vec<Box<dyn WorkspaceDiscoverer>>,
}

impl WorkspaceDiscovererRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        r.register(Box::new(super::cargo::CargoWorkspaceDiscoverer));
        r.register(Box::new(super::npm::NpmWorkspaceDiscoverer));
        r.register(Box::new(super::maven::MavenWorkspaceDiscoverer));
        r
    }

    pub fn register(&mut self, d: Box<dyn WorkspaceDiscoverer>) {
        self.discoverers.push(d);
    }

    pub fn discoverers(&self) -> impl Iterator<Item = &dyn WorkspaceDiscoverer> {
        self.discoverers.iter().map(|d| d.as_ref())
    }
}
