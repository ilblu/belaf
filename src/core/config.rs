use std::path::Path;

use crate::atry;
use crate::core::errors::{Error, Result};

pub mod syntax {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    use crate::core::release_unit::syntax::{
        AllowUncoveredConfig, EcosystemsConfig, IgnorePathsConfig, ReleaseUnitConfig,
    };

    /// Wire-form for the full `belaf/config.toml`. See the README + docs/configuration.md
    /// for the user-facing documentation; this is the literal serde shape.
    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ReleaseConfiguration {
        pub repo: RepoConfiguration,

        pub changelog: ChangelogConfiguration,

        pub bump: BumpConfiguration,

        pub commit_attribution: CommitAttributionConfiguration,

        /// `[binary_affecting]` (F3) — path-exclusion lists controlling which
        /// changes count toward a bump. Defaults from the embedded config.
        #[serde(default)]
        pub binary_affecting: BinaryAffectingConfiguration,

        /// `[cascade_inputs.<name>]` — declared path inputs that feed release
        /// units without being units themselves (a shared OCI base image,
        /// protobuf schemas, a shared config tree). A change under one of the
        /// declared paths cascades into every unit the input `affects`.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        pub cascade_inputs: HashMap<String, CascadeInputConfig>,

        /// Removed in 4.0.0 — superseded by `[cascade_inputs]`. Captured only
        /// so the config load can fail with a migration message instead of a
        /// raw serde `unknown field` blob (this struct is
        /// `deny_unknown_fields`). Never read for behaviour.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub codegen_edges: Option<HashMap<String, Vec<String>>>,

        /// `[group.<id>]` — bundles projects that release together with
        /// synchronised versions. Named-entry form only; the parser
        /// rejects an array-of-tables `[[group]]` shape.
        ///
        /// ```toml
        /// [group.schema]
        /// members = ["@org/schema", "com.org:schema"]
        /// tag_format = "schema-v{version}"
        /// ```
        #[serde(default, rename = "group", skip_serializing_if = "HashMap::is_empty")]
        pub groups: HashMap<String, GroupConfig>,

        #[serde(default, rename = "bump_source", skip_serializing_if = "Vec::is_empty")]
        pub bump_sources: Vec<BumpSourceConfig>,

        /// `[release_unit.<name>]` — named-entry release units. Each
        /// entry is either explicit (no `glob` field) or glob-form
        /// (with `glob` set, expanding at resolve-time into N units
        /// per matching directory). Named-entry only; the parser
        /// rejects array-of-tables `[[release_unit]]` and the separate
        /// `[[release_unit_glob]]` top-level key.
        #[serde(
            default,
            rename = "release_unit",
            skip_serializing_if = "HashMap::is_empty"
        )]
        pub release_units: HashMap<String, ReleaseUnitConfig>,

        /// `[ignore_paths]` — paths belaf does not scan inside.
        #[serde(default, skip_serializing_if = "IgnorePathsConfig::is_empty")]
        pub ignore_paths: IgnorePathsConfig,

        /// `[allow_uncovered]` — paths belaf scans but explicitly
        /// accepts as not mapping to any ReleaseUnit. Mobile apps go
        /// here on init.
        #[serde(default, skip_serializing_if = "AllowUncoveredConfig::is_empty")]
        pub allow_uncovered: AllowUncoveredConfig,

        /// `[ecosystems.*]` — per-ecosystem smart-default knobs.
        #[serde(default, skip_serializing_if = "EcosystemsConfig::is_empty")]
        pub ecosystems: EcosystemsConfig,
    }

    /// `[cascade_inputs.<name>]` named-entry — the TOML key is the input's
    /// name. It becomes the synthetic graph node's name, so it is also a
    /// valid commit scope for `belaf check` and shows up in changelogs as
    /// `via <name>`.
    ///
    /// ```toml
    /// [cascade_inputs.apko-base]
    /// paths   = ["apko/base.yaml", "apko/*.lock", "apko/overlays/**"]
    /// affects = "all-deploy-units"   # or ["gate", "rig", "kin"]
    /// bump    = "floor_minor"        # optional
    /// ```
    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct CascadeInputConfig {
        /// Repo-relative paths this input owns. Entries containing `*`, `?`
        /// or `[` are treated as globs (`apko/*.lock`); everything else is a
        /// literal path prefix (`apko/overlays`, `apko/base.yaml`).
        pub paths: Vec<String>,

        /// Which release units a change under `paths` cascades into.
        pub affects: CascadeInputAffects,

        /// Bump floor applied to the affected units, using the same
        /// vocabulary as `cascade_from`: `mirror` (default) | `floor_patch` |
        /// `floor_minor` | `floor_major`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub bump: Option<String>,
    }

    /// The `affects` field of a `[cascade_inputs.<name>]` block. Untagged so
    /// users write either a shorthand string or a plain list — precedent:
    /// `ManifestList` in `release_unit/syntax.rs`.
    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    #[serde(untagged)]
    pub enum CascadeInputAffects {
        /// A shorthand keyword. The only accepted value is
        /// [`ALL_DEPLOY_UNITS`](super::ALL_DEPLOY_UNITS); anything else is a
        /// hard config error (a typo must never silently affect nothing).
        Shorthand(String),
        /// An explicit list of release-unit names.
        Units(Vec<String>),
    }

    /// `[group.<id>]` named-entry — the TOML key is the group id.
    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct GroupConfig {
        pub members: Vec<String>,

        /// Group-level tag-format override. Wins over the ecosystem
        /// default but loses to per-project overrides. Useful when every
        /// member of a synchronised group should ship under one tag
        /// (e.g. `schema-v{version}` for a multi-ecosystem schema bundle).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub tag_format: Option<String>,
    }

    /// Runtime-adjacent shape: same fields as [`GroupConfig`] plus the
    /// id (lifted out of the TOML key). The rest of the codebase works
    /// in terms of this; the TOML form only exists at deserialize.
    #[derive(Clone, Debug)]
    pub struct ResolvedGroupConfig {
        pub id: String,
        pub members: Vec<String>,
        pub tag_format: Option<String>,
    }

    /// `[[bump_source]]` table: a subprocess belaf runs by default to
    /// gather externally-computed bump decisions (e.g. `graphql-inspector
    /// diff`). `cmd` is required; `release_unit` / `group` are pure
    /// diagnostic labels (the JSON output's own `release_unit` field is
    /// what wires decisions to ReleaseUnits).
    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    pub struct BumpSourceConfig {
        pub cmd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub release_unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub group: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub timeout_sec: Option<u64>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    pub struct BumpConfiguration {
        pub features_always_bump_minor: bool,

        pub breaking_always_bump_major: bool,

        pub initial_tag: String,

        #[serde(default)]
        pub bump_type: Option<String>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    pub struct ChangelogConfiguration {
        #[serde(default)]
        pub header: Option<String>,

        pub body: String,

        #[serde(default)]
        pub footer: Option<String>,

        pub trim: bool,

        pub output: String,

        pub conventional_commits: bool,

        pub protect_breaking_commits: bool,

        pub filter_unconventional: bool,

        pub filter_commits: bool,

        pub sort_commits: String,

        #[serde(default)]
        pub limit_commits: Option<usize>,

        #[serde(default)]
        pub tag_pattern: Option<String>,

        #[serde(default)]
        pub skip_tags: Option<String>,

        #[serde(default)]
        pub ignore_tags: Option<String>,

        #[serde(default)]
        pub commit_parsers: Vec<CommitParserConfig>,

        #[serde(default)]
        pub link_parsers: Vec<LinkParserConfig>,

        #[serde(default)]
        pub commit_preprocessors: Vec<TextProcessorConfig>,

        #[serde(default)]
        pub postprocessors: Vec<TextProcessorConfig>,

        pub include_breaking_section: bool,

        pub include_contributors: bool,

        pub include_statistics: bool,

        pub emoji_groups: bool,

        #[serde(default)]
        pub group_emojis: std::collections::HashMap<String, String>,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
    pub struct CommitParserConfig {
        #[serde(default)]
        pub message: Option<String>,

        #[serde(default)]
        pub body: Option<String>,

        #[serde(default)]
        pub footer: Option<String>,

        #[serde(default)]
        pub group: Option<String>,

        #[serde(default)]
        pub scope: Option<String>,

        #[serde(default)]
        pub default_scope: Option<String>,

        #[serde(default)]
        pub skip: Option<bool>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    pub struct LinkParserConfig {
        pub pattern: String,

        pub href: String,

        #[serde(default)]
        pub text: Option<String>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    pub struct TextProcessorConfig {
        pub pattern: String,

        #[serde(default)]
        pub replace: Option<String>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    pub struct CommitAttributionConfiguration {
        pub strategy: String,

        pub scope_matching: String,

        #[serde(default)]
        pub scope_mappings: HashMap<String, String>,

        #[serde(default)]
        pub package_scopes: HashMap<String, Vec<String>>,
    }

    /// `[binary_affecting]` (F3) — which changed paths count as affecting the
    /// build artifact. A commit that only touches *non*-binary-affecting files
    /// (tests, docs, …) does not trigger a bump. Defaults live in the embedded
    /// `default.toml`; a user's `belaf/config.toml` overrides each list.
    #[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
    pub struct BinaryAffectingConfiguration {
        /// Full path segments that are NOT binary-affecting (matched as whole
        /// `/segment/` components, never substrings) — e.g. `tests`, `benches`.
        #[serde(default)]
        pub exclude_segments: Vec<String>,

        /// File-name suffixes that are NOT binary-affecting — e.g. `.md`.
        #[serde(default)]
        pub exclude_suffixes: Vec<String>,

        /// Exact repo-relative file names that are NOT binary-affecting — e.g.
        /// `CHANGELOG.md`, `LICENSE`.
        #[serde(default)]
        pub exclude_names: Vec<String>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    pub struct RepoConfiguration {
        #[serde(default)]
        pub upstream_urls: Vec<String>,

        /// Template for the branch `prepare` pushes the release commit to.
        /// `{base}` expands to the branch the run started from. Defaults to
        /// [`DEFAULT_RELEASE_BRANCH_TEMPLATE`](crate::core::git::refs::DEFAULT_RELEASE_BRANCH_TEMPLATE)
        /// when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub release_branch: Option<String>,

        pub analysis: AnalysisConfig,
    }

    #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
    pub struct AnalysisConfig {
        pub commit_cache_size: usize,

        pub tree_cache_size: usize,
    }
}

/// Runtime-adjacent shape: a single named release unit with the name
/// lifted out of the HashMap key. Resolver consumes this.
#[derive(Clone, Debug)]
pub struct NamedReleaseUnitConfig {
    pub name: String,
    pub config: crate::core::release_unit::syntax::ReleaseUnitConfig,
}

/// The only accepted `affects` shorthand. Spelled out here so the parser,
/// the error message and the docs can never drift apart.
pub const ALL_DEPLOY_UNITS: &str = "all-deploy-units";

/// Which units a `[cascade_inputs.<name>]` block feeds — the validated form
/// of [`syntax::CascadeInputAffects`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CascadeInputTargets {
    /// Every `kind = "deploy"` unit in the repo. Internal/ignore units are
    /// excluded (they never release, so a floor on them is meaningless).
    AllDeployUnits,
    /// An explicit, non-empty list of unit names.
    Units(Vec<String>),
}

/// Runtime-adjacent shape of one `[cascade_inputs.<name>]` block: name lifted
/// out of the HashMap key, `affects` and `bump` validated into typed values.
/// Produced by [`resolve_cascade_inputs`] at config-load time so a bad entry
/// fails there rather than silently matching nothing at analysis time.
#[derive(Clone, Debug)]
pub struct ResolvedCascadeInput {
    pub name: String,
    pub paths: Vec<String>,
    pub affects: CascadeInputTargets,
    /// `None` = the user did not declare one; the floor then defaults to
    /// [`CascadeBumpStrategy::Mirror`](crate::core::release_unit::CascadeBumpStrategy::Mirror).
    /// Kept distinct from `Some(Mirror)` so the manifest only reports a
    /// `bump` the user actually wrote.
    pub bump: Option<crate::core::release_unit::CascadeBumpStrategy>,
}

/// Parse `bump = "..."` into a [`CascadeBumpStrategy`], mirroring the
/// `cascade_from` vocabulary.
fn parse_cascade_bump(
    input_name: &str,
    raw: &str,
) -> Result<crate::core::release_unit::CascadeBumpStrategy> {
    use crate::core::release_unit::CascadeBumpStrategy;
    match raw {
        "mirror" => Ok(CascadeBumpStrategy::Mirror),
        "floor_patch" => Ok(CascadeBumpStrategy::FloorPatch),
        "floor_minor" => Ok(CascadeBumpStrategy::FloorMinor),
        "floor_major" => Ok(CascadeBumpStrategy::FloorMajor),
        other => Err(Error::msg(format!(
            "[cascade_inputs.{input_name}] has bump = \"{other}\", which is not a known \
             strategy. Valid values: \"mirror\" (default), \"floor_patch\", \"floor_minor\", \
             \"floor_major\"."
        ))),
    }
}

/// Validate + promote the `[cascade_inputs]` table into the runtime shape,
/// sorted by name for deterministic node creation and error output.
///
/// Every failure mode here is a hard error on purpose: a `[cascade_inputs]`
/// entry that matches nothing is exactly the silent no-op this feature exists
/// to eliminate.
pub fn resolve_cascade_inputs(
    table: std::collections::HashMap<String, syntax::CascadeInputConfig>,
) -> Result<Vec<ResolvedCascadeInput>> {
    let mut out: Vec<ResolvedCascadeInput> = Vec::with_capacity(table.len());

    for (name, cfg) in table {
        if name.trim().is_empty() {
            return Err(Error::msg(
                "[cascade_inputs] has an entry with an empty name; the table key becomes the \
                 input's unit name and must be non-empty",
            ));
        }

        if cfg.paths.is_empty() {
            return Err(Error::msg(format!(
                "[cascade_inputs.{name}] has `paths = []`. An input with no paths can never \
                 match a change — declare at least one path, or remove the block."
            )));
        }
        if let Some(bad) = cfg.paths.iter().find(|p| p.trim().is_empty()) {
            let _ = bad;
            return Err(Error::msg(format!(
                "[cascade_inputs.{name}] has an empty string in `paths`. Every entry must be a \
                 repo-relative path or glob."
            )));
        }

        let affects = match cfg.affects {
            syntax::CascadeInputAffects::Shorthand(s) if s == ALL_DEPLOY_UNITS => {
                CascadeInputTargets::AllDeployUnits
            }
            syntax::CascadeInputAffects::Shorthand(s) => {
                return Err(Error::msg(format!(
                    "[cascade_inputs.{name}] has affects = \"{s}\", which is not a known \
                     shorthand. The only accepted string is \"{ALL_DEPLOY_UNITS}\"; to target \
                     specific units, write a list: affects = [\"unit-a\", \"unit-b\"]."
                )));
            }
            syntax::CascadeInputAffects::Units(units) => {
                if units.is_empty() {
                    return Err(Error::msg(format!(
                        "[cascade_inputs.{name}] has `affects = []`. An input that affects \
                         nothing is a no-op — list at least one release unit, or use \
                         affects = \"{ALL_DEPLOY_UNITS}\"."
                    )));
                }
                if let Some(bad) = units.iter().find(|u| u.trim().is_empty()) {
                    let _ = bad;
                    return Err(Error::msg(format!(
                        "[cascade_inputs.{name}] has an empty string in `affects`."
                    )));
                }
                CascadeInputTargets::Units(units)
            }
        };

        let bump = match &cfg.bump {
            Some(raw) => Some(parse_cascade_bump(&name, raw)?),
            None => None,
        };

        out.push(ResolvedCascadeInput {
            name,
            paths: cfg.paths,
            affects,
            bump,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Turn a legacy `[codegen_edges]` glob into the `[cascade_inputs]` key it
/// used to produce as a node name (`"proto/**"` → `proto`). Keeping the old
/// derivation means a migrated config keeps the same unit name, so existing
/// commit scopes and changelog `via` prefixes stay valid.
fn codegen_edge_key(glob: &str) -> String {
    let mut prefix = glob.trim_end_matches('*').to_string();
    if !prefix.ends_with('/') {
        prefix.push('/');
    }
    prefix
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}

/// Hard migration for the removed `[codegen_edges]` section: abort the config
/// load with a message that renders the equivalent `[cascade_inputs]` block.
/// Deliberately not a silent rewrite — the semantics changed (file globs now
/// work, the node name is the table key, and a bump floor is available), so
/// the user must look at the result once.
fn reject_codegen_edges(
    edges: &Option<std::collections::HashMap<String, Vec<String>>>,
) -> Result<()> {
    let Some(edges) = edges else {
        return Ok(());
    };

    let mut rendered = String::new();
    let mut globs: Vec<&String> = edges.keys().collect();
    globs.sort();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for glob in globs {
        let crates = &edges[glob];
        let base = {
            let k = codegen_edge_key(glob);
            if k.is_empty() {
                "input".to_string()
            } else {
                k
            }
        };
        // Two globs can derive the same key ("a/proto/**" and "b/proto/**").
        // Suffix duplicates so the rendered block is valid TOML.
        let count = seen.entry(base.clone()).or_insert(0);
        *count += 1;
        let key = if *count == 1 {
            base
        } else {
            format!("{base}-{count}")
        };

        let crate_list = crates
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        rendered.push_str(&format!(
            "\n[cascade_inputs.{key}]\npaths   = [\"{glob}\"]\naffects = [{crate_list}]\n"
        ));
    }

    if rendered.is_empty() {
        rendered.push_str(
            "\n[cascade_inputs.my-input]\npaths   = [\"proto/**\"]\naffects = [\"my-crate\"]\n",
        );
    }

    Err(Error::msg(format!(
        "`[codegen_edges]` was removed in belaf 4.0.0 and replaced by `[cascade_inputs]`, which \
         also matches file globs (`apko/*.lock`), supports a per-input bump floor, and records \
         provenance in the release manifest.\n\n\
         Delete the `[codegen_edges]` section from belaf/config.toml and add:\n{rendered}\n\
         Notes:\n\
         \x20 - The table key is now the input's unit name (it used to be derived from the \
         glob's last path segment). The keys above keep the old names, so existing commit \
         scopes such as `fix(<name>):` and changelog `via <name>` prefixes stay valid.\n\
         \x20 - `affects = \"{ALL_DEPLOY_UNITS}\"` targets every deploy unit at once.\n\
         \x20 - Optional: `bump = \"floor_minor\"` to raise the bump the input forces on the \
         units it affects."
    )))
}

#[derive(Clone, Debug)]
pub struct ConfigurationFile {
    pub repo: syntax::RepoConfiguration,
    pub changelog: syntax::ChangelogConfiguration,
    pub bump: syntax::BumpConfiguration,
    pub commit_attribution: syntax::CommitAttributionConfiguration,
    pub binary_affecting: syntax::BinaryAffectingConfiguration,
    /// `[cascade_inputs.<name>]`, validated and sorted by name.
    pub cascade_inputs: Vec<ResolvedCascadeInput>,
    pub groups: Vec<syntax::ResolvedGroupConfig>,
    pub bump_sources: Vec<syntax::BumpSourceConfig>,
    pub release_units: Vec<NamedReleaseUnitConfig>,
    pub ignore_paths: crate::core::release_unit::syntax::IgnorePathsConfig,
    pub allow_uncovered: crate::core::release_unit::syntax::AllowUncoveredConfig,
    pub ecosystems: crate::core::release_unit::syntax::EcosystemsConfig,
}

impl ConfigurationFile {
    pub fn get<P: AsRef<Path>>(path: P) -> Result<Self> {
        let embedded_config_str = super::embed::EmbeddedConfig::get_config_string()?;

        let mut builder = config::Config::builder().add_source(config::File::from_str(
            &embedded_config_str,
            config::FileFormat::Toml,
        ));

        if path.as_ref().exists() {
            builder = builder.add_source(config::File::from(path.as_ref()));
        }

        let cfg: syntax::ReleaseConfiguration = builder
            .build()
            .map_err(|e| Error::new(e).context("failed to build configuration"))?
            .try_deserialize()
            .map_err(|e| Error::new(e).context("failed to deserialize configuration"))?;

        // `[codegen_edges]` is gone — fail here with a rendered replacement
        // block rather than letting a stale config silently do nothing.
        reject_codegen_edges(&cfg.codegen_edges)?;

        let cascade_inputs = resolve_cascade_inputs(cfg.cascade_inputs)?;

        // Promote the HashMap keys into runtime-adjacent shapes with a
        // stable iteration order. Sort by name for deterministic
        // resolution + downstream tests.
        let mut groups: Vec<syntax::ResolvedGroupConfig> = cfg
            .groups
            .into_iter()
            .map(|(id, g)| syntax::ResolvedGroupConfig {
                id,
                members: g.members,
                tag_format: g.tag_format,
            })
            .collect();
        groups.sort_by(|a, b| a.id.cmp(&b.id));

        let mut release_units: Vec<NamedReleaseUnitConfig> = cfg
            .release_units
            .into_iter()
            .map(|(name, config)| NamedReleaseUnitConfig { name, config })
            .collect();
        release_units.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(ConfigurationFile {
            repo: cfg.repo,
            changelog: cfg.changelog,
            bump: cfg.bump,
            commit_attribution: cfg.commit_attribution,
            binary_affecting: cfg.binary_affecting,
            cascade_inputs,
            groups,
            bump_sources: cfg.bump_sources,
            release_units,
            ignore_paths: cfg.ignore_paths,
            allow_uncovered: cfg.allow_uncovered,
            ecosystems: cfg.ecosystems,
        })
    }

    pub fn into_toml(self) -> Result<String> {
        use std::collections::HashMap;
        let groups: HashMap<String, syntax::GroupConfig> = self
            .groups
            .into_iter()
            .map(|g| {
                (
                    g.id,
                    syntax::GroupConfig {
                        members: g.members,
                        tag_format: g.tag_format,
                    },
                )
            })
            .collect();
        let release_units: HashMap<String, crate::core::release_unit::syntax::ReleaseUnitConfig> =
            self.release_units
                .into_iter()
                .map(|u| (u.name, u.config))
                .collect();
        let cascade_inputs: HashMap<String, syntax::CascadeInputConfig> = self
            .cascade_inputs
            .into_iter()
            .map(|c| {
                (
                    c.name,
                    syntax::CascadeInputConfig {
                        paths: c.paths,
                        affects: match c.affects {
                            CascadeInputTargets::AllDeployUnits => {
                                syntax::CascadeInputAffects::Shorthand(ALL_DEPLOY_UNITS.to_string())
                            }
                            CascadeInputTargets::Units(u) => syntax::CascadeInputAffects::Units(u),
                        },
                        bump: c.bump.map(|b| b.wire_key().to_string()),
                    },
                )
            })
            .collect();
        let cfg = syntax::ReleaseConfiguration {
            repo: self.repo,
            changelog: self.changelog,
            bump: self.bump,
            commit_attribution: self.commit_attribution,
            binary_affecting: self.binary_affecting,
            cascade_inputs,
            codegen_edges: None,
            groups,
            bump_sources: self.bump_sources,
            release_units,
            ignore_paths: self.ignore_paths,
            allow_uncovered: self.allow_uncovered,
            ecosystems: self.ecosystems,
        };
        Ok(atry!(
            toml::to_string_pretty(&cfg);
            ["could not serialize configuration into TOML format"]
        ))
    }
}
