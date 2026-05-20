//! Stage 26 — features (§7.1), MCP (§25.5), deterministic helpers
//! (§39.2) end-to-end.

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
    let status = Command::new("cargo")
        .args(["build", "-q", "--bin", "axon"])
        .current_dir(workspace_root())
        .status()
        .expect("cargo build axon");
    assert!(status.success(), "build failed");
}

fn temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-stage26-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn run_in(dir: &std::path::Path, src: &str) -> std::process::Output {
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run")
}

fn run_in_project(
    dir: &std::path::Path,
    src: &str,
    extra_args: &[&str],
) -> std::process::Output {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src").join("main.ax"), src).unwrap();
    let mut args: Vec<&str> = vec!["run"];
    args.extend_from_slice(extra_args);
    args.push(dir.to_str().unwrap());
    Command::new(axon_bin())
        .args(&args)
        .output()
        .expect("axon run project")
}

// ---------- §39.2 deterministic helpers ----------

#[test]
fn clock_freeze_locks_time_now() {
    build_axon();
    let dir = temp_dir("freeze");
    let prog = r#"
fn main() uses { Console, Time } {
    clock_freeze(1700000000000000000)
    let a = time_now()
    let b = time_now()
    print(a == b)
    clock_unfreeze()
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.lines().next().unwrap_or(""), "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rand_seed_makes_random_int_deterministic() {
    build_axon();
    let dir = temp_dir("rand_seed");
    let prog = r#"
fn main() uses { Console, Random } {
    rand_seed(42)
    let a1 = random_int(0, 1000000)
    let a2 = random_int(0, 1000000)
    rand_seed(42)
    let b1 = random_int(0, 1000000)
    let b2 = random_int(0, 1000000)
    print(a1 == b1)
    print(a2 == b2)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §25.5 MCP ----------

#[test]
fn mcp_loads_inline_tools_from_manifest() {
    build_axon();
    let dir = temp_dir("mcp_inline");
    let toml = r#"
[package]
name = "demo"
version = "0.1.0"

[tools.calculator]
tools = [
    { name = "add", description = "Sum two numbers", input_schema = "schema-a" },
    { name = "sub", description = "Subtract two numbers", input_schema = "schema-b" },
]

[tools.github]
mcp = "https://example.com/mcp"
"#;
    let toml_path = dir.join("axon.toml");
    std::fs::write(&toml_path, toml).unwrap();
    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    let loaded = mcp_load_from_toml("{}")
    print(loaded)
    let tools = mcp_list_tools("calculator")
    print(list_len(tools))
    print(list_get(tools, 0).name)
    let r = mcp_call_tool("calculator", "add", "args-here")
    print(bool(r.ok))
    print(str_contains(r.body, "args-here"))
    let deferred = mcp_deferred_namespaces()
    print(list_len(deferred))
    print(list_get(deferred, 0))
}}
"#,
        toml_path.display()
    );
    let out = run_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "2");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "add");
    assert_eq!(lines[3], "true");
    assert_eq!(lines[4], "true");
    assert_eq!(lines[5], "1");
    assert_eq!(lines[6], "github");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_call_unknown_tool_returns_ok_false() {
    build_axon();
    let dir = temp_dir("mcp_unknown");
    let toml = r#"
[tools.calc]
tools = [{ name = "add", description = "Sum", input_schema = "" }]
"#;
    let toml_path = dir.join("axon.toml");
    std::fs::write(&toml_path, toml).unwrap();
    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    mcp_load_from_toml("{}")
    let r = mcp_call_tool("calc", "ghost", "args")
    print(bool(r.ok))
    print(str_contains(r.error, "no tool"))
}}
"#,
        toml_path.display()
    );
    let out = run_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "false");
    assert_eq!(lines[1], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §7.1 features ----------

#[test]
fn features_active_returns_resolved_default() {
    build_axon();
    let dir = temp_dir("features_default");
    let toml = r#"
[package]
name = "demo"
version = "0.1.0"

[features]
default = ["redis-cache"]
redis-cache = ["network"]
network = []
"#;
    std::fs::write(dir.join("axon.toml"), toml).unwrap();
    let prog = r#"
fn main() uses { Console } {
    let active = features_active()
    print(list_len(active))
    print(list_contains(active, "default"))
    print(list_contains(active, "redis-cache"))
    print(list_contains(active, "network"))
}
"#;
    let out = run_in_project(&dir, prog, &[]);
    assert!(out.status.success(), "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "3");
    assert_eq!(lines[1], "true");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_default_features_drops_default_set() {
    build_axon();
    let dir = temp_dir("features_no_default");
    let toml = r#"
[package]
name = "demo"
version = "0.1.0"

[features]
default = ["redis-cache"]
redis-cache = ["network"]
network = []
"#;
    std::fs::write(dir.join("axon.toml"), toml).unwrap();
    let prog = r#"
fn main() uses { Console } {
    let active = features_active()
    print(list_len(active))
}
"#;
    let out = run_in_project(&dir, prog, &["--no-default-features"]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.lines().next().unwrap_or(""), "0");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cfg_feature_attribute_strips_inactive_items() {
    build_axon();
    let dir = temp_dir("cfg_strip");
    let toml = r#"
[package]
name = "demo"
version = "0.1.0"

[features]
redis = []
"#;
    std::fs::write(dir.join("axon.toml"), toml).unwrap();
    // A program that references `gated_helper` is only valid when the
    // `redis` feature is on. Without it, the gated_helper item is
    // stripped and the call site fails to resolve.
    let prog = r#"
#[cfg(feature = "redis")]
fn gated_helper() -> Int { 99 }

fn main() uses { Console } {
    print(features_active())
}
"#;
    // Without the feature, the program still parses + type-checks since
    // main doesn't reference gated_helper directly.
    let out = run_in_project(&dir, prog, &[]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[]"), "stdout: {stdout}");

    // With --features redis, the gated helper survives. Replace main to
    // *call* the gated helper and confirm it's now in scope.
    let prog2 = r#"
#[cfg(feature = "redis")]
fn gated_helper() -> Int { 99 }

fn main() uses { Console } {
    print(gated_helper())
}
"#;
    let out2 = run_in_project(&dir, prog2, &["--features", "redis"]);
    assert!(out2.status.success(), "stderr: {}",
        String::from_utf8_lossy(&out2.stderr));
    let stdout = String::from_utf8_lossy(&out2.stdout);
    assert!(stdout.contains("99"), "stdout: {stdout}");

    // Same source, but without --features, must fail (gated_helper
    // stripped → unresolved name).
    let out3 = run_in_project(&dir, prog2, &["--no-default-features"]);
    assert!(!out3.status.success(), "should fail without feature");
    let _ = std::fs::remove_dir_all(&dir);
}
