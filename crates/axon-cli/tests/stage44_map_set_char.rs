//! Stage 44 — Map/Set method completeness + Char text-processing methods.

use std::path::PathBuf;
use std::process::Command;

fn axon_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p.push("debug");
    p.push("axon");
    p
}

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn build_axon() {
    let st = Command::new("cargo")
        .args(["build", "-q", "--bin", "axon"])
        .current_dir(workspace_root())
        .status()
        .expect("cargo build");
    assert!(st.success(), "build failed");
}

fn temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-stage44-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn run_src(dir: &std::path::Path, src: &str) -> std::process::Output {
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run")
}

fn lines(out: &std::process::Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn map_methods() {
    build_axon();
    let dir = temp_dir("map");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    var m: Map<String, Int> = {}
    m.set("a", 1)
    m.set("b", 2)
    print("{m.len()} {list_len(m.keys())} {list_len(m.values())} {m.is_empty()}")
    m.remove("a")
    let has = m.contains("a")
    print("{has} {m.len()}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["2 2 2 false", "false 1"], "got: {:?}", lines(&out));
}

#[test]
fn set_methods() {
    build_axon();
    let dir = temp_dir("set");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let a: Set<Int> = {1, 2, 3}
    let b: Set<Int> = {2, 3, 4}
    print("{a.len()} {a.union(b).len()} {a.intersection(b).len()} {a.difference(b).len()}")
    print("{list_len(a.to_list())}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["3 4 2 1", "3"], "got: {:?}", lines(&out));
}

#[test]
fn char_text_processing() {
    build_axon();
    let dir = temp_dir("char");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    var digits = 0
    var alphas = 0
    var sum = 0
    for ch in "a1b2c3 " {
        if ch.is_digit() { digits = digits + 1; sum = sum + ch.to_digit().unwrap() }
        if ch.is_alpha() { alphas = alphas + 1 }
    }
    let up = 'a'.to_upper().to_string()
    print("{digits} {alphas} {sum} {up}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["3 3 6 A"], "got: {:?}", lines(&out));
}
