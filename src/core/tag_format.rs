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
}
