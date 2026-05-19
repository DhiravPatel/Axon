//! Stage 11 — `mem_*` bindings exercised through the binary.
//!
//! Runs `axon run` against generated `.ax` programs and checks both the
//! exit code and on-disk side effects (the file-backed memory store).

use std::path::PathBuf;
use std::process::Command;

fn axon_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // workspace root
    p.push("target");
    p.push("debug");
    p.push("axon");
    p
}

fn temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-cli-test-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn build_axon() {
    let status = Command::new("cargo")
        .args(["build", "-q", "--bin", "axon"])
        .current_dir(workspace_root())
        .status()
        .expect("cargo build axon");
    assert!(status.success(), "build failed");
}

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // workspace root
    p
}

#[test]
fn mem_persists_across_runs_via_file_store() {
    build_axon();
    let dir = temp_dir("mem_persist");
    let mem_path = dir.join("mem.json").display().to_string();

    // First program: write a key.
    let src1 = dir.join("write.ax");
    std::fs::write(
        &src1,
        format!(
            "fn main() uses {{ Console }} {{\n    mem_open_file(\"{mem_path}\")\n    mem_set(\"counter\", 42)\n    print_int(mem_len())\n}}\n"
        ),
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args(["run", src1.to_str().unwrap()])
        .output()
        .expect("axon run write");
    assert!(out.status.success(), "first run: {:?}", out);

    // The file should now exist on disk.
    assert!(
        std::path::Path::new(&mem_path).exists(),
        "mem file should exist after write"
    );

    // Second program: read it back.
    let src2 = dir.join("read.ax");
    std::fs::write(
        &src2,
        format!(
            "fn main() uses {{ Console }} {{\n    mem_open_file(\"{mem_path}\")\n    print_int(mem_get(\"counter\"))\n}}\n"
        ),
    )
    .unwrap();
    let out2 = Command::new(axon_bin())
        .args(["run", src2.to_str().unwrap()])
        .output()
        .expect("axon run read");
    assert!(out2.status.success(), "second run: {:?}", out2);
    let stdout = String::from_utf8_lossy(&out2.stdout);
    // axon run prints the main return value at the end.
    assert!(
        stdout.contains("42"),
        "expected 42 in output, got: {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mem_remove_clears_key() {
    build_axon();
    let dir = temp_dir("mem_remove");
    let mem_path = dir.join("mem.json").display().to_string();

    let src = dir.join("p.ax");
    std::fs::write(
        &src,
        format!(
            "fn main() uses {{ Console }} {{\n    \
             mem_open_file(\"{mem_path}\")\n    \
             mem_set(\"k\", 99)\n    \
             mem_remove(\"k\")\n    \
             print_int(mem_len())\n\
             }}\n"
        ),
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .expect("axon run");
    assert!(out.status.success(), "run: {:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("0"), "expected len 0, got: {stdout:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
