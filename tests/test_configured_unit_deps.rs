//! Dependency edges for **configured** release units.
//!
//! `[release_unit.<name>]` blocks — explicit or glob-form — are registered as
//! graph nodes by `add_configured_unit_to_graph`, which is a different code
//! path from the auto-discovered units wired up by `register_discovered_unit`.
//! Until the ownership pass landed, only the latter got dependency edges, so a
//! repo where *both* sides of a dependency are configured had no edge between
//! them at all: a change to a shared library reached no service and `prepare`
//! reported `nothing_to_do`.
//!
//! The shape below is the clikd one that surfaced it: a glob of internal
//! library crates under `packages/*` and a glob of hexagonal services under
//! `apps/services/*`, where the service's unit name (`{basename}`) differs from
//! the cargo package name the dependency edge is expressed in.

mod common;

use common::TestRepo;

fn manifest_json(repo: &TestRepo) -> serde_json::Value {
    let dir = repo.path.join("belaf/releases");
    let entry = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("no belaf/releases dir at {}", dir.display()))
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|x| x == "json"))
        .expect("a release manifest json must exist");
    let body = std::fs::read_to_string(entry.path()).unwrap();
    serde_json::from_str(&body).expect("manifest must be valid json")
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

fn generate_lockfile(repo: &TestRepo) {
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
}

/// The clikd shape, minimised:
///
/// - `packages/observability` → crate `clikd-observability`, unit `observability`
///   (`kind = "internal"`, never released).
/// - `apps/services/gate` → crate `gate` at `crates/bin`, satellite crates
///   `crates/api`; unit `gate`.
///
/// Both the bin crate and the satellite depend on `clikd-observability`, so the
/// edge has to survive the satellite→owner aggregation as well.
fn scaffold_configured_globs(repo: &TestRepo) {
    repo.write_file(
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\n  \"packages/observability\",\n  \
         \"apps/services/gate/crates/api\",\n  \"apps/services/gate/crates/bin\",\n]\n",
    );

    repo.write_file(
        "packages/observability/Cargo.toml",
        "[package]\nname = \"clikd-observability\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("packages/observability/src/lib.rs", "pub fn init() {}\n");

    repo.write_file(
        "apps/services/gate/crates/api/Cargo.toml",
        "[package]\nname = \"gate-api\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\nclikd-observability = { path = \"../../../../../packages/observability\" }\n",
    );
    repo.write_file(
        "apps/services/gate/crates/api/src/lib.rs",
        "pub fn handler() { clikd_observability::init(); }\n",
    );

    repo.write_file(
        "apps/services/gate/crates/bin/Cargo.toml",
        "[package]\nname = \"gate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\ngate-api = { path = \"../api\" }\n",
    );
    repo.write_file(
        "apps/services/gate/crates/bin/src/main.rs",
        "fn main() { gate_api::handler(); }\n",
    );

    repo.write_file(
        "belaf/config.toml",
        "[release_unit.packages]\n\
         kind = \"internal\"\n\
         ecosystem = \"cargo\"\n\
         glob = \"packages/*\"\n\
         name = \"{basename}\"\n\
         manifests = [\"{path}/Cargo.toml\"]\n\
         \n\
         [release_unit.services]\n\
         ecosystem = \"cargo\"\n\
         glob = \"apps/services/*\"\n\
         name = \"{basename}\"\n\
         manifests = [\"{path}/crates/bin/Cargo.toml\"]\n\
         satellites = [\"{path}/crates\"]\n",
    );

    repo.commit("Initial commit");
    generate_lockfile(repo);
    repo.commit("chore: lockfile");
    repo.tag("gate-v0.1.0");
}

#[test]
fn configured_glob_unit_bumps_when_configured_internal_dep_changes() {
    let repo = TestRepo::new();
    scaffold_configured_globs(&repo);

    // A fix that lands only in the shared library crate.
    repo.write_file(
        "packages/observability/src/otlp.rs",
        "pub fn headers() -> &'static str { \"fixed\" }\n",
    );
    repo.commit("fix(observability): send the OTLP headers");

    let out = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    let names = released_names(&manifest_json(&repo));
    assert!(
        names.iter().any(|n| n == "gate"),
        "the configured service unit must bump when the configured internal library \
         crate it depends on changed; released: {names:?}\nstderr:\n{stderr}"
    );
    assert!(
        !names.iter().any(|n| n == "observability"),
        "the internal library unit is a pure cascade node and must never be \
         released; released: {names:?}"
    );
}

#[test]
fn configured_unit_is_not_also_auto_discovered() {
    // The workspace discoverer enumerates every cargo member, including the
    // ones a `[release_unit]` block already claims. Emitting those as
    // standalone units would put two nodes on the same directory — and for a
    // unit whose name equals its crate name (`gate`) the two qualified-name
    // vectors are identical, which is an unresolvable naming clash.
    let repo = TestRepo::new();
    scaffold_configured_globs(&repo);

    repo.write_file(
        "packages/observability/src/otlp.rs",
        "pub fn headers() -> &'static str { \"fixed\" }\n",
    );
    repo.commit("fix(observability): send the OTLP headers");

    let out = repo.run_belaf_command_with_env(&["status", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // Without the ownership pass this is `multiple projects with same name
    // `gate`` — the configured unit and the auto-discovered bin crate collide
    // on identical qualified names, which no disambiguation round can resolve.
    assert!(
        out.status.success(),
        "`belaf status --ci` must succeed on a workspace whose members are \
         claimed by [release_unit] blocks; got:\n{combined}"
    );
    assert!(
        !combined.contains("clikd-observability"),
        "`clikd-observability` is claimed by [release_unit.packages] and must not \
         appear as its own auto-discovered unit; got:\n{combined}"
    );
    assert!(
        !combined.contains("gate-api"),
        "`gate-api` is a satellite of the `gate` unit and must not appear as its \
         own auto-discovered unit; got:\n{combined}"
    );
}

/// A change inside a satellite must bump the unit that owns the satellite.
///
/// `satellites` is documented as "paths whose commits attribute to this unit
/// but receive no version writes" — the whole point of the hexagonal service
/// shape, where the released `crates/bin` is a thin wrapper and essentially all
/// real work lands in `crates/api`, `crates/core`, `crates/infrastructure`.
/// The paths reached the ownership map and the drift detector but never the
/// unit's path matcher, so those commits attributed to nothing.
#[test]
fn a_change_in_a_satellite_bumps_the_owning_unit() {
    let repo = TestRepo::new();
    scaffold_configured_globs(&repo);

    // Nothing under `crates/bin` changes — only the satellite crate.
    repo.write_file(
        "apps/services/gate/crates/api/src/routes.rs",
        "pub fn routes() {}\n",
    );
    repo.commit("feat(gate): add the routes module");

    let out = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    let names = released_names(&manifest_json(&repo));
    assert!(
        names.iter().any(|n| n == "gate"),
        "a commit touching only a satellite crate must bump the unit that declares \
         it; released: {names:?}\nstderr:\n{stderr}"
    );
}

/// `[cascade_inputs]` reaching the services *transitively* — the proto case.
/// The input names only the two generated-code crates; the services are
/// supposed to be pulled in through their cargo dependency on those crates.
/// That hop is exactly the one configured units were missing, so the input
/// fired and still nothing downstream moved.
#[test]
fn cascade_input_reaches_services_through_a_configured_library_unit() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\n  \"packages/grpc\",\n  \
         \"apps/services/gate/crates/bin\",\n]\n",
    );
    repo.write_file(
        "packages/grpc/Cargo.toml",
        "[package]\nname = \"clikd-grpc\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write_file("packages/grpc/src/lib.rs", "pub fn client() {}\n");
    repo.write_file(
        "apps/services/gate/crates/bin/Cargo.toml",
        "[package]\nname = \"gate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\nclikd-grpc = { path = \"../../../../../packages/grpc\" }\n",
    );
    repo.write_file(
        "apps/services/gate/crates/bin/src/main.rs",
        "fn main() { clikd_grpc::client(); }\n",
    );
    repo.write_file("proto/gate/v1/gate.proto", "syntax = \"proto3\";\n");

    repo.write_file(
        "belaf/config.toml",
        "[release_unit.packages]\n\
         kind = \"internal\"\n\
         ecosystem = \"cargo\"\n\
         glob = \"packages/*\"\n\
         name = \"{basename}\"\n\
         manifests = [\"{path}/Cargo.toml\"]\n\
         \n\
         [release_unit.services]\n\
         ecosystem = \"cargo\"\n\
         glob = \"apps/services/*\"\n\
         name = \"{basename}\"\n\
         manifests = [\"{path}/crates/bin/Cargo.toml\"]\n\
         satellites = [\"{path}/crates\"]\n\
         \n\
         [cascade_inputs.proto]\n\
         paths = [\"proto/**\"]\n\
         affects = [\"grpc\"]\n",
    );

    repo.commit("Initial commit");
    generate_lockfile(&repo);
    repo.commit("chore: lockfile");
    repo.tag("gate-v0.1.0");

    // Only the proto changes. `grpc` is the declared target of the input;
    // `gate` has to come along through the cargo edge onto `grpc`.
    repo.write_file(
        "proto/gate/v1/gate.proto",
        "syntax = \"proto3\";\nmessage Ping { string id = 1; }\n",
    );
    repo.commit("fix(proto): add the Ping message");

    let out = repo.run_belaf_command_with_env(&["prepare", "--ci"], &[("BELAF_NO_KEYRING", "1")]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    let names = released_names(&manifest_json(&repo));
    assert!(
        names.iter().any(|n| n == "gate"),
        "a proto change must reach the service transitively through the configured \
         `grpc` library unit; released: {names:?}\nstderr:\n{stderr}"
    );
}
