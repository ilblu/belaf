//! `FormatHandler` trait: per-language file-format handler +
//! workspace discoverer.
//!
//! Three responsibilities, kept separate:
//!
//! - **File-format I/O** lives in [`crate::core::version_field`]
//!   (free-function dispatch keyed off `VersionFieldSpec`).
//! - **Workspace discovery** is the handler's job: walk the repo
//!   and emit [`DiscoveredUnit`] records for every manifest not
//!   already claimed by a `[release_unit.X]` config block.
//! - **Graph construction** is the session's job. The
//!   [`AppBuilder`] consumes both configured
//!   `release_unit::ResolvedReleaseUnit`s (from the resolver) and
//!   discovered `DiscoveredUnit`s, and feeds both into the graph
//!   builder. Handlers stay stateless.
//!
//! [`AppBuilder`]: crate::core::session::AppBuilder

use crate::core::{
    errors::Result,
    git::repository::{RepoPath, RepoPathBuf, Repository},
    release_unit::VersionFieldSpec,
    resolved_release_unit::{DepRequirement, ReleaseUnitId},
    rewriters::Rewriter,
    version::Version,
};

/// Per-language file-format handler + workspace discoverer.
///
/// Stateless. A fresh handler per session is fine; loaders no longer
/// accumulate per-scan state.
pub trait FormatHandler: Send + Sync + std::fmt::Debug {
    /// Stable wire-format name (e.g. `"cargo"`, `"npm"`). Must match
    /// the string used as `qualified_names()[1]` and as the
    /// `ecosystem` field in the manifest.
    fn name(&self) -> &'static str;

    /// Human-facing label for CLI / PR-body output.
    fn display_name(&self) -> &'static str;

    /// Canonical manifest filename for this ecosystem (e.g.
    /// `"Cargo.toml"`, `"package.json"`).
    fn manifest_filename(&self) -> &'static str;

    /// The `VersionFieldSpec` that maps to this ecosystem's canonical
    /// manifest. Used by configured-RU initialization to read the
    /// initial version.
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

    /// Construct a `Rewriter` that updates the manifest at
    /// `manifest_path` when the unit at `unit_id` is bumped.
    /// Override for ecosystems that also rewrite internal-dep
    /// version requirements (cargo, npm).
    fn make_rewriter(
        &self,
        unit_id: ReleaseUnitId,
        manifest_path: RepoPathBuf,
    ) -> Box<dyn Rewriter>;

    /// Walk the repo for unconfigured units of this ecosystem.
    ///
    /// `configured_skip_paths` is a list of repo-relative directory
    /// paths already claimed by `[release_unit.X]` blocks (their
    /// manifest parents + satellites). Implementations must skip any
    /// manifest at-or-inside one of those paths.
    ///
    /// Workspace-aware ecosystems (cargo via `cargo metadata`, npm
    /// via `package.json` workspaces) override this to walk natively.
    /// Single-manifest ecosystems can use the default scan via
    /// [`scan_index_for_filename`].
    fn discover_units(
        &self,
        repo: &Repository,
        configured_skip_paths: &[RepoPathBuf],
    ) -> Result<Vec<DiscoveredUnit>>;
}

/// Build a [`Rewriter`] for a unit once its `ReleaseUnitId` is known.
///
/// `discover_units` doesn't have IDs yet (they're assigned by
/// `ReleaseUnitGraphBuilder::add_project`), so each unit carries one
/// or more closures that produce rewriters at registration time.
pub type RewriterFactory = Box<dyn FnOnce(ReleaseUnitId) -> Box<dyn Rewriter> + Send>;

/// A unit discovered by a [`FormatHandler`]. Carries everything the
/// session's [`AppBuilder`] needs to register the unit in the graph.
///
/// [`AppBuilder`]: crate::core::session::AppBuilder
pub struct DiscoveredUnit {
    /// Qualified names; `qnames[0]` is the package-manager-native
    /// name, `qnames[1]` is the ecosystem (must equal the
    /// FormatHandler's `name()`), additional entries help disambiguate
    /// monorepo collisions.
    pub qnames: Vec<String>,
    /// Current version read from the manifest.
    pub version: Version,
    /// The directory `qnames[0]` lives in (parent of
    /// `anchor_manifest`). Empty for repo-root manifests.
    pub prefix: RepoPathBuf,
    /// The canonical manifest path. Used for tag-format precedence
    /// and as the entry surfaced in CLI output. Bundles with multiple
    /// manifests still declare one anchor.
    pub anchor_manifest: RepoPathBuf,
    /// Closures that produce the unit's rewriters once its
    /// `ReleaseUnitId` is known. PyPA units typically have multiple
    /// (e.g. `pyproject.toml` + several `# belaf project-version`
    /// annotated `.py` files); single-file ecosystems have one.
    pub rewriter_factories: Vec<RewriterFactory>,
    /// Internal-dependency edges this unit declares, expressed as
    /// "this unit depends on the package called X". Resolved against
    /// the global name-to-id map at `complete_loading` time.
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

/// A pre-resolution internal dependency. The graph builder turns
/// `target_package_name` into a `ReleaseUnitId` after every unit is
/// in the builder.
#[derive(Debug, Clone)]
pub struct RawInternalDep {
    /// Package-manager-native name of the target unit (matches some
    /// other [`DiscoveredUnit::qnames`]`[0]`).
    pub target_package_name: String,
    /// The textual version requirement as declared in this unit's
    /// manifest (e.g. `"^1.2"` for npm, `"1.2"` for cargo).
    pub literal: String,
    /// belaf's logical requirement (commit ID, manual override, …).
    /// See [`DepRequirement`].
    pub requirement: DepRequirement,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Helper for single-manifest FormatHandlers (no workspace concept):
/// walks the git index, returns every path whose basename matches
/// `manifest_filename` and isn't inside any `skip_paths` entry.
pub fn scan_index_for_filename(
    repo: &Repository,
    manifest_filename: &str,
    skip_paths: &[RepoPathBuf],
) -> Result<Vec<RepoPathBuf>> {
    let mut out = Vec::new();
    repo.scan_paths(|p| {
        let (_dirname, basename) = p.split_basename();
        if basename.as_ref() == manifest_filename.as_bytes() && !is_path_inside_any(p, skip_paths) {
            out.push(p.to_owned());
        }
        Ok(())
    })?;
    Ok(out)
}

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

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Owning collection of [`FormatHandler`] implementations. Stateless
/// after construction.
#[derive(Debug, Default)]
pub struct FormatHandlerRegistry {
    handlers: Vec<Box<dyn FormatHandler>>,
}

impl FormatHandlerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a registry with every ecosystem belaf ships with.
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

    pub fn handlers(&self) -> impl Iterator<Item = &dyn FormatHandler> {
        self.handlers.iter().map(|h| h.as_ref())
    }
}

// ---------------------------------------------------------------------------
// discover_implicit_release_units — the new auto-discovery entry point.
// ---------------------------------------------------------------------------

/// Walk the repo for every unconfigured manifest of every registered
/// ecosystem. Skips paths covered by configured `[release_unit.X]`
/// blocks (passed via `configured_skip_paths`).
///
/// Each ecosystem's `discover_units` is called once. The returned
/// `Vec<DiscoveredUnit>` is the union, in registration order.
pub fn discover_implicit_release_units(
    repo: &Repository,
    registry: &FormatHandlerRegistry,
    configured_skip_paths: &[RepoPathBuf],
) -> Result<Vec<DiscoveredUnit>> {
    let mut all = Vec::new();
    for h in registry.handlers() {
        all.extend(h.discover_units(repo, configured_skip_paths)?);
    }
    Ok(all)
}
