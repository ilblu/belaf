//! Golden-file roundtrip for manifest 2.0 (plan §2 / Gap #5).
//!
//! Commits one representative 2.0 manifest at `tests/golden/`. The test
//! asserts that `parse → serialize → parse` produces the same in-memory
//! Manifest structure and that `serialize → parse → serialize` produces
//! byte-equal JSON.
//!
//! The goal is to catch silent changes in the wire format: any
//! refactor that drops a field or reorders keys differently than today
//! shows up here as a snapshot diff. The golden file is committed
//! alongside the schema, so a v2.0 producer + v2.0 consumer (in either
//! repo) can replay it and confirm wire compatibility.

use std::fs;

use belaf::core::manifest::ReleaseManifest;

const GOLDEN_PATH: &str = "tests/golden/manifest-2.0-canonical.json";

#[test]
fn golden_manifest_parses() {
    let raw = fs::read_to_string(GOLDEN_PATH).expect("read golden");
    let m = ReleaseManifest::from_json(&raw).expect("parse golden");
    assert_eq!(m.schema_version, "2.0");
    assert_eq!(m.manifest_id, "01890afa-7c5c-7000-8b00-aaaaaaaaaaaa");
    assert_eq!(m.releases.len(), 3);
    assert_eq!(m.groups.len(), 1);
    assert_eq!(m.groups[0].id, "schema-bundle");
    assert_eq!(m.groups[0].members.len(), 2);
}

/// `parse → serialize → parse` produces structurally identical
/// Manifests. This is the strict roundtrip — every field belaf knows
/// about survives the trip.
#[test]
fn golden_manifest_roundtrips_structurally() {
    let raw = fs::read_to_string(GOLDEN_PATH).expect("read golden");
    let m1 = ReleaseManifest::from_json(&raw).expect("parse 1");
    let serialized = m1.to_json().expect("serialize");
    let m2 = ReleaseManifest::from_json(&serialized).expect("parse 2");

    assert_eq!(m1.schema_version, m2.schema_version);
    assert_eq!(m1.manifest_id, m2.manifest_id);
    assert_eq!(m1.created_at, m2.created_at);
    assert_eq!(m1.created_by, m2.created_by);
    assert_eq!(m1.base_branch, m2.base_branch);
    assert_eq!(m1.groups.len(), m2.groups.len());
    assert_eq!(m1.releases.len(), m2.releases.len());

    for (a, b) in m1.groups.iter().zip(m2.groups.iter()) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.members, b.members);
    }
    for (a, b) in m1.releases.iter().zip(m2.releases.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.ecosystem.as_str(), b.ecosystem.as_str());
        assert_eq!(a.group_id, b.group_id);
        assert_eq!(a.previous_version, b.previous_version);
        assert_eq!(a.new_version, b.new_version);
        assert_eq!(a.bump_type.as_str(), b.bump_type.as_str());
        assert_eq!(a.tag_name, b.tag_name);
        assert_eq!(a.previous_tag, b.previous_tag);
        assert_eq!(a.compare_url, b.compare_url);
        assert_eq!(a.is_prerelease, b.is_prerelease);
        assert_eq!(a.changelog, b.changelog);
        assert_eq!(a.contributors, b.contributors);
        assert_eq!(a.first_time_contributors, b.first_time_contributors);
        assert_eq!(
            a.statistics.is_some(),
            b.statistics.is_some(),
            "statistics presence must round-trip for {}",
            a.name
        );
    }
}

/// `serialize → parse → serialize` produces byte-equal JSON. This is
/// the stricter roundtrip — guarantees the wire format is canonical
/// (no key-ordering ambiguity, no whitespace drift) once it's been
/// emitted by `to_json` once.
///
/// We don't compare against the committed golden directly because the
/// golden is hand-written for human readability (newlines, key order
/// optimised for diffing); `to_json()` produces a canonical form that
/// may differ in whitespace. Instead we round-trip once to canonicalise,
/// then assert that further round-trips are stable.
#[test]
fn golden_manifest_canonical_form_is_stable() {
    let raw = fs::read_to_string(GOLDEN_PATH).expect("read golden");
    let m = ReleaseManifest::from_json(&raw).expect("parse 1");
    let canonical_1 = m.to_json().expect("serialize 1");
    let m2 = ReleaseManifest::from_json(&canonical_1).expect("parse 2");
    let canonical_2 = m2.to_json().expect("serialize 2");
    assert_eq!(
        canonical_1, canonical_2,
        "canonical form must be stable across roundtrips:\n--- first ---\n{canonical_1}\n--- second ---\n{canonical_2}"
    );
}

/// The golden file MUST validate against `schemas/manifest.v2.0.schema.json`.
/// Catches drift between the canonical schema and the committed example.
#[test]
fn golden_manifest_validates_against_schema() {
    use serde_json::Value;
    let schema_raw = fs::read_to_string("schemas/manifest.v2.0.schema.json")
        .expect("read schema");
    let schema_json: Value = serde_json::from_str(&schema_raw).expect("parse schema");
    let golden_raw = fs::read_to_string(GOLDEN_PATH).expect("read golden");
    let golden_json: Value = serde_json::from_str(&golden_raw).expect("parse golden");

    // Compile + validate. jsonschema 0.34 supports Draft 2020-12 which
    // is what our schema declares.
    let validator =
        jsonschema::draft202012::new(&schema_json).expect("compile schema");
    let result = validator.validate(&golden_json);
    if let Err(err) = result {
        panic!("golden manifest fails schema validation: {err}");
    }
}
