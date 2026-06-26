//! Stage 40 — fundamental language constructs that were missing:
//! `loop { }` (with `break <value>`), and the bitwise/shift compound
//! assignments `&=` `|=` `^=` `<<=` `>>=`.

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
    p.push(format!("axon-stage40-{name}-{}-{ts}", std::process::id()));
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
fn loop_with_break_value_is_an_expression() {
    build_axon();
    let dir = temp_dir("loopval");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    var i = 0
    let r = loop {
        i += 1
        if i > 5 && i % 2 == 0 { break i }
    }
    print("{r}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["6"], "got: {:?}", lines(&out));
}

#[test]
fn plain_loop_break_and_continue() {
    build_axon();
    let dir = temp_dir("loopctl");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    var n = 0
    var seen = 0
    loop {
        n += 1
        if n % 2 == 0 { continue }
        seen += 1
        if n >= 7 { break }
    }
    print("{n} {seen}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    // n counts 1..7; odd values 1,3,5,7 -> seen = 4.
    assert_eq!(lines(&out), ["7 4"], "got: {:?}", lines(&out));
}

#[test]
fn bitwise_and_shift_compound_assignment() {
    build_axon();
    let dir = temp_dir("bitassign");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    var f = 0
    f |= 5
    f &= 6
    f ^= 1
    var s = 1
    s <<= 4
    s >>= 1
    print("{f} {s}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    // f: 0|5=5, 5&6=4, 4^1=5. s: 1<<4=16, 16>>1=8.
    assert_eq!(lines(&out), ["5 8"], "got: {:?}", lines(&out));
}
