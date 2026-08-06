//! Path filtering: [`PathMatcher`] (prefix include/exclude terms plus
//! residual glob patterns) and the `[binary_affecting]` relevance test
//! applied to each changed path.

use anyhow::Context;

use crate::core::{
    config::syntax::BinaryAffectingConfiguration,
    git::repository::{RepoPath, RepoPathBuf},
};

/// A filter that matches paths inside the repository and/or working directory.
///
/// We're not trying to get fully general here, but there is a common use case
/// that we need to support. A monorepo might contain a toplevel project, rooted
/// at the repo base, plus one or more subprojects in some kind of
/// subdirectories. For the toplevel project, we need to express a match for a
/// file anywhere in the repo *except* ones that match any of the subprojects.
/// Whether a changed path counts as affecting the build artifact (F3).
/// Returns `false` for paths excluded by `[binary_affecting]` (tests, docs,
/// `.md`, …) — those changes never trigger a bump. Segment exclusions match
/// whole `/`-delimited components, never substrings (so `src/examples_helper.rs`
/// is *not* excluded by an `examples` segment rule).
pub fn is_binary_affecting(path_bytes: &[u8], cfg: &BinaryAffectingConfiguration) -> bool {
    let path = String::from_utf8_lossy(path_bytes);
    let path: &str = &path;
    let basename = path.rsplit('/').next().unwrap_or(path);

    if cfg.exclude_names.iter().any(|n| n == basename) {
        return false;
    }
    if cfg
        .exclude_suffixes
        .iter()
        .any(|suf| path.ends_with(suf.as_str()))
    {
        return false;
    }
    for seg in path.split('/') {
        if cfg.exclude_segments.iter().any(|ex| ex == seg) {
            return false;
        }
    }
    true
}

#[derive(Debug)]
pub struct PathMatcher {
    terms: Vec<PathMatcherTerm>,
    /// F4-Glob (Tier-3) — residual glob patterns for manifest-less units whose
    /// `paths` aren't simple prefixes (e.g. `**/*.sql`). Additive: a path
    /// matches if any prefix `Include` term matches OR any glob matches. Empty
    /// for all normal (prefix-based) units, so they are entirely unaffected and
    /// `make_disjoint` (prefix-only) keeps working unchanged.
    globs: Vec<globset::GlobMatcher>,
}

impl PathMatcher {
    /// Create a new matcher that includes only files in the specified repopath
    /// prefix.
    pub fn new_include(p: RepoPathBuf) -> Self {
        PathMatcher {
            terms: vec![PathMatcherTerm::Include(p)],
            globs: Vec::new(),
        }
    }

    /// Create a matcher with no prefix `Include` terms — only globs (Tier-3).
    /// Used by glob-only `paths` units, where a prefix `Include` would
    /// over-match (e.g. an empty prefix matches everything).
    pub fn new_globs_only() -> Self {
        PathMatcher {
            terms: Vec::new(),
            globs: Vec::new(),
        }
    }

    /// Add another included prefix to this matcher. Used by multi-path units
    /// (e.g. a `paths`-only internal unit covering several directories).
    pub fn add_include(&mut self, p: RepoPathBuf) {
        self.terms.push(PathMatcherTerm::Include(p));
    }

    /// Compile + add a residual glob pattern (Tier-3). Returns an error if the
    /// pattern is not a valid glob.
    pub fn add_glob(&mut self, pattern: &str) -> anyhow::Result<()> {
        let glob = globset::Glob::new(pattern)
            .with_context(|| format!("invalid glob pattern `{pattern}`"))?;
        self.globs.push(glob.compile_matcher());
        Ok(())
    }

    /// Whether this matcher carries any residual glob patterns (Tier-3). Such
    /// units are exempt from `make_disjoint` and are subject to the overlap
    /// guard in `analyze_histories`.
    pub fn has_globs(&self) -> bool {
        !self.globs.is_empty()
    }

    /// Modify this matcher to exclude any paths that *other* would include.
    ///
    /// This whole framework could surely be a lot more efficient, but unless
    /// your repo has 1000 projects it's just not going to matter, I think.
    pub fn make_disjoint(&mut self, other: &PathMatcher) -> &mut Self {
        let mut new_terms = Vec::new();

        for other_term in &other.terms {
            if let PathMatcherTerm::Include(ref other_pfx) = other_term {
                for term in &self.terms {
                    if let PathMatcherTerm::Include(ref pfx) = term {
                        // We only need to exclude terms in the other matcher
                        // that are more specific than ours.
                        if other_pfx.starts_with(pfx) {
                            new_terms.push(PathMatcherTerm::Exclude(other_pfx.clone()));
                        }
                    }
                }
            }
        }

        new_terms.append(&mut self.terms);
        self.terms = new_terms;
        self
    }

    /// Test whether a repo-path matches.
    pub fn repo_path_matches(&self, p: &RepoPath) -> bool {
        for term in &self.terms {
            match term {
                PathMatcherTerm::Include(pfx) => {
                    if p.starts_with(pfx) {
                        return true;
                    }
                }

                PathMatcherTerm::Exclude(pfx) => {
                    if p.starts_with(pfx) {
                        return false;
                    }
                }
            }
        }

        // Tier-3 — residual globs are additive (checked after prefix terms).
        if !self.globs.is_empty() {
            let candidate = p.as_path();
            if self.globs.iter().any(|g| g.is_match(candidate)) {
                return true;
            }
        }

        false
    }
}

#[derive(Debug)]
enum PathMatcherTerm {
    /// Include paths prefixed by the value.
    Include(RepoPathBuf),

    /// Exclude paths prefixed by the value.
    Exclude(RepoPathBuf),
}

#[cfg(test)]
#[path = "path_matcher_tests.rs"]
mod path_matcher_tests;
