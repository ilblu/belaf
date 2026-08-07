//! `[cascade_inputs]` end-to-end.
//!
//! A declared path input is a file tree that feeds release units without
//! being one itself (a shared OCI base image, protobuf schemas, a shared
//! config tree). A change under its `paths` must bump every unit it
//! `affects`, while the input itself is never versioned, tagged or released.
//!
//! Like the other `prepare` integration tests these let `prepare --ci` fail at
//! the (offline, unauthenticated) push/PR step and assert on the on-disk
//! artifacts — by then the manifest and changelogs have been written. See the
//! header of `test_prepare_manifest_emission.rs`.

mod common;

use common::TestRepo;

const SCHEMA_PATH: &str = "schemas/manifest.v1.schema.json";

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn manifest_json(repo: &TestRepo) -> serde_json::Value {
    let path = manifest_path(repo).expect("a release manifest json must exist");
    let body = std::fs::read_to_string(path).expect("read manifest");
    serde_json::from_str(&body).expect("manifest must be valid json")
}

fn manifest_path(repo: &TestRepo) -> Option<std::path::PathBuf> {
    let dir = repo.path.join("belaf/releases");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find_map(|e| {
            let p = e.path();
            (p.extension().is_some_and(|x| x == "json")).then_some(p)
        })
}

fn released_names(manifest: &serde_json::Value) -> Vec<String> {
    manifest["releases"]
        .as_array()
        .map(|rs| {
            rs.iter()
                .filter_map(|r| r["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn release_of<'a>(manifest: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    manifest["releases"]
        .as_array()
        .expect("releases array")
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("no release for `{name}` in {manifest}"))
}

fn bump_of(manifest: &serde_json::Value, name: &str) -> String {
    release_of(manifest, name)["bump_type"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn validate_against_schema(json_value: &serde_json::Value) {
    let schema_raw = std::fs::read_to_string(SCHEMA_PATH).expect("read schema");
    let schema_json: serde_json::Value = serde_json::from_str(&schema_raw).expect("parse schema");
    let validator = jsonschema::draft202012::new(&schema_json).expect("compile schema");
    if let Err(err) = validator.validate(json_value) {
        panic!(
            "manifest fails schema validation: {err}\n\nmanifest:\n{}",
            serde_json::to_string_pretty(json_value).unwrap_or_default()
        );
    }
}

/// Two independent deploy services in one cargo workspace, both already
/// tagged so `prepare`'s window starts empty.
fn scaffold_two_services(repo: &TestRepo) {
    scaffold_two_services_seeded(repo, &[]);
}

/// As [`scaffold_two_services`], but `seed` files are written and committed
/// **before** the release tags are cut. Anything seeded this way is outside
/// the window `prepare` analyzes, so a test can assert that a later change is
/// what triggered a bump rather than the file's mere existence.
fn scaffold_two_services_seeded(repo: &TestRepo, seed: &[(&str, &str)]) {
    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n",
    );
    for name in ["gate", "rig"] {
        repo.write_file(
            &format!("crates/{name}/Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nedition = \"2021\"\n"),
        );
        repo.write_file(&format!("crates/{name}/src/main.rs"), "fn main() {}\n");
    }
    for (path, contents) in seed {
        repo.write_file(path, contents);
    }
    repo.commit("Initial commit");

    let lock = std::process::Command::new("cargo")
        .args(["generate-lockfile"])
        .current_dir(&repo.path)
        .output()
        .expect("cargo generate-lockfile");
    assert!(
        lock.status.success(),
        "generate-lockfile failed:\n{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    repo.commit("chore: lockfile");
    repo.tag("gate-v1.0.0");
    repo.tag("rig-v1.0.0");
}

fn prepare(repo: &TestRepo) -> std::process::Output {
    repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")])
}

// ---------------------------------------------------------------------------
// path forms
// ---------------------------------------------------------------------------

#[test]
fn directory_glob_form_still_cascades() {
    // The `[codegen_edges]` shape that used to work (`proto/**` → a directory
    // prefix) must keep working under the new syntax.
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.proto]\npaths = [\"proto/**\"]\naffects = [\"gate\"]\n",
    );
    repo.write_file("proto/schema.proto", "message A {}\n");
    repo.commit("fix(proto): correct a field number");

    let _ = prepare(&repo);

    let manifest = manifest_json(&repo);
    let names = released_names(&manifest);
    assert!(
        names.iter().any(|n| n == "gate"),
        "gate must bump on a change under the declared input; released: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "rig"),
        "rig is not affected by this input; released: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "proto"),
        "the input node is internal and must never be released; released: {names:?}"
    );
}

#[test]
fn literal_directory_form_cascades() {
    // A plain directory path (no glob metacharacters) becomes a prefix
    // include, the same behaviour `[codegen_edges]` had for every entry.
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.proto]\npaths = [\"proto\"]\naffects = [\"gate\"]\n",
    );
    repo.write_file("proto/deep/nested/schema.proto", "message A {}\n");
    repo.commit("fix(proto): correct a nested field number");

    let _ = prepare(&repo);

    let names = released_names(&manifest_json(&repo));
    assert!(
        names.iter().any(|n| n == "gate"),
        "gate must bump on a change under the literal directory input; released: {names:?}"
    );
}

#[test]
fn file_glob_form_cascades() {
    // The actual bug this feature fixes. `[codegen_edges]` turned every entry
    // into a directory prefix, so `apko/*.lock` became the never-matching
    // prefix `apko/*.lock/` — a silent no-op with no warning.
    let repo = TestRepo::new();
    // Seed both files before the tags so only the later *.lock edit is inside
    // the analyzed window — otherwise the literal `apko/base.yaml` entry would
    // carry the test and a broken glob would still look green.
    scaffold_two_services_seeded(
        &repo,
        &[
            ("apko/base.yaml", "contents:\n  packages: [zlib]\n"),
            ("apko/base.lock", "{ \"version\": 1 }\n"),
        ],
    );
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/base.yaml\", \"apko/*.lock\"]\n\
         affects = [\"gate\", \"rig\"]\n",
    );

    // Only the *.lock file changes — this is the file-glob path.
    repo.write_file(
        "apko/base.lock",
        "{ \"version\": 2, \"zlib\": \"1.3.1\" }\n",
    );
    repo.commit("fix(apko-base): patch zlib CVE");

    let _ = prepare(&repo);

    let names = released_names(&manifest_json(&repo));
    assert!(
        names.iter().any(|n| n == "gate") && names.iter().any(|n| n == "rig"),
        "a change matching the file glob `apko/*.lock` must cascade to both \
         affected units; released: {names:?}"
    );
}

#[test]
fn affects_all_deploy_units() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\naffects = \"all-deploy-units\"\n",
    );
    repo.write_file("apko/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("fix(apko-base): patch zlib CVE");

    let _ = prepare(&repo);

    let names = released_names(&manifest_json(&repo));
    assert!(
        names.iter().any(|n| n == "gate") && names.iter().any(|n| n == "rig"),
        "`all-deploy-units` must pull in every deploy unit; released: {names:?}"
    );
}

#[test]
fn input_paths_may_overlap_a_real_units_directory() {
    // A declared input inside a service's own tree is legitimate: the file
    // belongs to `gate` AND feeds `rig`. The Tier-3 glob overlap guard, which
    // hard-errors when a path is claimed by more than one unit and any of them
    // carries globs, must not fire on the input.
    let repo = TestRepo::new();
    scaffold_two_services_seeded(
        &repo,
        &[(
            "crates/gate/apko/base.yaml",
            "contents:\n  packages: [zlib]\n",
        )],
    );
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.shared-base]\npaths = [\"crates/gate/apko/*.yaml\"]\n\
         affects = [\"rig\"]\n",
    );

    repo.write_file(
        "crates/gate/apko/base.yaml",
        "contents:\n  packages: [zlib, openssl]\n",
    );
    repo.commit("fix(shared-base): patch openssl");

    let out = prepare(&repo);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("claimed by multiple release units"),
        "the overlap guard must not fire on a cascade-input node:\n{stderr}"
    );

    let names = released_names(&manifest_json(&repo));
    assert!(
        names.iter().any(|n| n == "gate"),
        "gate owns the changed path and must bump; released: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "rig"),
        "rig is fed by the input and must bump too; released: {names:?}"
    );
}

// ---------------------------------------------------------------------------
// bump floor
// ---------------------------------------------------------------------------

#[test]
fn bump_floor_minor_escalates_a_patch() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\n\
         affects = \"all-deploy-units\"\nbump = \"floor_minor\"\n",
    );
    repo.write_file("apko/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("fix(apko-base): patch zlib CVE");

    let _ = prepare(&repo);

    let manifest = manifest_json(&repo);
    assert_eq!(
        bump_of(&manifest, "gate"),
        "minor",
        "`bump = \"floor_minor\"` must raise the fix-derived patch to a minor: {manifest}"
    );
    assert_eq!(bump_of(&manifest, "rig"), "minor");
}

#[test]
fn bump_floor_does_not_apply_without_an_input_hit() {
    // The floor must never resurrect a unit with no commits, and must not
    // touch a unit whose commits did not come through the input.
    let repo = TestRepo::new();
    scaffold_two_services_seeded(
        &repo,
        &[("apko/base.yaml", "contents:\n  packages: [zlib]\n")],
    );
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\n\
         affects = \"all-deploy-units\"\nbump = \"floor_major\"\n",
    );

    // A change in gate only. The input is untouched.
    repo.write_file("crates/gate/src/extra.rs", "pub fn extra() {}\n");
    repo.commit("fix(gate): correct a thing");

    let _ = prepare(&repo);

    let manifest = manifest_json(&repo);
    let names = released_names(&manifest);
    assert!(
        !names.iter().any(|n| n == "rig"),
        "rig has no commits at all — a declared input's floor must not resurrect \
         it; released: {names:?}"
    );
    assert_eq!(
        bump_of(&manifest, "gate"),
        "patch",
        "gate's commit did not come through the input, so the floor must not \
         apply: {manifest}"
    );
    assert!(
        release_of(&manifest, "gate")
            .get("cascade_inputs")
            .and_then(|v| v.as_array())
            .is_none_or(|a| a.is_empty()),
        "gate must carry no cascade_inputs provenance: {manifest}"
    );
}

// ---------------------------------------------------------------------------
// manifest provenance
// ---------------------------------------------------------------------------

#[test]
fn manifest_records_cascade_input_provenance_and_validates() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\n\
         affects = \"all-deploy-units\"\nbump = \"floor_minor\"\n",
    );
    repo.write_file("apko/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("fix(apko-base): patch zlib CVE");

    let _ = prepare(&repo);

    let manifest = manifest_json(&repo);
    validate_against_schema(&manifest);

    for unit in ["gate", "rig"] {
        let inputs = release_of(&manifest, unit)["cascade_inputs"]
            .as_array()
            .unwrap_or_else(|| panic!("`{unit}` must carry cascade_inputs: {manifest}"));
        assert_eq!(inputs.len(), 1, "{manifest}");
        assert_eq!(inputs[0]["name"], "apko-base");
        assert_eq!(
            inputs[0]["bump"], "floor_minor",
            "the declared bump must round-trip into the manifest: {manifest}"
        );
    }
}

#[test]
fn manifest_omits_declared_bump_when_the_user_did_not_write_one() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\naffects = [\"gate\"]\n",
    );
    repo.write_file("apko/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("fix(apko-base): patch zlib CVE");

    let _ = prepare(&repo);

    let manifest = manifest_json(&repo);
    validate_against_schema(&manifest);
    let inputs = release_of(&manifest, "gate")["cascade_inputs"]
        .as_array()
        .expect("gate must carry cascade_inputs");
    assert_eq!(inputs[0]["name"], "apko-base");
    assert!(
        inputs[0].get("bump").is_none_or(serde_json::Value::is_null),
        "no `bump` was declared, so none must be reported: {manifest}"
    );
}

// ---------------------------------------------------------------------------
// hard errors
// ---------------------------------------------------------------------------

#[test]
fn name_collision_with_a_release_unit_is_a_clear_error() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.gate]\npaths = [\"apko/**\"]\naffects = [\"rig\"]\n",
    );
    repo.write_file("apko/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("fix(gate): patch zlib CVE");

    let out = prepare(&repo);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "must fail; stderr:\n{stderr}");
    assert!(
        stderr.contains("collides with an existing release unit"),
        "the collision must be reported by name, not as `unrecognized project \
         name`:\n{stderr}"
    );
    assert!(
        stderr.contains("cascade_inputs.gate"),
        "the error must name the offending block:\n{stderr}"
    );
}

#[test]
fn legacy_codegen_edges_aborts_with_a_migration_block() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[codegen_edges]\n\"proto/**\" = [\"gate\", \"rig\"]\n",
    );
    repo.commit("keep the old config");

    let out = prepare(&repo);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "must fail; stderr:\n{stderr}");
    assert!(
        stderr.contains("[codegen_edges]` was removed"),
        "the error must explain the removal rather than emitting a serde blob:\n{stderr}"
    );
    // The replacement block is rendered ready to paste: the key keeps the name
    // the node used to get from the glob's last path segment.
    assert!(
        stderr.contains("[cascade_inputs.proto]"),
        "the migration message must render the replacement block:\n{stderr}"
    );
    assert!(
        stderr.contains("paths   = [\"proto/**\"]"),
        "the rendered block must carry the original glob:\n{stderr}"
    );
    assert!(
        stderr.contains("affects = [\"gate\", \"rig\"]"),
        "the rendered block must carry the original consumers:\n{stderr}"
    );
}

#[test]
fn empty_affects_list_is_rejected() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\naffects = []\n",
    );
    repo.commit("add a no-op input");

    let out = prepare(&repo);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "must fail; stderr:\n{stderr}");
    assert!(
        stderr.contains("affects = []"),
        "an input that affects nothing must be rejected loudly:\n{stderr}"
    );
}

#[test]
fn empty_paths_list_is_rejected() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = []\naffects = [\"gate\"]\n",
    );
    repo.commit("add a no-op input");

    let out = prepare(&repo);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "must fail; stderr:\n{stderr}");
    assert!(
        stderr.contains("paths = []"),
        "an input with no paths must be rejected loudly:\n{stderr}"
    );
}

#[test]
fn unknown_affects_shorthand_names_the_valid_one() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\naffects = \"all-units\"\n",
    );
    repo.commit("typo the shorthand");

    let out = prepare(&repo);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "must fail; stderr:\n{stderr}");
    assert!(
        stderr.contains("all-deploy-units"),
        "the error must name the only valid shorthand:\n{stderr}"
    );
}

#[test]
fn a_path_binary_affecting_would_filter_out_is_warned_about() {
    // Not an error — the path may become live if `[binary_affecting]` changes —
    // but silence here is the same silent-no-op class the whole feature exists
    // to remove, so it must be said out loud.
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        // `docs` is in the default `[binary_affecting] exclude_segments`, so no
        // change under it ever counts toward a bump.
        "[cascade_inputs.handbook]\npaths = [\"docs/**\"]\naffects = [\"gate\"]\n",
    );
    repo.write_file("docs/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("fix(handbook): patch zlib CVE");

    let out = prepare(&repo);
    // `tracing_subscriber::fmt()` writes to stdout; diagnostics go to stderr.
    // Search both so this doesn't depend on which stream the warning lands on.
    let logs = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        logs.contains("cascade_inputs.handbook") && logs.contains("binary_affecting"),
        "a permanently-filtered input path must be warned about:\n{logs}"
    );
    // Warning, not error: the run itself still succeeds.
    assert!(
        out.status.success(),
        "a filtered path is a warning, not a failure:\n{logs}"
    );
}

#[test]
fn unknown_bump_strategy_lists_the_valid_ones() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\n\
         affects = [\"gate\"]\nbump = \"floor_massive\"\n",
    );
    repo.commit("typo the bump");

    let out = prepare(&repo);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "must fail; stderr:\n{stderr}");
    assert!(
        stderr.contains("floor_minor"),
        "the error must list the valid strategies:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// user-visible name: `belaf check` scopes + changelog `via`
// ---------------------------------------------------------------------------

fn check_message(repo: &TestRepo, message: &str) -> std::process::Output {
    repo.run_belaf_command_with_env(
        &["check", "--ci", "--message", message],
        &[("BELAF_NO_KEYRING", "1")],
    )
}

#[test]
fn input_name_is_a_valid_commit_scope() {
    // `belaf check` treats internal unit names as known scopes, and a
    // cascade-input node is one — so the config key is a valid commit scope.
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\naffects = \"all-deploy-units\"\n",
    );
    repo.write_file("apko/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("seed apko base");

    let ok = check_message(&repo, "fix(apko-base): patch zlib");
    assert!(
        ok.status.success(),
        "`fix(apko-base):` must be a known scope:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&ok.stdout),
        String::from_utf8_lossy(&ok.stderr)
    );

    let bogus = check_message(&repo, "fix(nowhere): patch zlib");
    assert!(
        !bogus.status.success(),
        "an unrelated scope must still be a violation:\nstdout: {}",
        String::from_utf8_lossy(&bogus.stdout)
    );
}

#[test]
fn accepted_scope_set_follows_the_config_key_under_exact_matching() {
    // The node name is now the config KEY, not the glob's last path segment
    // (`[codegen_edges] "apko/**"` used to produce a node called `apko`). Under
    // `scope_matching = "exact"` that shift is directly observable:
    // `fix(apko-base):` is valid and the pre-rename `fix(apko):` is not.
    //
    // Under the default `scope_matching = "smart"` the `contains` fallback
    // still resolves `apko` to `apko-base`, which softens the migration but
    // hides the change — hence pinning it in `exact` mode.
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[commit_attribution]\nscope_matching = \"exact\"\n\n\
         [cascade_inputs.apko-base]\npaths = [\"apko/**\"]\naffects = \"all-deploy-units\"\n",
    );
    repo.write_file("apko/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("seed apko base");

    let ok = check_message(&repo, "fix(apko-base): patch zlib");
    assert!(
        ok.status.success(),
        "`fix(apko-base):` must be a known scope:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&ok.stdout),
        String::from_utf8_lossy(&ok.stderr)
    );

    let stale = check_message(&repo, "fix(apko): patch zlib");
    assert!(
        !stale.status.success(),
        "the pre-rename scope `apko` must be reported as a violation:\nstdout: {}",
        String::from_utf8_lossy(&stale.stdout)
    );
    let stderr = String::from_utf8_lossy(&stale.stderr);
    assert!(
        stderr.contains("apko") && stderr.contains("not a known release unit"),
        "the violation must name the bad scope:\n{stderr}"
    );
}

#[test]
fn cascaded_commit_lands_in_the_changelog_with_a_via_prefix() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\naffects = \"all-deploy-units\"\n",
    );
    repo.write_file("apko/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("fix(apko-base): patch zlib CVE");

    let _ = prepare(&repo);

    let manifest = manifest_json(&repo);
    for unit in ["gate", "rig"] {
        let changelog = release_of(&manifest, unit)["changelog"]
            .as_str()
            .unwrap_or_default();
        assert!(
            changelog.contains("via apko-base"),
            "`{unit}`'s changelog must attribute the cascaded commit to the input \
             it came through; got:\n{changelog}"
        );
    }
}

// ---------------------------------------------------------------------------
// `affects` naming a glob-form `[release_unit.<key>]`
// ---------------------------------------------------------------------------

// NOTE: the positive path — `affects = ["<glob key>"]` actually expanding to
// the glob's units — has no integration test yet. Writing one requires a
// glob-form `[release_unit]` over crates that auto-discovery also finds, and
// that combination currently aborts with "multiple projects with same name"
// regardless of `[cascade_inputs]`. That collision is a separate, pre-existing
// bug; the expansion logic itself is covered by the two error cases below,
// which exercise the same resolution path.

/// A name that is neither a unit nor a glob key used to be a `warn!`, so a
/// typo produced a release that quietly left every intended service out.
#[test]
fn unknown_affects_name_is_a_hard_error() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    repo.write_file(
        "belaf/config.toml",
        "[cascade_inputs.apko-base]\npaths = [\"apko/**\"]\naffects = [\"servcies\"]\n",
    );
    repo.write_file("apko/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("fix(apko-base): patch zlib CVE");

    let out = prepare(&repo);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(!out.status.success(), "a typo must not pass silently");
    assert!(
        stderr.contains("servcies") && stderr.contains("neither a known release unit"),
        "the error should name the offending entry; stderr:\n{stderr}"
    );
}

/// If a name means two different things, releasing either set would be a
/// guess. Say so instead.
#[test]
fn ambiguous_affects_name_is_a_hard_error() {
    let repo = TestRepo::new();
    scaffold_two_services(&repo);
    // `gate` is a real unit AND the key of a glob covering both services.
    repo.write_file(
        "belaf/config.toml",
        "[release_unit.gate]\nglob = \"crates/*\"\nname = \"{basename}\"\n\
         ecosystem = \"cargo\"\nmanifests = [\"{path}/Cargo.toml\"]\n\n\
         [cascade_inputs.apko-base]\npaths = [\"apko/**\"]\naffects = [\"gate\"]\n",
    );
    repo.write_file("apko/base.yaml", "contents:\n  packages: [zlib]\n");
    repo.commit("fix(apko-base): patch zlib CVE");

    let out = prepare(&repo);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "an ambiguous name must not be guessed"
    );
    assert!(
        stderr.contains("ambiguous"),
        "the error should call out the ambiguity; stderr:\n{stderr}"
    );
}
