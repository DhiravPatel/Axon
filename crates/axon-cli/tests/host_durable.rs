//! Stage 14 — `trigger_*`, `skill_*`, `a2a_*` exercised through the binary.

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
    p.push(format!("axon-stage14-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn run_program_in(dir: &std::path::Path, src: &str) -> std::process::Output {
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run")
}

// ---------- triggers ----------

#[test]
fn trigger_every_fires_once_per_tick_window() {
    build_axon();
    let dir = temp_dir("trigger_every");
    // The handler is identified by name; trigger_tick() returns the list of
    // fired triggers so the test can count fires deterministically rather
    // than relying on shared mutable state (which Axon's parser doesn't
    // expose at the top level).
    let out = run_program_in(
        &dir,
        r#"
fn on_tick() uses { Console } { print("fire") }

fn main() uses { Console } {
    trigger_every("t1", "on_tick", 60)
    // First tick: due now (Every fires immediately first time).
    let f1 = trigger_tick(30000000000)
    print_int(list_len(f1))
    // Second tick at 50s: not due yet (last=30s, period=60s).
    let f2 = trigger_tick(50000000000)
    print_int(list_len(f2))
    // Third tick at 100s: due again.
    let f3 = trigger_tick(100000000000)
    print_int(list_len(f3))
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let fires: usize = stdout.matches("fire\n").count();
    assert_eq!(fires, 2, "handler should run twice across the 3 ticks");
    // First number after the "fire" lines: the tick return-counts in order
    // 1, 0, 1.
    let lines: Vec<&str> = stdout.lines().collect();
    let digits: Vec<&str> = lines.iter().copied().filter(|l| *l != "fire").collect();
    assert_eq!(digits, vec!["1", "0", "1"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trigger_at_fires_exactly_once() {
    build_axon();
    let dir = temp_dir("trigger_at");
    let out = run_program_in(
        &dir,
        r#"
fn alarm() uses { Console } { print("ring") }

fn main() uses { Console } {
    trigger_at("a1", "alarm", 100000000000)
    let f1 = trigger_tick(50000000000)
    print_int(list_len(f1))
    let f2 = trigger_tick(200000000000)
    print_int(list_len(f2))
    let f3 = trigger_tick(300000000000)
    print_int(list_len(f3))
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.matches("ring\n").count(), 1, "alarm fires exactly once");
    let digits: Vec<&str> = stdout.lines().filter(|l| *l != "ring").collect();
    assert_eq!(digits, vec!["0", "1", "0"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trigger_state_persists_across_processes() {
    build_axon();
    let dir = temp_dir("trigger_persist");
    let mem_path = dir.join("mem.json").display().to_string();

    // Process 1: define a trigger, tick once, save.
    let src1 = dir.join("p1.ax");
    std::fs::write(
        &src1,
        format!(
            r#"
fn on_tick() {{}}
fn main() uses {{ Console }} {{
    mem_open_file("{mem_path}")
    trigger_every("t", "on_tick", 60)
    trigger_tick(0)
    trigger_save()
    print_int(trigger_len())
}}
"#
        ),
    )
    .unwrap();
    let out1 = Command::new(axon_bin())
        .args(["run", src1.to_str().unwrap()])
        .output()
        .expect("axon run process 1");
    assert!(out1.status.success(), "process 1: {:?}", out1);
    assert!(String::from_utf8_lossy(&out1.stdout).contains("1"));

    // Process 2: load, observe last_fired survives, tick again (not due yet).
    let src2 = dir.join("p2.ax");
    std::fs::write(
        &src2,
        format!(
            r#"
fn on_tick() uses {{ Console }} {{ print("FIRE") }}
fn main() uses {{ Console }} {{
    mem_open_file("{mem_path}")
    trigger_load()
    print_int(trigger_len())
    // Last fired was at t=0; ticking at t=30s should NOT fire (period=60s).
    let fired = trigger_tick(30000000000)
    print_int(list_len(fired))
}}
"#
        ),
    )
    .unwrap();
    let out2 = Command::new(axon_bin())
        .args(["run", src2.to_str().unwrap()])
        .output()
        .expect("axon run process 2");
    assert!(out2.status.success(), "process 2: {:?}", out2);
    let s = String::from_utf8_lossy(&out2.stdout);
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines[0], "1", "trigger survived");
    assert_eq!(lines[1], "0", "did not re-fire (last_fired preserved)");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- skills ----------

#[test]
fn skill_pack_inspect_install_round_trip_through_cli() {
    build_axon();
    let dir = temp_dir("skill_rt");
    let src_dir = dir.join("src_skill");
    let dest_dir = dir.join("installed");
    let pkg_path = dir.join("demo.axskill");

    std::fs::create_dir_all(src_dir.join("src")).unwrap();
    std::fs::write(
        src_dir.join("manifest.json"),
        r#"{
            "name": "greeter",
            "version": "0.2.0",
            "description": "Says hi.",
            "entrypoint": "src/main.ax",
            "capabilities": ["Console"],
            "dependencies": [],
            "authors": []
        }"#,
    )
    .unwrap();
    std::fs::write(
        src_dir.join("src/main.ax"),
        "fn main() uses { Console } { print(\"hi\") }\n",
    )
    .unwrap();

    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    skill_pack("{src}", "{pkg}")
    let info = skill_inspect("{pkg}")
    print(info.name)
    print(info.version)
    print(info.entrypoint)
    print_int(info.file_count)

    let installed = skill_install("{pkg}", "{dest}")
    print(installed.content_hash)
}}
"#,
        src = src_dir.display(),
        pkg = pkg_path.display(),
        dest = dest_dir.display()
    );
    let out = run_program_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "greeter");
    assert_eq!(lines[1], "0.2.0");
    assert_eq!(lines[2], "src/main.ax");
    assert_eq!(lines[3], "1");
    assert!(lines[4].starts_with("h_"));

    let installed_main = std::fs::read_to_string(dest_dir.join("src/main.ax")).unwrap();
    assert!(installed_main.contains("print(\"hi\")"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- a2a ----------

#[test]
fn a2a_card_load_returns_capabilities() {
    build_axon();
    let dir = temp_dir("a2a_load");
    let card_path = dir.join("card.json");
    let card_json = r#"{
        "format_version": 1,
        "agent_id": "research-1",
        "name": "Research Agent",
        "version": "0.1.0",
        "description": "demo",
        "endpoint": "https://research.example.com/agent",
        "capabilities": [
            { "name": "Research", "input_schema_url": null, "output_schema_url": null, "description": "" },
            { "name": "Summarize", "input_schema_url": null, "output_schema_url": null, "description": "" }
        ],
        "auth": { "scheme": "bearer" },
        "pricing": null,
        "rate_limits": null,
        "metadata": {}
    }"#;
    std::fs::write(&card_path, card_json).unwrap();

    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    let card = a2a_card_load("{}")
    print(card.agent_id)
    print(card.name)
    print(card.endpoint)
    print_int(list_len(card.capabilities))
    print(list_get(card.capabilities, 0))
    print(list_get(card.capabilities, 1))

    let has_r = a2a_card_has_capability("{}", "Research")
    let has_x = a2a_card_has_capability("{}", "Unknown")
    print(bool(has_r))
    print(bool(has_x))
}}
"#,
        card_path.display(),
        card_path.display(),
        card_path.display()
    );
    let out = run_program_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "research-1");
    assert_eq!(lines[1], "Research Agent");
    assert_eq!(lines[2], "https://research.example.com/agent");
    assert_eq!(lines[3], "2");
    assert_eq!(lines[4], "Research");
    assert_eq!(lines[5], "Summarize");
    assert_eq!(lines[6], "true");
    assert_eq!(lines[7], "false");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a2a_card_load_rejects_invalid_card() {
    build_axon();
    let dir = temp_dir("a2a_bad");
    let card_path = dir.join("card.json");
    // Invalid: empty agent_id.
    std::fs::write(
        &card_path,
        r#"{
            "format_version": 1, "agent_id": "", "name": "x", "version": "1",
            "description": "", "endpoint": "https://x.example.com",
            "capabilities": [], "auth": { "scheme": "none" },
            "pricing": null, "rate_limits": null, "metadata": {}
        }"#,
    )
    .unwrap();
    let prog = format!(
        r#"
fn main() {{
    a2a_card_load("{}")
}}
"#,
        card_path.display()
    );
    let out = run_program_in(&dir, &prog);
    assert!(!out.status.success(), "should have failed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("invalid") || stderr.contains("empty"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
