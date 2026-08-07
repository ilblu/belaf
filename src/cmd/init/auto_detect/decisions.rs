//! Decisions loaded from an existing `belaf/config.toml`
//! (`[ignore_paths]`, `[allow_uncovered]`, `[release_unit]` coverage),
//! threaded into auto-detect so re-runs never override a human
//! decision with a machine guess.

use crate::core::config::ConfigurationFile;
use crate::core::ecosystem::format_handler::{FormatHandlerRegistry, WorkspaceDiscovererRegistry};
use crate::core::git::repository::Repository;
use crate::core::release_unit::detector;
use crate::core::release_unit::discovery::discover_implicit_release_units;
use crate::core::release_unit::resolver;

/// Decisions already recorded in an existing `belaf/config.toml`,
/// threaded into auto-detect so a re-run never overrides them with a
/// machine guess:
///
/// - `[ignore_paths]` suppresses **detection** entirely — per its own
///   contract ("belaf does not scan inside at all") hits under these
///   paths are dropped before any emission or counting.
/// - `[allow_uncovered]` suppresses **emission** only — the paths are
///   still scanned (their contract is "scanned but accepted"), but a
///   human already classified them, so no blocks are emitted and the
///   hits surface as [`super::DetectionCounters::already_covered`]
///   instead.
///
/// Coverage semantics are shared with the drift detector
/// ([`detector::is_covered_by_config_paths`]): whatever `prepare`
/// treats as covered, auto-detect refuses to re-emit.
#[derive(Clone, Debug, Default)]
pub struct ExistingDecisions {
    pub ignore_paths: Vec<String>,
    pub allow_uncovered: Vec<String>,
    /// Directory prefixes claimed by already-configured
    /// `[release_unit.<name>]` blocks (resolved explicit + glob-form
    /// units — see [`detector::unit_coverage_paths`]). Emission-level
    /// suppression like `allow_uncovered`: re-emitting a block for a
    /// configured unit would append a duplicate table.
    pub unit_paths: Vec<String>,
}

impl ExistingDecisions {
    pub fn from_config(cfg: &ConfigurationFile) -> Self {
        Self {
            ignore_paths: cfg.ignore_paths.paths.clone(),
            allow_uncovered: cfg.allow_uncovered.paths.clone(),
            unit_paths: Vec::new(),
        }
    }

    /// Like [`Self::from_config`], but additionally claims the paths of
    /// already-configured `[release_unit.<name>]` blocks so a re-run
    /// (`init --ci --auto-detect --force`, or the wizard on an existing
    /// config) emits only genuinely new detection results. Glob-form
    /// units are expanded against the working tree, so a new directory
    /// matching an existing glob counts as covered — the config already
    /// handles it.
    ///
    /// Mirrors `AppBuilder::initialize` including partial-override
    /// resolution against the discovered unit set, so this coverage is
    /// exactly the drift check's coverage: whatever `prepare` is silent
    /// about, a re-detect will not re-emit.
    pub fn from_config_with_units(
        cfg: &ConfigurationFile,
        repo: &Repository,
    ) -> anyhow::Result<Self> {
        let mut decisions = Self::from_config(cfg);
        let output = resolver::resolve(repo, &cfg.release_units)
            .map_err(|e| anyhow::anyhow!("release_unit resolution: {e}"))?;
        let mut resolved = output.resolved;

        // Partial-override blocks (no `ecosystem`) reference discovered
        // units; without resolving them a re-detect could emit a second
        // block for a unit that already exists as an override. The
        // discovery walk only runs when such blocks are present.
        if !output.partial_overrides.is_empty() {
            let ownership = crate::core::release_unit::ownership::ownership_for(
                &resolved,
                &cfg.ignore_paths.paths,
            );
            let handlers = FormatHandlerRegistry::with_defaults();
            let discoverers = WorkspaceDiscovererRegistry::with_defaults();
            let discovered =
                discover_implicit_release_units(repo, &handlers, &discoverers, &ownership)
                    .map_err(|e| anyhow::anyhow!("implicit unit discovery: {e}"))?;
            let partial = resolver::resolve_partial_against_discovered(
                &output.partial_overrides,
                &discovered.units,
            )
            .map_err(|e| anyhow::anyhow!("partial-override resolution: {e}"))?;
            resolved.extend(partial);
        }

        decisions.unit_paths = detector::unit_coverage_paths(&resolved)
            .iter()
            .map(|p| p.escaped().to_string())
            .collect();
        Ok(decisions)
    }
}
