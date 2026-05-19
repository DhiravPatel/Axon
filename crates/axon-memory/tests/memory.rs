//! Stage 11 — `axon-memory` tests.

use std::path::PathBuf;

use axon_memory::{EphemeralStore, Entry, FileStore, MemoryError, Store, SCHEMA_VERSION};

fn temp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-memory-{name}-{pid}-{ts}.json"));
    let _ = std::fs::remove_file(&p);
    p
}

// ---------- ephemeral --------------------------------------------------

#[test]
fn ephemeral_crud_round_trip() {
    let s = EphemeralStore::new();
    s.set("a", Entry::new(serde_json::json!(1))).unwrap();
    s.set("b", Entry::new(serde_json::json!("two"))).unwrap();
    assert_eq!(s.len().unwrap(), 2);
    assert_eq!(s.get("a").unwrap().unwrap().value, serde_json::json!(1));
    assert!(s.remove("a").unwrap());
    assert!(!s.remove("a").unwrap());
    assert_eq!(s.keys().unwrap(), vec!["b".to_string()]);
}

#[test]
fn ephemeral_forget_by_tag() {
    let s = EphemeralStore::new();
    s.set("a", Entry::tagged(serde_json::json!(1), "scratch"))
        .unwrap();
    s.set("b", Entry::tagged(serde_json::json!(2), "important"))
        .unwrap();
    s.set("c", Entry::new(serde_json::json!(3))).unwrap();
    let dropped = s.forget_tagged("scratch").unwrap();
    assert_eq!(dropped, 1);
    assert_eq!(s.len().unwrap(), 2);
    assert!(s.get("a").unwrap().is_none());
}

#[test]
fn ephemeral_forget_older_than_preserves_untagged_time() {
    let s = EphemeralStore::new();
    s.set(
        "old",
        Entry {
            value: serde_json::json!("o"),
            tag: None,
            written_at_ns: Some(100),
        },
    )
    .unwrap();
    s.set(
        "new",
        Entry {
            value: serde_json::json!("n"),
            tag: None,
            written_at_ns: Some(900),
        },
    )
    .unwrap();
    s.set("notimed", Entry::new(serde_json::json!("?"))).unwrap();
    let dropped = s.forget_older_than(500).unwrap();
    assert_eq!(dropped, 1);
    assert!(s.get("old").unwrap().is_none());
    assert!(s.get("new").unwrap().is_some());
    assert!(s.get("notimed").unwrap().is_some(), "no timestamp ⇒ kept");
}

// ---------- file backend ----------------------------------------------

#[test]
fn file_store_persists_through_reopen() {
    let path = temp_path("persist");
    {
        let s = FileStore::open(&path).unwrap();
        s.set("greeting", Entry::new(serde_json::json!("hi"))).unwrap();
        s.set("count", Entry::new(serde_json::json!(7))).unwrap();
    }
    let s = FileStore::open(&path).unwrap();
    assert_eq!(s.len().unwrap(), 2);
    assert_eq!(
        s.get("greeting").unwrap().unwrap().value,
        serde_json::json!("hi")
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn file_store_atomic_write_leaves_no_temp() {
    let path = temp_path("atomic");
    let s = FileStore::open(&path).unwrap();
    s.set("k", Entry::new(serde_json::json!("v"))).unwrap();

    // No leftover `*.tmp.*` files next to the target path.
    let parent = path.parent().unwrap();
    let stem = path.file_stem().unwrap().to_string_lossy().to_string();
    let strays: Vec<_> = std::fs::read_dir(parent)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with(&stem) && name.contains(".tmp.")
        })
        .collect();
    assert!(
        strays.is_empty(),
        "found stray temp files: {strays:?}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn file_store_does_not_create_file_until_first_write() {
    let path = temp_path("lazy");
    let _ = FileStore::open(&path).unwrap();
    assert!(
        !path.exists(),
        "opening a fresh store must not touch the disk"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn file_store_refuses_unknown_schema_version() {
    let path = temp_path("schema");
    std::fs::write(
        &path,
        format!(
            r#"{{"version": {bad}, "entries": {{}} }}"#,
            bad = SCHEMA_VERSION + 99
        ),
    )
    .unwrap();
    let err = FileStore::open(&path).unwrap_err();
    assert!(matches!(err, MemoryError::SchemaMismatch { .. }));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn file_store_remove_persists() {
    let path = temp_path("remove");
    {
        let s = FileStore::open(&path).unwrap();
        s.set("a", Entry::new(serde_json::json!(1))).unwrap();
        s.set("b", Entry::new(serde_json::json!(2))).unwrap();
        assert!(s.remove("a").unwrap());
    }
    let s = FileStore::open(&path).unwrap();
    assert!(s.get("a").unwrap().is_none());
    assert_eq!(s.get("b").unwrap().unwrap().value, serde_json::json!(2));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn snapshot_is_deterministic() {
    let s = EphemeralStore::new();
    s.set("b", Entry::new(serde_json::json!(2))).unwrap();
    s.set("a", Entry::new(serde_json::json!(1))).unwrap();
    s.set("c", Entry::new(serde_json::json!(3))).unwrap();
    let snap = s.snapshot().unwrap();
    let json = serde_json::to_string(&snap).unwrap();
    // BTreeMap means keys are sorted; assert the order we expect.
    let a_idx = json.find("\"a\"").unwrap();
    let b_idx = json.find("\"b\"").unwrap();
    let c_idx = json.find("\"c\"").unwrap();
    assert!(a_idx < b_idx && b_idx < c_idx, "got: {json}");
}
