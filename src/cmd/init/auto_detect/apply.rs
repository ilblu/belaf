//! Validated, merge-aware writers for auto-detect output. Everything
//! that mutates `belaf/config.toml` on disk lives here; emission (what
//! to write) stays in the parent module.

use std::path::Path;

use anyhow::Context as _;

use super::{
    advice_comments, allow_uncovered_paths_line, AutoDetectResult, ALLOW_UNCOVERED_HEADER,
    AUTO_DETECT_MARKER, CODEGEN_STUB, MODEL_HEADER,
};

/// Coverage-filtered config update for auto-detect results (first init
/// AND re-runs — `--force` in `--ci`, or the wizard on an existing
/// config). Not gated on the marker: per-path filtering via
/// [`super::ExistingDecisions`] is what makes re-appends idempotent. It
/// never duplicates tables:
///
/// - the body (`[release_unit.<name>]` blocks + comments) is appended
///   as text so the emitted comment blocks survive,
/// - new `[allow_uncovered]` paths are merged into an existing table
///   via `toml_edit` (format- and comment-preserving); the table is
///   only emitted as text when the config has none yet,
/// - the merged result is parsed BEFORE writing — a residual collision
///   (e.g. a re-emitted unit name that already exists as a table)
///   aborts with an error instead of corrupting config.toml.
///
/// `first_run` prepends the marker + model header + codegen stub +
/// advice comments, so a fresh init produces the same file as the
/// historical single-shot append; re-runs append the bare body and
/// leave advice to the caller's log / CI status.
pub fn apply_to_config(
    config_path: &Path,
    result: &AutoDetectResult,
    first_run: bool,
) -> anyhow::Result<()> {
    let advice_relevant = first_run && !result.advice.is_empty();
    if result.body_snippet.is_empty() && result.allow_uncovered_paths.is_empty() && !advice_relevant
    {
        return Ok(());
    }
    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    let existing_doc: toml_edit::DocumentMut = existing
        .parse()
        .with_context(|| format!("existing {} is not valid TOML", config_path.display()))?;
    let merge_allow =
        !result.allow_uncovered_paths.is_empty() && existing_doc.contains_key("allow_uncovered");

    let mut body = result.body_snippet.clone();
    // Advice becomes config comments exactly once — on the first run.
    // Re-runs leave it to the caller's log / CI status.
    if first_run {
        body.push_str(&advice_comments(&result.advice));
    }
    if !result.allow_uncovered_paths.is_empty() && !merge_allow {
        body.push_str(ALLOW_UNCOVERED_HEADER);
        body.push_str(&allow_uncovered_paths_line(&result.allow_uncovered_paths));
    }

    let mut content = existing;
    if !body.is_empty() {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        if first_run {
            content.push_str(&format!(
                "\n{AUTO_DETECT_MARKER}\n{MODEL_HEADER}{body}{CODEGEN_STUB}"
            ));
        } else {
            content.push_str(&body);
        }
    }

    let mut doc: toml_edit::DocumentMut = content.parse().with_context(|| {
        format!(
            "re-detected blocks conflict with the existing {} — nothing was written; add the entries by hand",
            config_path.display()
        )
    })?;

    if merge_allow {
        let table = doc["allow_uncovered"]
            .as_table_mut()
            .context("[allow_uncovered] is not a table")?;
        let arr = table
            .entry("paths")
            .or_insert(toml_edit::value(toml_edit::Array::new()))
            .as_array_mut()
            .context("[allow_uncovered] paths is not an array")?;
        for p in &result.allow_uncovered_paths {
            if !arr.iter().any(|v| v.as_str() == Some(p.as_str())) {
                arr.push(p.as_str());
            }
        }
    }

    std::fs::write(config_path, doc.to_string())
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

/// Append a raw snippet (e.g. the wizard's `[projects.<name>]`
/// tag-format override) after verifying the combined file still parses
/// as TOML. A duplicate table — say, re-running the wizard and picking
/// a tag format for a project that already has one — aborts with an
/// error instead of corrupting config.toml.
pub fn append_validated(config_path: &Path, snippet: &str) -> anyhow::Result<()> {
    if snippet.is_empty() {
        return Ok(());
    }
    let mut content = std::fs::read_to_string(config_path).unwrap_or_default();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(snippet);
    content.parse::<toml_edit::DocumentMut>().with_context(|| {
        format!(
            "snippet conflicts with the existing {} — nothing was written; add the entry by hand",
            config_path.display()
        )
    })?;
    std::fs::write(config_path, content)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_to_config_first_run_emits_marker_and_allow_table() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "").unwrap();
        let result = AutoDetectResult {
            body_snippet: "\n[release_unit.alpha]\necosystem = \"cargo\"\nmanifests = [{ path = \"a/Cargo.toml\", version_field = \"package.version\" }]\n".to_string(),
            allow_uncovered_paths: vec!["apps/ios/".to_string()],
            ..Default::default()
        };
        apply_to_config(&cfg, &result, true).unwrap();
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(after.contains(AUTO_DETECT_MARKER));
        assert!(after.contains("[release_unit.alpha]"));
        assert!(after.contains("[allow_uncovered]"));
        assert!(after.contains("apps/ios/"));
        toml::from_str::<toml::Value>(&after).expect("written config must parse");
    }

    #[test]
    fn apply_to_config_merges_into_existing_allow_uncovered_table() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "# hand-written\n[allow_uncovered]\npaths = [\"apps/android/\"]\n",
        )
        .unwrap();
        let result = AutoDetectResult {
            allow_uncovered_paths: vec!["apps/ios/".to_string()],
            ..Default::default()
        };
        apply_to_config(&cfg, &result, false).unwrap();
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(
            after.matches("[allow_uncovered]").count(),
            1,
            "must merge into the existing table, not append a duplicate header:\n{after}"
        );
        assert!(after.contains("# hand-written"), "comments must survive");
        let parsed: toml::Value = toml::from_str(&after).expect("merged config must parse");
        let paths: Vec<&str> = parsed["allow_uncovered"]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(paths, vec!["apps/android/", "apps/ios/"]);
    }

    #[test]
    fn apply_to_config_rejects_colliding_unit_name_without_writing() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.toml");
        let before = "[release_unit.alpha]\necosystem = \"cargo\"\nmanifests = [{ path = \"a/Cargo.toml\", version_field = \"package.version\" }]\n";
        std::fs::write(&cfg, before).unwrap();
        let result = AutoDetectResult {
            body_snippet: "\n[release_unit.alpha]\necosystem = \"npm\"\n".to_string(),
            ..Default::default()
        };
        let err = apply_to_config(&cfg, &result, false);
        assert!(err.is_err(), "duplicate table must be rejected");
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(after, before, "config must be untouched on rejection");
    }

    #[test]
    fn apply_to_config_is_noop_on_empty_result() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "# untouched\n").unwrap();
        apply_to_config(&cfg, &AutoDetectResult::default(), false).unwrap();
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "# untouched\n");
    }

    #[test]
    fn apply_to_config_renders_advice_only_on_first_run() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "").unwrap();
        let result = AutoDetectResult {
            body_snippet: "\n[release_unit.alpha]\necosystem = \"cargo\"\nmanifests = [{ path = \"a/Cargo.toml\", version_field = \"package.version\" }]\n".to_string(),
            advice: vec![
                "Nested npm workspace detected at web/ — members auto-detected.".to_string(),
            ],
            ..Default::default()
        };
        apply_to_config(&cfg, &result, true).unwrap();
        let after_first = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            after_first.contains("# Nested npm workspace detected"),
            "first run must render advice as comments:\n{after_first}"
        );

        let rerun = AutoDetectResult {
            body_snippet: "\n[release_unit.beta]\necosystem = \"npm\"\nmanifests = [{ path = \"b/package.json\", version_field = \"version\" }]\n".to_string(),
            advice: vec![
                "Nested npm workspace detected at web/ — members auto-detected.".to_string(),
            ],
            ..Default::default()
        };
        apply_to_config(&cfg, &rerun, false).unwrap();
        let after_second = std::fs::read_to_string(&cfg).unwrap();
        assert!(after_second.contains("[release_unit.beta]"));
        assert_eq!(
            after_second
                .matches("Nested npm workspace detected")
                .count(),
            1,
            "re-run must not duplicate advice comments:\n{after_second}"
        );
    }

    #[test]
    fn append_validated_rejects_duplicate_projects_table() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.toml");
        let before = "[projects.tokio]\ntag_format = \"v{version}\"\n";
        std::fs::write(&cfg, before).unwrap();
        let err = append_validated(
            &cfg,
            "\n[projects.tokio]\ntag_format = \"{name}-v{version}\"\n",
        );
        assert!(err.is_err(), "duplicate [projects.<name>] must be rejected");
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), before);
    }

    #[test]
    fn append_validated_appends_new_table() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "# base\n").unwrap();
        append_validated(&cfg, "\n[projects.tokio]\ntag_format = \"v{version}\"\n").unwrap();
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(after.contains("# base"));
        assert!(after.contains("[projects.tokio]"));
        toml::from_str::<toml::Value>(&after).expect("must parse");
    }
}
