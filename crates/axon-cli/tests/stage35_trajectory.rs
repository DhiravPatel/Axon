//! Stage 35.4 — `axon test --record-trajectory` / `--match-trajectory`.

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
    p.push(format!("axon-stage35traj-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn project(dir: &std::path::Path, main_src: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("axon.toml"),
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\n[run]\nentry = \"main\"\nsrc = \"src\"\n[caps]\ndefault = [\"Console\", \"LLM\", \"Net\", \"Memory\", \"Tool\"]\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/main.ax"), main_src).unwrap();
}

#[test]
fn record_then_match_round_trips_for_a_simple_test() {
    build_axon();
    let dir = temp_dir("record_match");
    project(
        &dir,
        "fn ask_it() -> String uses { LLM, Net } {\n\
         \x20   let m = mock_model(\"fixed\", \"hi\")\n\
         \x20   ask m { user: \"q\" }\n\
         }\n\
         fn main() uses { Console } {}\n\
         test \"call a model\" { let _ = ask_it() }\n",
    );
    let record = Command::new(axon_bin())
        .args([
            "test",
            "--record-trajectory",
            "baseline",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("axon test --record-trajectory");
    assert!(record.status.success(), "{:?}", record);
    let stdout = String::from_utf8_lossy(&record.stdout);
    assert!(stdout.contains("trajectory recorded"), "{stdout}");
    assert!(
        dir.join("tests/.trajectories/baseline").is_dir(),
        "expected trajectory dir"
    );

    let matched = Command::new(axon_bin())
        .args([
            "test",
            "--match-trajectory",
            "baseline",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("axon test --match-trajectory");
    assert!(matched.status.success(), "{:?}", matched);
    let stdout = String::from_utf8_lossy(&matched.stdout);
    assert!(stdout.contains("matched"), "{stdout}");
    assert!(stdout.contains("0 drifted"), "{stdout}");
}

#[test]
fn match_without_saved_snapshot_reports_clearly() {
    build_axon();
    let dir = temp_dir("no_baseline");
    project(
        &dir,
        "fn ask_it() -> String uses { LLM, Net } {\n\
         \x20   let m = mock_model(\"fixed\", \"hi\")\n\
         \x20   ask m { user: \"q\" }\n\
         }\n\
         fn main() uses { Console } {}\n\
         test \"call a model\" { let _ = ask_it() }\n",
    );
    let out = Command::new(axon_bin())
        .args([
            "test",
            "--match-trajectory",
            "no_such_snapshot",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("axon test");
    // Tests pass on their own (no model call drift to check); the
    // missing-snapshot message appears per-test.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("no saved trajectory"),
        "expected 'no saved trajectory' message: {stdout}"
    );
}

#[test]
fn record_and_match_are_mutually_exclusive() {
    build_axon();
    let dir = temp_dir("mutex_traj");
    project(&dir, "fn main() {}\n");
    let out = Command::new(axon_bin())
        .args([
            "test",
            "--record-trajectory",
            "a",
            "--match-trajectory",
            "a",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("axon test");
    assert!(!out.status.success(), "expected refusal: {:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "expected mutex error: {stderr}"
    );
}

#[test]
fn tests_without_model_calls_are_skipped_in_trajectory_mode() {
    // No model calls → no ModelCall events → no trajectory captured.
    // The test still passes; we just don't write a snapshot.
    build_axon();
    let dir = temp_dir("no_model");
    project(
        &dir,
        "fn main() uses { Console } {}\n\
         test \"pure test\" { assert(1 + 1 == 2) }\n",
    );
    let out = Command::new(axon_bin())
        .args([
            "test",
            "--record-trajectory",
            "baseline",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("axon test");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "{stdout}");
    assert!(
        stdout.contains("trajectories recorded: 0"),
        "should record 0 trajectories: {stdout}"
    );
}

#[test]
fn shape_drift_triggers_match_failure_and_nonzero_exit() {
    // Step-count drift: record a single-call trajectory, then change
    // the test to make 3 calls and assert match fails.
    build_axon();
    let dir = temp_dir("shape_drift");
    project(
        &dir,
        "fn one() -> String uses { LLM, Net } {\n\
         \x20   let m = mock_model(\"fixed\", \"a\")\n\
         \x20   ask m { user: \"q\" }\n\
         }\n\
         fn main() uses { Console } {}\n\
         test \"shaped\" { let _ = one() }\n",
    );
    let record = Command::new(axon_bin())
        .args([
            "test",
            "--record-trajectory",
            "baseline",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("axon test --record");
    assert!(record.status.success(), "{:?}", record);

    // Now blow up the step count: 3 ask calls instead of 1.
    std::fs::write(
        dir.join("src/main.ax"),
        "fn three() -> String uses { LLM, Net } {\n\
         \x20   let m = mock_model(\"fixed\", \"a\")\n\
         \x20   let _ = ask m { user: \"q1\" }\n\
         \x20   let _ = ask m { user: \"q2\" }\n\
         \x20   ask m { user: \"q3\" }\n\
         }\n\
         fn main() uses { Console } {}\n\
         test \"shaped\" { let _ = three() }\n",
    )
    .unwrap();
    let matched = Command::new(axon_bin())
        .args([
            "test",
            "--match-trajectory",
            "baseline",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("axon test --match");
    let stdout = String::from_utf8_lossy(&matched.stdout);
    assert!(
        stdout.contains("DRIFT"),
        "expected DRIFT line for step count: {stdout}"
    );
    assert!(
        stdout.contains("steps"),
        "expected `steps` metric in drift output: {stdout}"
    );
    // Exit non-zero on drift so CI catches it.
    assert!(!matched.status.success(), "drift should exit non-zero");
}
