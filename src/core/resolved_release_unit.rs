// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Information about a single project within the repository.
//!
//! Here, a project is defined as something that’s assigned version numbers.
//! Many repositories contain only a single project, but in the general case
//! (i.e., a monorepo) there can be many projects within a single repo, with
//! interdependencies inducing a Directed Acyclic Graph (DAG) structure on them,
//! as implemented in the `graph` module.

use anyhow::{anyhow, bail};

use crate::core::{
    errors::Result,
    git::repository::{CommitId, PathMatcher, RepoPath, RepoPathBuf},
    rewriters::Rewriter,
    version::Version,
};

/// An internal, unique identifier for a project in this app session.
///
/// These identifiers should not be persisted and are not guaranteed to have any
/// particular semantics other than being cheaply copyable.
pub type ReleaseUnitId = usize;

/// How a unit participates in the release model (F1). Deliberately a
/// separate axis from [`crate::core::release_unit::Visibility`]:
/// `Visibility::Internal` means "versioned + in the manifest, just no git
/// tag", whereas `UnitKind::Internal` means **no version, tag, release, or
/// manifest entry at all** — a pure cascade node in the dependency graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UnitKind {
    /// Released: versioned, tagged, emitted to the manifest. Root of a
    /// dependency closure. The back-compat default.
    #[default]
    Deploy,

    /// A named graph node that cascades to the `deploy` units depending on
    /// it but is **never** versioned/tagged/released (e.g. internal library
    /// crates, `proto/`). Participates in closures and path attribution.
    Internal,

    /// Skipped entirely — not a candidate, not a closure root, not traversed
    /// into (e.g. flat test crates like `apps/services/e2e`).
    Ignore,
}

#[derive(Debug)]
pub struct ResolvedReleaseUnit {
    ident: ReleaseUnitId,

    /// Qualified names. The package name, qualified with hierarchical
    /// indicators. The first item in the vector is the most specific name and
    /// the one that the user is most likely to recognize as corresponding to
    /// the project. Additional terms become more and more general and can be
    /// used to disambiguate packages originating from different schemes: e.g.,
    /// a repo containing related Python and NPM packages that both have the
    /// same name.
    qnames: Vec<String>,

    /// The user-facing package name; this is computed from the qualified names
    /// after all of the projects are loaded, so that we can make sure that
    /// these are unique.
    pub user_facing_name: String,

    /// The version associated with this project.
    pub version: Version,

    /// Steps to perform when rewriting this project's metadata to produce
    /// a release commit.
    pub rewriters: Vec<Box<dyn Rewriter>>,

    /// The project's unique prefix in the repository.
    ///
    /// Should be empty if the prefix is the project root. Otherwise, should end
    /// with a trailing slash for easy path combination.
    ///
    /// Note that actual path relevance matching should be done using the
    /// `repo_paths` field, to handle common cases where a project has
    /// sub-projects contained in subdirectories. When matching paths we will
    /// generally want to exclude the sub-projects, which requires more
    /// sophistication than a simple prefix match.
    prefix: RepoPathBuf,

    /// A data structure describing the paths inside the repository that are
    /// considered to affect this project.
    pub repo_paths: PathMatcher,

    /// This project's internal dependencies.
    pub internal_deps: Vec<Dependency>,

    /// How this unit participates in the release model (F1). `Deploy` units
    /// are versioned/tagged/released; `Internal` units are pure cascade
    /// nodes; `Ignore` units are skipped entirely.
    pub kind: UnitKind,

    /// Per-unit bump-policy override (F11a). `None` = inherit global `[bump]`.
    pub bump_override: Option<crate::core::release_unit::syntax::BumpOverrideConfig>,

    /// True for the synthetic nodes materialized from `[cascade_inputs.<name>]`.
    ///
    /// These are always [`UnitKind::Internal`], but not every `Internal` unit
    /// is a cascade input (an internal library crate is one too), and the two
    /// need different path-partition rules: an input's paths are *meant* to
    /// overlap the real units they feed, so inputs are exempt from
    /// `make_disjoint` and from the Tier-3 glob overlap guard. Keeping the
    /// distinction explicit rather than sniffing `qnames[1]` keeps that
    /// behaviour from silently attaching to user-declared internal units.
    pub is_cascade_input: bool,
}

impl ResolvedReleaseUnit {
    /// Get the internal unique identifier of this project.
    ///
    /// These identifiers should not be persisted and are not guaranteed to have
    /// any particular semantics other than being cheaply copyable.
    pub fn ident(&self) -> ReleaseUnitId {
        self.ident
    }

    /// Get a reference to this project's full qualified names.
    pub fn qualified_names(&self) -> &Vec<String> {
        &self.qnames
    }

    /// Get this project's prefix in the repository filesystem.
    ///
    /// To check whether a particular path is relevant to this project, use the
    /// `repo_paths` field, which will properly account for any projects in
    /// subdirectorie relative to this project.
    pub fn prefix(&self) -> &RepoPath {
        &self.prefix
    }
}

/// Metadata about internal interdependencies between projects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dependency {
    /// The project that is depended upon
    pub ident: ReleaseUnitId,

    /// The current expression of the requirement in the project metadata files.
    /// In normal operations this should be an explicit requirement on version
    /// "0.0.0-dev.0", or the equivalent, so that the project can be built on
    /// the main branch.
    pub literal: String,

    /// The logical expression of the requirement in Belaf's framework. Belaf
    /// prefers to express version dependencies in terms of commit IDs. Since
    /// this concept is (properly) not integrated into package manager metadata
    /// files, the information expressing the requirement must be recorded in
    /// Belaf-specific metadata that are different than the literal expression.
    pub belaf_requirement: DepRequirement,

    /// If the requirement is expressed as a DepRequirement::Commit, *and* we
    /// have resolved that requirement to a specific version of the dependee
    /// project, that version is stored here. None values could be found if the
    /// requirement is not a commit or if the resolution process hasn't
    /// occurred, or if resolution failed.
    pub resolved_version: Option<Version>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DepRequirement {
    /// The depending project requires a version of the dependee project later
    /// than the specified commit.
    Commit(CommitId),

    /// The depending project requires some version of the dependee project that
    /// has been manually specified by the user. This is discouraged, but
    /// necessary to support to enable bootstrapping. Note that the value of
    /// this manual specification is not redundant with `Dependency::literal`:
    /// in steady-state, the former will be something like `0.0.0-dev.0` so that
    /// everyday builds can work, while this might be `^0.1` if the project
    /// requires that version of its dependency and 0.1 was released before
    /// Belaf was introduced.
    Manual(String),

    /// Belaf metadata are missing, so we can't process this dependency in the
    /// Belaf framework.
    Unavailable,
}

impl std::fmt::Display for DepRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DepRequirement::Commit(cid) => write!(f, "{cid} (commit)"),
            DepRequirement::Manual(t) => write!(f, "{t} (manual)"),
            DepRequirement::Unavailable => write!(f, "(unavailable)"),
        }
    }
}

/// A builder for initializing a new project entry that will be added to the
/// graph.
#[derive(Debug)]
pub struct ResolvedReleaseUnitBuilder {
    pub qnames: Vec<String>,
    pub version: Option<Version>,
    pub prefix: Option<RepoPathBuf>,
    pub rewriters: Vec<Box<dyn Rewriter>>,
    pub internal_deps: Vec<DependencyBuilder>,
    /// See [`ResolvedReleaseUnit::kind`]. Defaults to `Deploy`.
    pub kind: UnitKind,
    /// Extra repo-relative path prefixes to include in `repo_paths` beyond
    /// the unit's primary `prefix`. Used by multi-path units (e.g. a
    /// `paths`-only internal unit covering several directories).
    pub extra_includes: Vec<RepoPathBuf>,
    /// Residual glob patterns (Tier-3, F4-Glob) for manifest-less units whose
    /// `paths` aren't simple prefixes (e.g. `**/*.sql`). Compiled into
    /// `repo_paths` at `finalize`.
    pub extra_globs: Vec<String>,
    /// When set, `finalize` does NOT add the `prefix` as an `Include` term —
    /// the unit matches purely via `extra_globs`/`extra_includes`. Used by
    /// glob-only `paths` units, where a prefix `Include` would over-match.
    pub repo_paths_no_prefix_include: bool,
    /// See [`ResolvedReleaseUnit::bump_override`].
    pub bump_override: Option<crate::core::release_unit::syntax::BumpOverrideConfig>,
    /// See [`ResolvedReleaseUnit::is_cascade_input`].
    pub is_cascade_input: bool,
}

/// An in-process dependency. We haven't necessarily yet resolved references to
/// project ids.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyBuilder {
    pub target: DependencyTarget,
    pub literal: String,
    pub belaf_requirement: DepRequirement,
    pub resolved_version: Option<Version>,
}

/// The target of a DependencyBuilder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyTarget {
    /// The target expressed as user-specified text that will be resolved to a
    /// user-facing name. Use this for dependencies manually specified by the
    /// user that might refer to any packaging system.
    Text(String),

    /// The target expressed as a known ReleaseUnitId. This is generally only
    /// possible for dependencies within the same packaging system.
    Ident(ReleaseUnitId),
}

impl ResolvedReleaseUnitBuilder {
    #[doc(hidden)]
    pub(crate) fn new() -> Self {
        ResolvedReleaseUnitBuilder {
            qnames: Vec::new(),
            version: None,
            prefix: None,
            rewriters: Vec::new(),
            internal_deps: Vec::new(),
            kind: UnitKind::Deploy,
            extra_includes: Vec::new(),
            extra_globs: Vec::new(),
            repo_paths_no_prefix_include: false,
            bump_override: None,
            is_cascade_input: false,
        }
    }

    #[doc(hidden)]
    pub(crate) fn finalize(
        self,
        ident: ReleaseUnitId,
        user_facing_name: String,
        internal_deps: Vec<Dependency>,
    ) -> Result<ResolvedReleaseUnit> {
        if self.qnames.is_empty() {
            bail!(
                "could not load project `{}`: never figured out its naming",
                user_facing_name
            );
        }

        let version = self.version.ok_or_else(|| {
            anyhow!(
                "could not load project `{}`: never figured out its version",
                user_facing_name
            )
        })?;

        let prefix = self.prefix.ok_or_else(|| {
            anyhow!(
                "could not load project `{}`: never figured out its directory prefix",
                user_facing_name
            )
        })?;

        let mut repo_paths = if self.repo_paths_no_prefix_include {
            PathMatcher::new_globs_only()
        } else {
            PathMatcher::new_include(prefix.clone())
        };
        for extra in self.extra_includes {
            repo_paths.add_include(extra);
        }
        for pattern in &self.extra_globs {
            repo_paths.add_glob(pattern)?;
        }

        Ok(ResolvedReleaseUnit {
            ident,
            qnames: self.qnames,
            user_facing_name,
            version,
            prefix: prefix.clone(),
            rewriters: self.rewriters,
            repo_paths,
            internal_deps,
            kind: self.kind,
            bump_override: self.bump_override,
            is_cascade_input: self.is_cascade_input,
        })
    }
}
