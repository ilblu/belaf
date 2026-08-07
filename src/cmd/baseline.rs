//! `belaf baseline` — report (and optionally fix) release units that have
//! no release tag and no history baseline.
//!
//! `belaf prepare` refuses to analyze a `kind = "deploy"` unit whose tag
//! template matches nothing in a repo that already carries version-shaped
//! tags: walking the full history would over-count old commits and inflate
//! the bump. The refusal is correct; what was missing was a way to *answer*
//! it per unit. This command surfaces the exact set `prepare` would refuse
//! on — the same [`AppSession::untagged_deploy_units`] call, so the two can
//! never drift — and `--fix` writes the answer into `belaf/config.toml`.
//!
//! The write is comment- and format-preserving (`toml_edit`) and validated
//! before it touches disk, following `cmd::init::auto_detect::apply`.

use anyhow::{Context as _, Result};
use owo_colors::OwoColorize;

use crate::core::{
    exit_code::ExitCode,
    git::history::UntaggedDeployUnit,
    release_unit::{BaselineSpec, ResolveOrigin},
    session::{AppBuilder, AppSession},
};

/// Comment written above every generated `baseline` key so the next reader
/// knows what put it there and when it stops being needed.
const GENERATED_COMMENT: &str = "\
# `belaf baseline --fix`: no release tag matched this unit, so analyze from repo
# start. A real tag wins over this key — it is a no-op after the first release.
";

/// Structured `--ci` payload. The only thing on stdout in `--ci` mode.
#[derive(serde::Serialize)]
struct CiStatus {
    /// Stable, snake_case label. One of: `ok` (nothing to do),
    /// `needs_baseline` (units reported, nothing written),
    /// `fixed` (at least one `baseline` key written).
    status: &'static str,

    /// The `belaf/config.toml` this command read (and, with `--fix`, wrote).
    config_path: String,

    /// Every unit with no release tag and no baseline, in graph order.
    units: Vec<CiStatusUnit>,

    /// Config keys a `baseline = "first-release"` was written for. Always
    /// empty without `--fix`.
    written: Vec<String>,

    /// Units `--fix` deliberately did not touch, with the reason. Always
    /// empty without `--fix`.
    skipped: Vec<CiStatusSkip>,
}

#[derive(serde::Serialize)]
struct CiStatusUnit {
    /// Disambiguated, user-facing unit name.
    name: String,
    /// The `[release_unit.<key>]` key a baseline goes under.
    config_key: String,
    /// The tag template the lookup tried and missed.
    tag_template: String,
}

#[derive(serde::Serialize)]
struct CiStatusSkip {
    config_key: String,
    reason: String,
}

fn emit_ci_status(status: &CiStatus) {
    match serde_json::to_string_pretty(status) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error: failed to serialise --ci status: {e}"),
    }
}

pub fn run(ci: bool, fix: bool) -> Result<i32> {
    // Same pre-flight as `prepare`: the answer depends on which tags exist,
    // and the release tags are created server-side after the manifest PR
    // merges. `BELAF_NO_FETCH=1` bypasses, as everywhere else.
    let sess = AppBuilder::new()?.fetch_tags_first(true).initialize()?;
    let config_path = {
        let mut p = sess.repo.resolve_config_dir();
        p.push("config.toml");
        p
    };

    let units = sess.untagged_deploy_units()?;

    if units.is_empty() {
        if ci {
            emit_ci_status(&CiStatus {
                status: "ok",
                config_path: config_path.display().to_string(),
                units: vec![],
                written: vec![],
                skipped: vec![],
            });
        } else {
            println!();
            println!(
                "{} Every deploy unit has a release tag or a `baseline`.",
                "✓".green().bold()
            );
            println!();
        }
        return Ok(ExitCode::Ok.into());
    }

    if !fix {
        if ci {
            emit_ci_status(&CiStatus {
                status: "needs_baseline",
                config_path: config_path.display().to_string(),
                units: units.iter().map(to_ci_unit).collect(),
                written: vec![],
                skipped: vec![],
            });
        } else {
            print_report(&units, &config_path.display().to_string());
        }
        return Ok(ExitCode::Precondition.into());
    }

    // --fix: writing a bare `[release_unit.<name>]` block for a
    // glob-expanded unit would produce a *partial override* whose name
    // matches no auto-detected unit, i.e. a config that no longer loads.
    // Leave those to the human, naming the glob block they belong to.
    let (writable, skipped) = partition_writable(&sess, &units);

    let outcome = apply_baselines(&config_path, &writable)
        .with_context(|| format!("writing baselines into {}", config_path.display()))?;

    if ci {
        emit_ci_status(&CiStatus {
            status: if outcome.written.is_empty() {
                "needs_baseline"
            } else {
                "fixed"
            },
            config_path: config_path.display().to_string(),
            units: units.iter().map(to_ci_unit).collect(),
            written: outcome.written.clone(),
            skipped,
        });
    } else {
        print_fix_result(&outcome, &skipped, &config_path.display().to_string());
    }

    // Anything left unfixed still blocks `prepare`, so it must not read as
    // success.
    if outcome.written.len() == units.len() {
        Ok(ExitCode::Ok.into())
    } else {
        Ok(ExitCode::Precondition.into())
    }
}

fn to_ci_unit(u: &UntaggedDeployUnit) -> CiStatusUnit {
    CiStatusUnit {
        name: u.name.clone(),
        config_key: u.config_key.clone(),
        tag_template: u.tag_template.clone(),
    }
}

/// Split the reported units into the ones `--fix` may write a per-unit
/// block for and the ones it must not.
fn partition_writable(
    sess: &AppSession,
    units: &[UntaggedDeployUnit],
) -> (Vec<String>, Vec<CiStatusSkip>) {
    let mut writable = Vec::new();
    let mut skipped = Vec::new();
    for u in units {
        let origin = sess
            .resolved_release_units()
            .iter()
            .find(|r| r.unit.name == u.config_key)
            .map(|r| &r.origin);
        match origin {
            Some(ResolveOrigin::Glob { .. }) => skipped.push(CiStatusSkip {
                config_key: u.config_key.clone(),
                reason: "unit comes from a glob-form `[release_unit]` block; a bare per-unit \
                         block would be read as a partial override of an auto-detected unit \
                         and fail to resolve. Set `baseline` on the glob block (it applies to \
                         every expansion) or declare this unit explicitly."
                    .to_string(),
            }),
            _ => writable.push(u.config_key.clone()),
        }
    }
    (writable, skipped)
}

/// What [`apply_baselines`] did.
#[derive(Debug, Default, PartialEq, Eq)]
struct FixOutcome {
    /// Config keys a `baseline` was written for.
    written: Vec<String>,
    /// Config keys that already carried a `baseline` and were left alone.
    already_set: Vec<String>,
}

/// Write `baseline = "first-release"` for each key into `config_path`,
/// preserving formatting and comments.
///
/// Idempotent: a `[release_unit.<key>]` that already has a `baseline` is
/// left byte-for-byte alone. Validated before writing — the rendered result
/// is re-parsed and the `[release_unit]` table is deserialised through the
/// real (`deny_unknown_fields`) config type, so a merge that would produce
/// an unloadable config aborts with the file untouched.
fn apply_baselines(config_path: &std::path::Path, keys: &[String]) -> Result<FixOutcome> {
    let mut outcome = FixOutcome::default();
    if keys.is_empty() {
        return Ok(outcome);
    }

    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = existing
        .parse()
        .with_context(|| format!("existing {} is not valid TOML", config_path.display()))?;

    if doc.get("release_unit").is_none() {
        let mut t = toml_edit::Table::new();
        // Implicit so the rendered output is `[release_unit.<name>]` rather
        // than a bare `[release_unit]` header followed by sub-tables.
        t.set_implicit(true);
        doc.insert("release_unit", toml_edit::Item::Table(t));
    }
    let release_unit = doc["release_unit"]
        .as_table_mut()
        .context("`release_unit` exists but is not a table")?;

    for key in keys {
        let entry = release_unit
            .entry(key)
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
        let table = entry.as_table_mut().with_context(|| {
            format!("`[release_unit.{key}]` exists but is not a table — fix it by hand")
        })?;
        if table.contains_key("baseline") {
            outcome.already_set.push(key.clone());
            continue;
        }
        table.insert("baseline", toml_edit::value(BaselineSpec::FIRST_RELEASE));
        if let Some(mut k) = table.key_mut("baseline") {
            k.leaf_decor_mut().set_prefix(GENERATED_COMMENT);
        }
        outcome.written.push(key.clone());
    }

    if outcome.written.is_empty() {
        return Ok(outcome);
    }

    let rendered = doc.to_string();
    validate_rendered(&rendered).with_context(|| {
        format!(
            "the merged result would not load as a belaf config — nothing was written to {}; \
             add the `baseline` keys by hand",
            config_path.display()
        )
    })?;

    std::fs::write(config_path, rendered)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(outcome)
}

/// Re-parse the rendered config before it reaches disk. Two checks: it is
/// still TOML at all, and its `[release_unit]` table still deserialises
/// through the real config type (which is `deny_unknown_fields`, so a
/// misplaced key surfaces here rather than on the user's next run).
fn validate_rendered(rendered: &str) -> Result<()> {
    let value: toml::Value = toml::from_str(rendered).context("result is not valid TOML")?;
    let Some(units) = value.get("release_unit") else {
        return Ok(());
    };
    let _: std::collections::HashMap<String, crate::core::release_unit::syntax::ReleaseUnitConfig> =
        units
            .clone()
            .try_into()
            .context("`[release_unit]` table no longer deserialises")?;
    Ok(())
}

fn print_report(units: &[UntaggedDeployUnit], config_path: &str) {
    let width = units.iter().map(|u| u.name.len()).max().unwrap_or(0);
    println!();
    println!(
        "{} {} release unit{} have no release tag and no `baseline`:",
        "!".yellow().bold(),
        units.len(),
        if units.len() == 1 { "" } else { "s" }
    );
    println!();
    for u in units {
        println!(
            "  {:width$}  tried template {}",
            u.name.bold(),
            format!("`{}`", u.tag_template).dimmed(),
        );
    }
    println!();
    println!(
        "  {} This repo already has version-shaped tags, so `belaf prepare` refuses to",
        "→".dimmed()
    );
    println!(
        "  {} analyze these from repo start — it would over-count old commits.",
        " ".dimmed()
    );
    println!();
    println!("  Answer it per unit in {config_path}:");
    println!();
    let example = &units[0].config_key;
    println!("    [release_unit.{example}]");
    println!(
        "    baseline = \"first-release\"    {}",
        "# never released — analyze from repo start".dimmed()
    );
    println!("    {}", "# ...or...".dimmed());
    println!(
        "    baseline = \"8eb3e3cf78ac\"     {}",
        "# released before belaf — start the window here".dimmed()
    );
    println!();
    println!(
        "  {} If the tags DO exist and just look different, fix `tag_format` instead.",
        "→".dimmed()
    );
    println!();
    println!(
        "  Run {} to write `baseline = \"first-release\"` for all of them.",
        "belaf baseline --fix".cyan()
    );
    println!();
}

fn print_fix_result(outcome: &FixOutcome, skipped: &[CiStatusSkip], config_path: &str) {
    println!();
    if outcome.written.is_empty() {
        println!("{} Nothing to write.", "ℹ".cyan().bold());
    } else {
        println!(
            "{} Wrote `baseline = \"first-release\"` for {} release unit{} in {config_path}:",
            "✓".green().bold(),
            outcome.written.len(),
            if outcome.written.len() == 1 { "" } else { "s" }
        );
        for k in &outcome.written {
            println!("  {} {k}", "•".dimmed());
        }
    }
    if !outcome.already_set.is_empty() {
        println!();
        println!("  Already had a `baseline`, left alone:");
        for k in &outcome.already_set {
            println!("  {} {k}", "•".dimmed());
        }
    }
    for s in skipped {
        println!();
        println!(
            "{} `{}` was not written: {}",
            "!".yellow().bold(),
            s.config_key,
            s.reason
        );
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &tempfile::TempDir, content: &str) -> std::path::PathBuf {
        let p = dir.path().join("config.toml");
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn writes_a_bare_block_and_preserves_surrounding_comments() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = write(
            &dir,
            "# hand-written header\n[repo]\nrelease_branch = \"release\"\n",
        );

        let outcome = apply_baselines(&cfg, &["desktop".to_string()]).unwrap();
        assert_eq!(outcome.written, vec!["desktop".to_string()]);
        assert!(outcome.already_set.is_empty());

        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(after.contains("# hand-written header"), "{after}");
        assert!(after.contains("release_branch = \"release\""), "{after}");
        assert!(after.contains("[release_unit.desktop]"), "{after}");
        assert!(after.contains("baseline = \"first-release\""), "{after}");
        assert!(
            after.contains("`belaf baseline --fix`: no release tag matched"),
            "the generated key must carry its explanation:\n{after}"
        );
        assert!(
            !after.contains("\n[release_unit]\n"),
            "the parent table must stay implicit:\n{after}"
        );
        toml::from_str::<toml::Value>(&after).expect("written config must parse");
    }

    #[test]
    fn merges_into_an_existing_explicit_block() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = write(
            &dir,
            "[release_unit.docs]\n# keep me\necosystem = \"npm\"\nmanifests = [{ path = \"docs/package.json\", version_field = \"npm_package_json\" }]\n",
        );

        apply_baselines(&cfg, &["docs".to_string()]).unwrap();
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(
            after.matches("[release_unit.docs]").count(),
            1,
            "must merge, not duplicate the table:\n{after}"
        );
        assert!(after.contains("# keep me"), "{after}");
        assert!(after.contains("ecosystem = \"npm\""), "{after}");
        assert!(after.contains("baseline = \"first-release\""), "{after}");
    }

    #[test]
    fn second_run_leaves_the_file_byte_identical() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = write(&dir, "# base\n");

        apply_baselines(&cfg, &["a".to_string(), "b".to_string()]).unwrap();
        let after_first = std::fs::read_to_string(&cfg).unwrap();

        let outcome = apply_baselines(&cfg, &["a".to_string(), "b".to_string()]).unwrap();
        assert!(outcome.written.is_empty(), "{outcome:?}");
        assert_eq!(
            outcome.already_set,
            vec!["a".to_string(), "b".to_string()],
            "{outcome:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            after_first,
            "a second --fix must not rewrite the file at all"
        );
    }

    #[test]
    fn an_existing_baseline_is_never_overwritten() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = write(&dir, "[release_unit.docs]\nbaseline = \"abc1234\"\n");

        let outcome = apply_baselines(&cfg, &["docs".to_string()]).unwrap();
        assert!(outcome.written.is_empty());
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(after.contains("baseline = \"abc1234\""), "{after}");
        assert!(!after.contains("first-release"), "{after}");
    }

    #[test]
    fn empty_key_list_is_a_noop() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = write(&dir, "# untouched\n");
        let outcome = apply_baselines(&cfg, &[]).unwrap();
        assert_eq!(outcome, FixOutcome::default());
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "# untouched\n");
    }

    #[test]
    fn invalid_existing_toml_aborts_without_writing() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = write(&dir, "[release_unit.docs\n");
        let err = apply_baselines(&cfg, &["docs".to_string()]);
        assert!(
            err.is_err(),
            "malformed TOML must not be silently rewritten"
        );
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            "[release_unit.docs\n"
        );
    }

    #[test]
    fn a_non_table_release_unit_entry_is_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = write(&dir, "[release_unit]\ndocs = \"oops\"\n");
        let err = apply_baselines(&cfg, &["docs".to_string()]);
        assert!(
            err.is_err(),
            "a scalar under [release_unit] must be an error"
        );
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            "[release_unit]\ndocs = \"oops\"\n"
        );
    }

    #[test]
    fn validate_rendered_rejects_an_unknown_release_unit_field() {
        let err = validate_rendered("[release_unit.docs]\nbaselin = \"first-release\"\n");
        assert!(
            err.is_err(),
            "deny_unknown_fields must catch a typo'd key before it reaches disk"
        );
    }
}
