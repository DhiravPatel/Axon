//! Stage 14 — `axon-skill` tests.

use std::collections::BTreeMap;
use std::path::PathBuf;

use axon_skill::{Manifest, Skill, SkillError};

fn temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-skill-test-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn fixture_manifest() -> Manifest {
    Manifest {
        name: "demo".into(),
        version: "0.1.0".into(),
        description: "Demo skill".into(),
        entrypoint: "src/main.ax".into(),
        capabilities: vec!["Console".into()],
        dependencies: vec![],
        authors: vec!["test@example.com".into()],
    }
}

fn fixture_files() -> BTreeMap<String, String> {
    let mut f = BTreeMap::new();
    f.insert(
        "src/main.ax".to_string(),
        "fn main() -> Int { 42 }\n".to_string(),
    );
    f.insert(
        "src/util.ax".to_string(),
        "fn helper(x: Int) -> Int { x + 1 }\n".to_string(),
    );
    f
}

#[test]
fn new_skill_has_consistent_hash() {
    let s = Skill::new(fixture_manifest(), fixture_files());
    assert_eq!(s.format_version, axon_skill::FORMAT_VERSION);
    assert_eq!(s.content_hash, Skill::compute_hash(&s.files));
    s.verify().unwrap();
}

#[test]
fn same_files_produce_same_hash() {
    let s1 = Skill::new(fixture_manifest(), fixture_files());
    let s2 = Skill::new(fixture_manifest(), fixture_files());
    assert_eq!(s1.content_hash, s2.content_hash);
}

#[test]
fn changing_a_file_changes_the_hash() {
    let s1 = Skill::new(fixture_manifest(), fixture_files());
    let mut files = fixture_files();
    files.insert(
        "src/main.ax".into(),
        "fn main() -> Int { 99 }\n".to_string(),
    );
    let s2 = Skill::new(fixture_manifest(), files);
    assert_ne!(s1.content_hash, s2.content_hash);
}

#[test]
fn json_round_trip() {
    let s = Skill::new(fixture_manifest(), fixture_files());
    let bytes = s.to_json().unwrap();
    let back = Skill::from_json(&bytes).unwrap();
    assert_eq!(back, s);
}

#[test]
fn tampered_json_is_rejected() {
    let s = Skill::new(fixture_manifest(), fixture_files());
    let mut v: serde_json::Value = serde_json::from_slice(&s.to_json().unwrap()).unwrap();
    // Tamper with a file body but leave content_hash unchanged.
    v["files"]["src/main.ax"] = serde_json::Value::String("fn main() -> Int { 0 }".into());
    let tampered = serde_json::to_vec(&v).unwrap();
    let err = Skill::from_json(&tampered).unwrap_err();
    assert!(matches!(err, SkillError::HashMismatch { .. }));
}

#[test]
fn unsupported_format_version_is_rejected() {
    let s = Skill::new(fixture_manifest(), fixture_files());
    let mut v: serde_json::Value = serde_json::from_slice(&s.to_json().unwrap()).unwrap();
    v["format_version"] = serde_json::Value::Number(999.into());
    let bytes = serde_json::to_vec(&v).unwrap();
    let err = Skill::from_json(&bytes).unwrap_err();
    assert!(matches!(err, SkillError::UnsupportedVersion { .. }));
}

#[test]
fn entrypoint_missing_from_files_is_rejected() {
    let mut m = fixture_manifest();
    m.entrypoint = "src/not_there.ax".into();
    let s = Skill::new(m, fixture_files());
    assert!(matches!(s.verify().unwrap_err(), SkillError::Invalid(_)));
}

#[test]
fn pack_and_unpack_round_trip_through_filesystem() {
    let src = temp_dir("packsrc");
    let dst = temp_dir("packdst");

    std::fs::write(
        src.join("manifest.json"),
        serde_json::to_vec_pretty(&fixture_manifest()).unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(src.join("src")).unwrap();
    std::fs::write(src.join("src/main.ax"), "fn main() -> Int { 7 }\n").unwrap();
    std::fs::write(src.join("src/util.ax"), "fn h(x: Int) -> Int { x }\n").unwrap();

    let packed = Skill::pack(&src).unwrap();
    assert!(packed.files.contains_key("src/main.ax"));
    assert!(packed.files.contains_key("src/util.ax"));
    assert!(!packed.files.contains_key("manifest.json"));

    packed.unpack_to(&dst).unwrap();
    let main = std::fs::read_to_string(dst.join("src/main.ax")).unwrap();
    assert_eq!(main, "fn main() -> Int { 7 }\n");

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dst);
}
