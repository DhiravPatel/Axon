//! Stage 43 — functional collection methods on `List<T>`:
//! `any`/`all`/`find`/`count` (predicate), `take`/`drop`, `min`/`max`,
//! and `enumerate` (for `for (i, x) in xs.enumerate()`).

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
    p.push(format!("axon-stage43-{name}-{}-{ts}", std::process::id()));
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
fn predicate_methods() {
    build_axon();
    let dir = temp_dir("pred");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let xs = [3, 1, 4, 1, 5, 9, 2, 6]
    print("{xs.any(|x| x > 8)} {xs.all(|x| x > 0)} {xs.count(|x| x % 2 == 0)}")
    print("{xs.find(|x| x > 4).unwrap()}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["true true 3", "5"], "got: {:?}", lines(&out));
}

#[test]
fn take_drop_min_max() {
    build_axon();
    let dir = temp_dir("slice");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let xs = [3, 1, 4, 1, 5]
    print("{list_len(xs.take(2))} {list_get(xs.drop(3), 0)}")
    print("{xs.min().unwrap()} {xs.max().unwrap()}")
    let empty: List<Int> = []
    print("{empty.min().is_nil()}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["2 1", "1 5", "true"], "got: {:?}", lines(&out));
}

#[test]
fn enumerate_with_tuple_destructuring() {
    build_axon();
    let dir = temp_dir("enum");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    var out = ""
    for (i, v) in ["a", "b", "c"].enumerate() {
        out = out + "{i}{v}"
    }
    print("{out}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["0a1b2c"], "got: {:?}", lines(&out));
}
