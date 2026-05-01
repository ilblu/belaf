//! Ecosystem trait + registry.
//!
//! Every language belaf supports implements [`Ecosystem`]. The
//! [`EcosystemRegistry`] owns a `Vec<Box<dyn Ecosystem>>` and dispatches the
//! two lifecycle hooks — `process_index_item` (per file in the git index) and
//! `finalize` (consume + register projects in the graph).
//!
//! Adding a new ecosystem is one new `impl Ecosystem for FooLoader` block plus
//! one `register(...)` call in [`EcosystemRegistry::with_defaults`]. There is
//! no central `match` to extend, no `EcosystemType` enum to widen — that was
//! the v1.x design and it leaked the closed-set assumption into every site
//! that touched ecosystems (and silently dropped Swift in `from_qname`).
//!
//! See plan §11 for the full spec.

use crate::core::{
    errors::Result,
    git::repository::{RepoPath, RepoPathBuf, Repository},
    graph::ReleaseUnitGraphBuilder,
    session::AppBuilder,
};

/// Returns `true` if `path` equals or is inside (strict child of) any
/// path in `skip_list`. Used to silence ecosystem-level scanning for
/// paths already covered by a `[[release_unit]]` (its manifests +
/// satellites). Phase C.
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

/// Per-language loader/rewriter contract.
///
/// Lifecycle: a fresh implementation is registered with the
/// [`EcosystemRegistry`] at session start. Each git-index entry is fed into
/// [`Ecosystem::process_index_item`] (loaders pick what they care about and
/// stash it). After the scan, [`Ecosystem::finalize`] consumes the loader and
/// pushes the resulting projects + rewriters into the graph.
pub trait Ecosystem: Send + Sync + std::fmt::Debug {
    /// Stable wire-format name (e.g. `"cargo"`, `"npm"`). Must match the
    /// string used as `qualified_names()[1]` and as the `ecosystem` field in
    /// the manifest.
    fn name(&self) -> &'static str;

    /// Human-facing label for CLI / PR-body output (e.g. `"Rust (Cargo)"`).
    fn display_name(&self) -> &'static str;

    /// Path or filename pattern of the canonical version-bearing file.
    /// Used by the wizard to nudge users toward the right file when
    /// scaffolding a project.
    fn version_file(&self) -> &'static str;

    /// Default tag template for releases in this ecosystem (B10).
    /// Variables in `{...}` are resolved at tag-emission time. Override
    /// per-project / per-group via `belaf/config.toml`.
    fn tag_format_default(&self) -> &'static str {
        "{name}-v{version}"
    }

    /// Whitelist of template variables this ecosystem accepts in
    /// `tag_format`. CLI hard-errors if an override uses an unsupported
    /// variable for this ecosystem (e.g. `{groupId}` for npm).
    fn tag_template_vars(&self) -> &'static [&'static str] {
        &["name", "version", "ecosystem"]
    }

    /// Called once per file in the git index. Loaders inspect `basename`
    /// (and optionally `dirname` / `repopath`) and stash anything they
    /// recognise for later. Default: ignore everything.
    fn process_index_item(
        &mut self,
        _repo: &Repository,
        _graph: &mut ReleaseUnitGraphBuilder,
        _repopath: &RepoPath,
        _dirname: &RepoPath,
        _basename: &RepoPath,
    ) -> Result<()> {
        Ok(())
    }

    /// Consume the loader and register its discovered projects +
    /// rewriters with the [`AppBuilder`]. Runs after the index scan,
    /// in registry insertion order.
    fn finalize(self: Box<Self>, app: &mut AppBuilder) -> Result<()>;

    /// Receive the resolved release-unit skip-list for this session.
    /// Called before any `process_index_item` / `finalize` work.
    /// Loaders that perform their own workspace enumeration (e.g.
    /// the Cargo loader's `cargo metadata` walk in `finalize`)
    /// override this to filter accordingly.
    fn set_skip_list(&mut self, _skip_list: &[RepoPathBuf]) {}
}

/// Owning collection of ecosystem implementations.
///
/// Insertion order is meaningful: `process_index_item` and `finalize` both
/// iterate in registration order, which lets later loaders observe earlier
/// loaders' outputs via the graph.
#[derive(Debug, Default)]
pub struct EcosystemRegistry {
    ecosystems: Vec<Box<dyn Ecosystem>>,
    /// Paths already covered by resolved `[[release_unit]]`s.
    ///
    /// Set via [`Self::set_skip_list`] before
    /// [`Self::process_index_item`] runs. Loaders ignore index entries
    /// inside any of these paths so the ReleaseUnit's atomic claim on
    /// the directory is respected.
    skip_list: Vec<RepoPathBuf>,
}

impl EcosystemRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a registry pre-populated with every ecosystem belaf ships with.
    /// Adding a new built-in is one line here plus the new `impl Ecosystem`
    /// block — no other site in the codebase needs editing.
    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        r.register(Box::new(super::cargo::CargoLoader::default()));
        r.register(Box::new(super::npm::NpmLoader::default()));
        r.register(Box::new(super::pypa::PypaLoader::default()));
        r.register(Box::new(super::go::GoLoader::default()));
        r.register(Box::new(super::elixir::ElixirLoader::default()));
        // v1.x bug regression: the old `EcosystemType::from_qname` enum
        // silently dropped Swift. The registry-based approach fixes it
        // structurally — Swift is right here in the default set.
        r.register(Box::new(super::swift::SwiftLoader::default()));
        r.register(Box::new(super::maven::MavenLoader::default()));
        #[cfg(feature = "csharp")]
        r.register(Box::new(super::csproj::CsProjLoader::default()));
        r
    }

    pub fn register(&mut self, eco: Box<dyn Ecosystem>) {
        self.ecosystems.push(eco);
    }

    /// Install the resolved release-unit skip-list. Forwards it to
    /// every registered loader via [`Ecosystem::set_skip_list`] so
    /// loaders that perform their own workspace enumeration (Cargo)
    /// can filter accordingly. Plan Phase C.
    pub fn set_skip_list(&mut self, skip_list: Vec<RepoPathBuf>) {
        for eco in &mut self.ecosystems {
            eco.set_skip_list(&skip_list);
        }
        self.skip_list = skip_list;
    }

    /// Read-only access to the configured skip-list. Useful for tests
    /// and `belaf config explain`.
    pub fn skip_list(&self) -> &[RepoPathBuf] {
        &self.skip_list
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.ecosystems.iter().map(|e| e.name()).collect()
    }

    pub fn lookup(&self, name: &str) -> Option<&dyn Ecosystem> {
        self.ecosystems
            .iter()
            .find(|e| e.name() == name)
            .map(|e| e.as_ref())
    }

    /// Dispatch one git-index entry through every registered
    /// ecosystem. Paths inside any `skip_list` entry are silently
    /// ignored so the ReleaseUnit's atomic claim on its directory is
    /// respected.
    pub fn process_index_item(
        &mut self,
        repo: &Repository,
        graph: &mut ReleaseUnitGraphBuilder,
        repopath: &RepoPath,
        dirname: &RepoPath,
        basename: &RepoPath,
    ) -> Result<()> {
        if is_path_inside_any(repopath, &self.skip_list) {
            return Ok(());
        }
        for eco in &mut self.ecosystems {
            eco.process_index_item(repo, graph, repopath, dirname, basename)?;
        }
        Ok(())
    }

    /// Consume all registered ecosystems, draining them into the graph.
    pub fn finalize_all(self, app: &mut AppBuilder) -> Result<()> {
        for eco in self.ecosystems {
            eco.finalize(app)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_include_swift() {
        // Regression for the v1.x `EcosystemType::from_qname` bug — the
        // closed enum silently dropped Swift. The registry default set
        // must include it.
        let r = EcosystemRegistry::with_defaults();
        assert!(
            r.names().contains(&"swift"),
            "Swift missing from default registry: {:?}",
            r.names()
        );
    }

    #[test]
    fn defaults_include_all_open_source_ecosystems() {
        let r = EcosystemRegistry::with_defaults();
        let names = r.names();
        for expected in &["cargo", "npm", "pypa", "go", "elixir", "swift"] {
            assert!(
                names.contains(expected),
                "missing ecosystem {expected:?} in {names:?}"
            );
        }
    }

    #[test]
    fn lookup_returns_registered_ecosystem() {
        let r = EcosystemRegistry::with_defaults();
        let cargo = r.lookup("cargo").expect("cargo registered");
        assert_eq!(cargo.name(), "cargo");
        assert_eq!(cargo.display_name(), "Rust (Cargo)");
    }

    #[test]
    fn lookup_unknown_returns_none() {
        let r = EcosystemRegistry::with_defaults();
        assert!(r.lookup("gradle").is_none());
    }

    // -----------------------------------------------------------------------
    // Phase C — skip-list helper coverage.
    // -----------------------------------------------------------------------

    #[test]
    fn is_path_inside_any_matches_exact_paths() {
        let list = vec![
            RepoPathBuf::new(b"apps/services/aura/crates"),
            RepoPathBuf::new(b"sdks/kotlin"),
        ];
        let path = RepoPathBuf::new(b"apps/services/aura/crates");
        assert!(is_path_inside_any(&path, &list));
    }

    #[test]
    fn is_path_inside_any_matches_strict_children() {
        let list = vec![RepoPathBuf::new(b"apps/services/aura/crates")];
        // child of an entry
        let child = RepoPathBuf::new(b"apps/services/aura/crates/api/Cargo.toml");
        assert!(is_path_inside_any(&child, &list));
        // deep child
        let deep = RepoPathBuf::new(b"apps/services/aura/crates/api/src/lib.rs");
        assert!(is_path_inside_any(&deep, &list));
    }

    #[test]
    fn is_path_inside_any_rejects_unrelated_paths() {
        let list = vec![RepoPathBuf::new(b"apps/services/aura/crates")];
        // sibling — wrong dir
        let sibling = RepoPathBuf::new(b"apps/services/ekko/crates/bin/Cargo.toml");
        assert!(!is_path_inside_any(&sibling, &list));
        // partial-prefix lookalike — apps/services/aura-extra (NOT
        // a child of aura/crates)
        let lookalike = RepoPathBuf::new(b"apps/services/aura-extra/Cargo.toml");
        assert!(!is_path_inside_any(&lookalike, &list));
    }

    #[test]
    fn is_path_inside_any_empty_list_never_matches() {
        let path = RepoPathBuf::new(b"some/deep/path");
        assert!(!is_path_inside_any(&path, &[]));
    }

    #[test]
    fn set_skip_list_propagates_to_loaders() {
        let mut r = EcosystemRegistry::with_defaults();
        let list = vec![RepoPathBuf::new(b"apps/services/aura/crates")];
        r.set_skip_list(list.clone());
        assert_eq!(r.skip_list().len(), 1);
        assert_eq!(r.skip_list()[0].escaped(), "apps/services/aura/crates");
    }
}
