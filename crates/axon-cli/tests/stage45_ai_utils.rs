//! Stage 45 — pure AI utilities: estimate_tokens, chunk_text, extract_json,
//! extract_code, cosine_similarity.

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
    p.push(format!("axon-stage45-{name}-{}-{ts}", std::process::id()));
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
fn estimate_tokens_and_chunk_text() {
    build_axon();
    let dir = temp_dir("tok");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    print("{estimate_tokens("hello world")}")
    let cs = chunk_text("abcdefghij", 4, 1)
    print("{list_len(cs)} {list_get(cs, 0)} {list_get(cs, 2)}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    // 11 chars -> ceil(11/4)=3 tokens; chunks of 4 w/ overlap 1 -> abcd/defg/ghij.
    assert_eq!(lines(&out), ["3", "3 abcd ghij"], "got: {:?}", lines(&out));
}

#[test]
fn extract_json_from_model_reply() {
    build_axon();
    let dir = temp_dir("json");
    // The JSON braces are written `{{`/`}}` (literal braces in an interpolated
    // string) and quotes are escaped.
    let out = run_src(
        &dir,
        "fn main() uses { Console } {\n\
         \x20   let reply = \"Sure: {{\\\"n\\\": 30}} done\"\n\
         \x20   let j = extract_json(reply).unwrap_or(\"none\")\n\
         \x20   print(j)\n\
         }\n",
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["{\"n\": 30}"], "got: {:?}", lines(&out));
}

#[test]
fn extract_code_fenced_block() {
    build_axon();
    let dir = temp_dir("code");
    let out = run_src(
        &dir,
        "fn main() uses { Console } {\n\
         \x20   let reply = \"here:\\n```axon\\nlet x = 1\\n```\\nbye\"\n\
         \x20   let c = extract_code(reply).unwrap_or(\"none\")\n\
         \x20   print(c)\n\
         }\n",
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["let x = 1"], "got: {:?}", lines(&out));
}

#[test]
fn cosine_similarity_basics() {
    build_axon();
    let dir = temp_dir("cos");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let a = [1.0, 0.0]
    let same = cosine_similarity(a, [1.0, 0.0])
    let orth = cosine_similarity(a, [0.0, 1.0])
    print("{same} {orth}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["1 0"], "got: {:?}", lines(&out));
}
