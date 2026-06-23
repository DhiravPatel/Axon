//! Stage 39 (wave 2) — method-surface completeness + multi-line string
//! interpolation. Pins the numeric/List/String methods that previously only
//! existed as `math_*`/`list_*`/`str_*` free functions, plus `"""...{x}..."""`
//! interpolation.

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
    p.push(format!("axon-stage39m-{name}-{}-{ts}", std::process::id()));
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
fn numeric_methods() {
    build_axon();
    let dir = temp_dir("num");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let neg = 0 - 7
    print("{neg.abs()} {2.pow(10)} {3.min(9)} {3.max(9)}")
    let f = 3.7
    print("{f.round()} {f.floor()} {f.ceil()} {f.to_int()}")
    print("{42.to_string()} {f.to_string()}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        lines(&out),
        ["7 1024 3 9", "4 3 4 3", "42 3.7"],
        "got: {:?}",
        lines(&out)
    );
}

#[test]
fn list_methods() {
    build_axon();
    let dir = temp_dir("list");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let xs = [3, 1, 2]
    print("{xs.sum()} {xs.is_empty()} {xs.contains(2)} {xs.index_of(2)}")
    let s = xs.sort()
    print("{list_get(s, 0)}{list_get(s, 1)}{list_get(s, 2)}")
    let words = ["a", "b", "c"]
    let j = words.join("-")
    print("{j}")
    let total = xs.fold(100, |acc, x| acc + x)
    print("{total}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        lines(&out),
        ["6 false true 2", "123", "a-b-c", "106"],
        "got: {:?}",
        lines(&out)
    );
}

#[test]
fn string_methods() {
    build_axon();
    let dir = temp_dir("str");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let kv = "key=value".split_once("=")
    print("{list_get(kv, 0)} {list_get(kv, 1)}")
    let io = "hello".index_of("ll")
    let sub = "hello".substring(1, 4)
    let n = list_len("héllo".chars())
    print("{io} {sub} {n}")
    let nl = list_len("a\nb\nc".split_lines())
    print("{nl}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        lines(&out),
        ["key value", "2 ell 5", "3"],
        "got: {:?}",
        lines(&out)
    );
}

#[test]
fn multiline_string_interpolation() {
    build_axon();
    let dir = temp_dir("p3b");
    // `"""..."""` and `prompt"""..."""` interpolate `{expr}`; `{{`/`}}` escape.
    let out = run_src(
        &dir,
        "fn main() uses { Console } {\n\
         \x20   let name = \"Ada\"\n\
         \x20   let n = 2\n\
         \x20   let msg = \"\"\"\n\
         \x20       Hi {name}, you have {n} items. Braces {{x}}.\n\
         \x20   \"\"\"\n\
         \x20   print(msg.trim())\n\
         }\n",
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        lines(&out),
        ["Hi Ada, you have 2 items. Braces {x}."],
        "got: {:?}",
        lines(&out)
    );
}
