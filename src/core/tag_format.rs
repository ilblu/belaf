//! Tag-name templating + validation.
//!
//! Every release in the manifest carries a `tag_name` field — the git tag
//! the github-app will create on PR-merge. v2.0 makes that name
//! ecosystem-aware:
//!
//! - npm:   `{name}@v{version}`
//! - cargo: `{name}-v{version}`
//! - maven: `{groupId}/{artifactId}@v{version}` (slash, not colon — git
//!   ref-format rejects `:` so the v1 default would have produced
//!   un-pushable tags for any Maven project)
//! - pypa:  `{name}-{version}`
//! - go:    `{module}/v{version}`
//! - csproj/swift/elixir: `{name}@v{version}`
//!
//! Users override per-project via `[project."name".tag_format]` and per-
//! group via `[group.<id>].tag_format`. Precedence: project > group >
//! ecosystem default.
//!
//! Two validations layered on top:
//!
//! 1. **Template-variable whitelist per ecosystem**. The trait surface
//!    `Ecosystem::tag_template_vars()` is the source of truth. A
//!    `{groupId}` reference in an npm tag-format is a hard error at
//!    template-resolve time.
//! 2. **`git check-ref-format --allow-onelevel`** on the resolved string.
//!    Catches anything that survived template expansion but git would
//!    reject — control characters, `..`, `.lock` suffix, lone `@`, etc.

use std::collections::HashMap;
use std::process::Command;

use anyhow::anyhow;
use regex::Regex;

use crate::core::errors::Result;

/// Inputs for tag-name resolution. `ecosystem_default` is the trait-
/// supplied template; `override_template` (if any) wins over it.
#[derive(Debug, Clone)]
pub struct TagFormatInputs<'a> {
    pub project_name: &'a str,
    pub version: &'a str,
    pub ecosystem: &'a str,
    pub ecosystem_default: &'a str,
    pub allowed_vars: &'a [&'static str],
    pub override_template: Option<&'a str>,
    /// Maven-only — `(groupId, artifactId)` pair, derived from a project
    /// name like `com.example:lib`.
    pub maven_coords: Option<(String, String)>,
    /// Go-only — module path. Defaults to `project_name` if absent.
    pub module_path: Option<&'a str>,
}

/// Resolve a tag template against the project's ecosystem variables.
/// Returns the formatted tag string, or a descriptive error if the
/// template uses an unsupported variable for this ecosystem.
pub fn format_tag(inputs: &TagFormatInputs<'_>) -> Result<String> {
    let template = inputs.override_template.unwrap_or(inputs.ecosystem_default);

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("name", inputs.project_name.to_string());
    vars.insert("version", inputs.version.to_string());
    vars.insert("ecosystem", inputs.ecosystem.to_string());
    if let Some((g, a)) = &inputs.maven_coords {
        vars.insert("groupId", g.clone());
        vars.insert("artifactId", a.clone());
    }
    if let Some(m) = inputs.module_path {
        vars.insert("module", m.to_string());
    }

    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find('}') else {
            return Err(anyhow!(
                "tag-format template `{template}` has an unclosed `{{` — every `{{` must be paired with `}}`"
            ));
        };
        let var_name = &rest[1..end];
        if !inputs.allowed_vars.contains(&var_name) {
            return Err(anyhow!(
                "tag-format template `{template}` uses variable `{{{var_name}}}` which is not valid for the `{}` ecosystem. \
                 Allowed variables here: {{{}}}.",
                inputs.ecosystem,
                inputs.allowed_vars.join("}, {")
            ));
        }
        let value = vars.get(var_name).ok_or_else(|| {
            anyhow!(
                "tag-format template `{template}`: variable `{{{var_name}}}` is allowed for the `{}` ecosystem but no value was supplied (this is a belaf bug — please report)",
                inputs.ecosystem
            )
        })?;
        out.push_str(value);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);

    validate_git_ref_format(&out)?;
    Ok(out)
}

/// Run the resolved tag string through `git check-ref-format
/// --allow-onelevel`. We shell out rather than reimplementing git's rules
/// because the rules are subtle (control chars, `..`, `.lock`, lone `@`,
/// and so on); using git itself guarantees the tag will be acceptable
/// when the github-app tries to create it.
pub fn validate_git_ref_format(tag: &str) -> Result<()> {
    let out = Command::new("git")
        .args(["check-ref-format", "--allow-onelevel", tag])
        .output()
        .map_err(|e| {
            anyhow!("failed to invoke `git check-ref-format` to validate tag `{tag}`: {e}")
        })?;
    if out.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "tag `{tag}` is not a valid git reference name — `git check-ref-format` rejected it. \
         Common causes: contains `:` (Maven coordinates need `/` instead), contains `..`, ends with `.lock`, \
         contains control characters, or is a lone `@`."
    ))
}

// ---------------------------------------------------------------------------
// Tag lookup — the inverse direction. Given the *same* template that
// `format_tag` would *write*, build a regex that recognises tags that
// have been written before and extracts the version. This is what
// `find_latest_tag_for_project` uses to walk existing tags and pick the
// previous release.
//
// **Why this exists.** The legacy read-path in
// `Repository::find_latest_tag_for_project` hard-coded `{name}-v{version}`
// (cargo style) and a bare-`v{version}` single-project fallback. Any
// ecosystem whose default template wasn't cargo-style — npm's
// `{name}@v{version}`, maven's `{groupId}/{artifactId}@v{version}`,
// pypa's `{name}-{version}` (no `v`), go's `{module}/v{version}` —
// silently never matched. Lookup then fell back to "walk every commit
// since repo start", which produced wildly wrong bumps. See the bug
// note on `TagMatcher::match_version` for the failure mode.
// ---------------------------------------------------------------------------

/// Inputs for compiling a [`TagMatcher`]. Mirrors [`TagFormatInputs`]
/// but omits `version` (the regex *captures* the version rather than
/// templating it in).
#[derive(Debug, Clone)]
pub struct TagPatternInputs<'a> {
    pub project_name: &'a str,
    pub ecosystem: &'a str,
    pub ecosystem_default: &'a str,
    pub allowed_vars: &'a [&'static str],
    pub override_template: Option<&'a str>,
    pub maven_coords: Option<(String, String)>,
    pub module_path: Option<&'a str>,
    /// When `true`, the matcher also recognises bare `v{version}` tags
    /// regardless of the configured template. Set by callers in
    /// single-project repos to preserve the cargo convention.
    pub allow_bare_v_fallback: bool,
}

/// Compiled recognizer for one project's release tags.
///
/// The primary pattern is derived from the project's effective
/// `tag_format` (unit > group > ecosystem default). When
/// `allow_bare_v_fallback` was set on the inputs, a secondary
/// `v{version}` pattern is also tried — single-project cargo repos
/// commonly tag plain `v1.2.3`.
#[derive(Debug, Clone)]
pub struct TagMatcher {
    primary: Regex,
    bare_v_fallback: Option<Regex>,
    project_name: String,
    template: String,
}

impl TagMatcher {
    /// If `tag` is a release tag for this project, return the captured
    /// semver version. `None` for tags that don't match (other
    /// projects' tags, unrelated refs, malformed versions).
    pub fn match_version(&self, tag: &str) -> Option<semver::Version> {
        if let Some(v) = capture_version(&self.primary, tag) {
            return Some(v);
        }
        if let Some(re) = &self.bare_v_fallback {
            if let Some(v) = capture_version(re, tag) {
                return Some(v);
            }
        }
        None
    }

    pub fn project_name(&self) -> &str {
        &self.project_name
    }

    pub fn template(&self) -> &str {
        &self.template
    }
}

fn capture_version(re: &Regex, tag: &str) -> Option<semver::Version> {
    let caps = re.captures(tag)?;
    let m = caps.name("version")?;
    semver::Version::parse(m.as_str()).ok()
}

/// Semver capture group used inside generated tag regexes. Matches the
/// full SemVer 2.0 production: `MAJOR.MINOR.PATCH(-PRE)?(+BUILD)?`.
/// Kept conservative — we feed the captured slice into
/// [`semver::Version::parse`], so this only needs to be permissive
/// enough to not pre-filter valid versions.
const VERSION_CAPTURE: &str =
    r"(?P<version>\d+\.\d+\.\d+(?:-[0-9A-Za-z\-]+(?:\.[0-9A-Za-z\-]+)*)?(?:\+[0-9A-Za-z\-]+(?:\.[0-9A-Za-z\-]+)*)?)";

/// Compile a [`TagMatcher`] from the project's effective tag template.
/// Mirrors [`format_tag`]'s variable-substitution + whitelist rules but
/// emits a regex with `{version}` as a named capture group.
pub fn build_tag_matcher(inputs: &TagPatternInputs<'_>) -> Result<TagMatcher> {
    let template = inputs.override_template.unwrap_or(inputs.ecosystem_default);

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("name", inputs.project_name.to_string());
    vars.insert("ecosystem", inputs.ecosystem.to_string());
    if let Some((g, a)) = &inputs.maven_coords {
        vars.insert("groupId", g.clone());
        vars.insert("artifactId", a.clone());
    }
    if let Some(m) = inputs.module_path {
        vars.insert("module", m.to_string());
    }

    let mut pattern = String::with_capacity(template.len() + 32);
    pattern.push('^');
    let mut rest = template;
    let mut saw_version = false;
    while let Some(start) = rest.find('{') {
        pattern.push_str(&regex::escape(&rest[..start]));
        rest = &rest[start..];
        let Some(end) = rest.find('}') else {
            return Err(anyhow!(
                "tag-format template `{template}` has an unclosed `{{` — every `{{` must be paired with `}}`"
            ));
        };
        let var_name = &rest[1..end];
        if !inputs.allowed_vars.contains(&var_name) {
            return Err(anyhow!(
                "tag-format template `{template}` uses variable `{{{var_name}}}` which is not valid for the `{}` ecosystem. \
                 Allowed variables here: {{{}}}.",
                inputs.ecosystem,
                inputs.allowed_vars.join("}, {")
            ));
        }
        if var_name == "version" {
            if saw_version {
                return Err(anyhow!(
                    "tag-format template `{template}` references `{{version}}` more than once"
                ));
            }
            saw_version = true;
            pattern.push_str(VERSION_CAPTURE);
        } else {
            let value = vars.get(var_name).ok_or_else(|| {
                anyhow!(
                    "tag-format template `{template}`: variable `{{{var_name}}}` is allowed for the `{}` ecosystem but no value was supplied (this is a belaf bug — please report)",
                    inputs.ecosystem
                )
            })?;
            pattern.push_str(&regex::escape(value));
        }
        rest = &rest[end + 1..];
    }
    pattern.push_str(&regex::escape(rest));
    pattern.push('$');

    if !saw_version {
        return Err(anyhow!(
            "tag-format template `{template}` does not reference `{{version}}` — refusing to compile a lookup pattern that would match every tag for this project equally"
        ));
    }

    let primary = Regex::new(&pattern).map_err(|e| {
        anyhow!("failed to compile tag-format regex from `{template}`: {e}")
    })?;

    let bare_v_fallback = if inputs.allow_bare_v_fallback {
        // Bare `v{version}` — the cargo single-project convention.
        // Independent of the configured template so it survives override.
        let p = format!("^v{VERSION_CAPTURE}$");
        Some(Regex::new(&p).expect("BUG: bare-v fallback regex must compile"))
    } else {
        None
    };

    Ok(TagMatcher {
        primary,
        bare_v_fallback,
        project_name: inputs.project_name.to_string(),
        template: template.to_string(),
    })
}

/// Try to split a project name like `com.example:lib` into Maven
/// `(groupId, artifactId)`. Belaf registers Maven projects with a
/// user-facing name `groupId:artifactId`, so this is a 1:1 inversion.
pub fn split_maven_coords(name: &str) -> Option<(String, String)> {
    let (g, a) = name.split_once(':')?;
    if g.is_empty() || a.is_empty() {
        return None;
    }
    Some((g.to_string(), a.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn npm_inputs<'a>() -> TagFormatInputs<'a> {
        TagFormatInputs {
            project_name: "@org/foo",
            version: "1.2.3",
            ecosystem: "npm",
            ecosystem_default: "{name}@v{version}",
            allowed_vars: &["name", "version", "ecosystem"],
            override_template: None,
            maven_coords: None,
            module_path: None,
        }
    }

    #[test]
    fn npm_default_template() {
        let t = format_tag(&npm_inputs()).unwrap();
        assert_eq!(t, "@org/foo@v1.2.3");
    }

    #[test]
    fn maven_default_uses_slash_not_colon() {
        let inputs = TagFormatInputs {
            project_name: "com.example:lib",
            version: "2.0.0",
            ecosystem: "maven",
            ecosystem_default: "{groupId}/{artifactId}@v{version}",
            allowed_vars: &["name", "version", "ecosystem", "groupId", "artifactId"],
            override_template: None,
            maven_coords: Some(("com.example".into(), "lib".into())),
            module_path: None,
        };
        let t = format_tag(&inputs).unwrap();
        assert_eq!(t, "com.example/lib@v2.0.0");
    }

    #[test]
    fn override_wins_over_default() {
        let mut inputs = npm_inputs();
        inputs.override_template = Some("custom-{name}-{version}");
        let t = format_tag(&inputs).unwrap();
        assert_eq!(t, "custom-@org/foo-1.2.3");
    }

    #[test]
    fn unsupported_variable_for_ecosystem_is_hard_error() {
        let mut inputs = npm_inputs();
        inputs.override_template = Some("{groupId}-{name}@v{version}");
        let err = format_tag(&inputs).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("groupId"),
            "should name the offender; got: {msg}"
        );
        assert!(msg.contains("npm"), "should name the ecosystem; got: {msg}");
    }

    #[test]
    fn unclosed_brace_is_hard_error() {
        let mut inputs = npm_inputs();
        inputs.override_template = Some("{name");
        let err = format_tag(&inputs).unwrap_err();
        assert!(format!("{err:#}").contains("unclosed"));
    }

    #[test]
    fn validate_git_ref_format_rejects_colon_in_tag() {
        // Tag with a colon — Maven's `groupId:artifactId` shape — must be
        // rejected by `git check-ref-format`.
        let err = validate_git_ref_format("com.example:lib@v1.0.0").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not a valid git reference name"));
    }

    #[test]
    fn validate_git_ref_format_accepts_slash_form() {
        // The slash form (Maven's safe default) must pass.
        validate_git_ref_format("com.example/lib@v1.0.0")
            .expect("slash-form Maven tag should validate");
    }

    #[test]
    fn split_maven_coords_round_trip() {
        assert_eq!(
            split_maven_coords("com.example:lib"),
            Some(("com.example".into(), "lib".into()))
        );
        assert!(split_maven_coords("no-colon-here").is_none());
        assert!(split_maven_coords(":missing-group").is_none());
        assert!(split_maven_coords("missing-artifact:").is_none());
    }

    // ----- TagMatcher / build_tag_matcher -----------------------------

    fn npm_pattern_inputs<'a>() -> TagPatternInputs<'a> {
        TagPatternInputs {
            project_name: "@clikd/landing",
            ecosystem: "npm",
            ecosystem_default: "{name}@v{version}",
            allowed_vars: &["name", "version", "ecosystem"],
            override_template: None,
            maven_coords: None,
            module_path: None,
            allow_bare_v_fallback: false,
        }
    }

    #[test]
    fn matcher_npm_default_recovers_version() {
        // The exact bug: npm tag `@clikd/landing@v0.7.0` must match,
        // not silently miss like the legacy hardcoded `-v` lookup.
        let m = build_tag_matcher(&npm_pattern_inputs()).unwrap();
        assert_eq!(
            m.match_version("@clikd/landing@v0.7.0"),
            Some(semver::Version::new(0, 7, 0))
        );
        assert_eq!(
            m.match_version("@clikd/landing@v1.2.3-rc.1"),
            Some(semver::Version::parse("1.2.3-rc.1").unwrap())
        );
    }

    #[test]
    fn matcher_npm_rejects_other_projects_tags() {
        let m = build_tag_matcher(&npm_pattern_inputs()).unwrap();
        // Same shape, different project — must not collide.
        assert_eq!(m.match_version("@clikd/api@v0.7.0"), None);
        // Cargo-style tag for an unrelated cargo crate — must not match.
        assert_eq!(m.match_version("belaf-v1.0.0"), None);
        // Bare v-tag without the bare-v fallback must not match.
        assert_eq!(m.match_version("v1.0.0"), None);
    }

    #[test]
    fn matcher_maven_slash_form() {
        let inputs = TagPatternInputs {
            project_name: "com.example:lib",
            ecosystem: "maven",
            ecosystem_default: "{groupId}/{artifactId}@v{version}",
            allowed_vars: &["name", "version", "ecosystem", "groupId", "artifactId"],
            override_template: None,
            maven_coords: Some(("com.example".into(), "lib".into())),
            module_path: None,
            allow_bare_v_fallback: false,
        };
        let m = build_tag_matcher(&inputs).unwrap();
        assert_eq!(
            m.match_version("com.example/lib@v2.0.0"),
            Some(semver::Version::new(2, 0, 0))
        );
        // Different artifact under the same group must not match.
        assert_eq!(m.match_version("com.example/other@v2.0.0"), None);
    }

    #[test]
    fn matcher_pypa_no_v_prefix() {
        // pypa default is `{name}-{version}` — no `v`. The bug would
        // have matched any `name-*` prefixed tag; the regex anchor
        // makes sure only the project's actual release tags match.
        let inputs = TagPatternInputs {
            project_name: "mylib",
            ecosystem: "pypa",
            ecosystem_default: "{name}-{version}",
            allowed_vars: &["name", "version", "ecosystem"],
            override_template: None,
            maven_coords: None,
            module_path: None,
            allow_bare_v_fallback: false,
        };
        let m = build_tag_matcher(&inputs).unwrap();
        assert_eq!(
            m.match_version("mylib-1.2.3"),
            Some(semver::Version::new(1, 2, 3))
        );
        // Confusing prefix — `mylib-extra-1.0.0` is NOT `mylib`.
        assert_eq!(m.match_version("mylib-extra-1.0.0"), None);
        // No leading `v` accepted (pypa convention).
        assert_eq!(m.match_version("mylib-v1.2.3"), None);
    }

    #[test]
    fn matcher_go_module_path_with_slash() {
        let inputs = TagPatternInputs {
            project_name: "github.com/org/repo/v2",
            ecosystem: "go",
            ecosystem_default: "{module}/v{version}",
            allowed_vars: &["module", "version", "ecosystem", "name"],
            override_template: None,
            maven_coords: None,
            module_path: Some("github.com/org/repo/v2"),
            allow_bare_v_fallback: false,
        };
        let m = build_tag_matcher(&inputs).unwrap();
        assert_eq!(
            m.match_version("github.com/org/repo/v2/v2.3.4"),
            Some(semver::Version::new(2, 3, 4))
        );
    }

    #[test]
    fn matcher_override_wins_over_ecosystem_default() {
        let mut inputs = npm_pattern_inputs();
        inputs.override_template = Some("release/{name}/{version}");
        let m = build_tag_matcher(&inputs).unwrap();
        assert_eq!(
            m.match_version("release/@clikd/landing/0.7.0"),
            Some(semver::Version::new(0, 7, 0))
        );
        // The npm default must NOT match when an override is in force.
        assert_eq!(m.match_version("@clikd/landing@v0.7.0"), None);
    }

    #[test]
    fn matcher_single_project_bare_v_fallback() {
        // A single-project cargo repo tagging plain `v1.2.3` (no
        // `name-v` prefix). With `allow_bare_v_fallback`, the matcher
        // accepts these even though the configured template is
        // `{name}-v{version}`.
        let inputs = TagPatternInputs {
            project_name: "my-crate",
            ecosystem: "cargo",
            ecosystem_default: "{name}-v{version}",
            allowed_vars: &["name", "version", "ecosystem"],
            override_template: None,
            maven_coords: None,
            module_path: None,
            allow_bare_v_fallback: true,
        };
        let m = build_tag_matcher(&inputs).unwrap();
        assert_eq!(
            m.match_version("v1.2.3"),
            Some(semver::Version::new(1, 2, 3))
        );
        assert_eq!(
            m.match_version("my-crate-v1.2.3"),
            Some(semver::Version::new(1, 2, 3))
        );
        // Without the fallback flag, `v1.2.3` must not match.
        let mut inputs_no_fb = inputs;
        inputs_no_fb.allow_bare_v_fallback = false;
        let m2 = build_tag_matcher(&inputs_no_fb).unwrap();
        assert_eq!(m2.match_version("v1.2.3"), None);
    }

    #[test]
    fn matcher_template_missing_version_is_hard_error() {
        let mut inputs = npm_pattern_inputs();
        inputs.override_template = Some("static-tag-no-version");
        let err = build_tag_matcher(&inputs).unwrap_err();
        assert!(format!("{err:#}").contains("does not reference `{version}`"));
    }

    #[test]
    fn matcher_picks_highest_version_via_caller_sort() {
        // The matcher itself only matches; sorting belongs to the
        // caller in `find_latest_tag_for_project`. Verify the matcher
        // surfaces the captured version so the caller can sort.
        let m = build_tag_matcher(&npm_pattern_inputs()).unwrap();
        let tags = [
            "@clikd/landing@v0.6.0",
            "@clikd/landing@v0.7.0",
            "@clikd/landing@v0.6.99",
        ];
        let highest = tags
            .iter()
            .filter_map(|t| m.match_version(t))
            .max()
            .unwrap();
        assert_eq!(highest, semver::Version::new(0, 7, 0));
    }
}
