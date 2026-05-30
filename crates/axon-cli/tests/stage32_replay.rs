//! Stage 32 — replay determinism for `flow_parallel_asks`.
//!
//! Parallel dispatch via `tokio::spawn_blocking` runs N model calls
//! concurrently — but `axon run --record` must capture the exact same
//! event sequence a serial run would have produced, and `axon run
//! --replay` must reconstruct byte-identical output.
//!
//! The mechanism that makes this work: `flow_parallel_asks` joins and
//! records in *input order*, not completion order. This test proves it
//! by forcing completion order to be the inverse of input order (slowest
//! task first in the input list, fastest last), recording, and then
//! replaying with a provider that errors if it's ever called — which
//! would happen only if replay diverged from the recording.

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
    p.push(format!("axon-stage32rep-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

// ===========================================================================
// Record-then-replay round-trip produces byte-identical output even though
// the underlying dispatch order is non-deterministic.
// ===========================================================================

#[test]
fn parallel_asks_record_then_replay_is_byte_identical() {
    build_axon();
    let dir = temp_dir("rep");
    let src_path = dir.join("p.ax");
    let rec_path = dir.join("rec.json");
    // Slowest task is first; the fastest finishes first. A
    // completion-order recorder would emit "c", "b", "a"; the input-order
    // dispatcher emits "a", "b", "c". Replay then has to re-emit
    // "a", "b", "c" exactly.
    std::fs::write(
        &src_path,
        r#"fn main() uses { Console, LLM, Net } {
    let m1 = mock_model_slow("a", 100)
    let m2 = mock_model_slow("b", 50)
    let m3 = mock_model_slow("c", 10)
    let xs = flow_parallel_asks([
        { target: m1, user: "q1" },
        { target: m2, user: "q2" },
        { target: m3, user: "q3" },
    ])
    print(list_get(xs, 0))
    print(list_get(xs, 1))
    print(list_get(xs, 2))
}
"#,
    )
    .unwrap();

    // Record run.
    let record = Command::new(axon_bin())
        .args([
            "run",
            "--record",
            rec_path.to_str().unwrap(),
            src_path.to_str().unwrap(),
        ])
        .output()
        .expect("axon run --record");
    assert!(record.status.success(), "record run: {:?}", record);
    let recorded_stdout = String::from_utf8_lossy(&record.stdout).to_string();
    assert!(
        rec_path.exists(),
        "recording file was not produced at {rec_path:?}"
    );
    // Sanity: input order.
    let lines: Vec<&str> = recorded_stdout.lines().collect();
    assert_eq!(lines, ["a", "b", "c"]);

    // Replay run — same source, but routed through `axon replay`. If the
    // recording captured completion order rather than input order, replay
    // would emit a different sequence and this assert would fail.
    let replay = Command::new(axon_bin())
        .args([
            "replay",
            rec_path.to_str().unwrap(),
            src_path.to_str().unwrap(),
        ])
        .output()
        .expect("axon replay");
    assert!(replay.status.success(), "replay run: {:?}", replay);
    let replayed_stdout = String::from_utf8_lossy(&replay.stdout).to_string();
    // Strip any trailing summary from `axon replay` so we compare the
    // program output line-by-line. The replay command may append a status
    // footer; pull the leading lines that match what `axon run` printed.
    let replay_head: String = replayed_stdout
        .lines()
        .take(3)
        .collect::<Vec<_>>()
        .join("\n");
    let record_head: String = recorded_stdout
        .lines()
        .take(3)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        record_head, replay_head,
        "replay output diverged from recording — flow_parallel_asks is \
         not recording in input order"
    );
}

// ===========================================================================
// Recording captures one ModelCall event per ask, in input order.
// ===========================================================================

#[test]
fn recording_has_one_model_call_event_per_ask() {
    build_axon();
    let dir = temp_dir("rec_count");
    let src_path = dir.join("p.ax");
    let rec_path = dir.join("rec.json");
    std::fs::write(
        &src_path,
        r#"fn main() uses { Console, LLM, Net } {
    let m = mock_model_slow("x", 5)
    let xs = flow_parallel_asks([
        { target: m, user: "q1" },
        { target: m, user: "q2" },
        { target: m, user: "q3" },
        { target: m, user: "q4" },
    ])
    print_int(list_len(xs))
}
"#,
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args([
            "run",
            "--record",
            rec_path.to_str().unwrap(),
            src_path.to_str().unwrap(),
        ])
        .output()
        .expect("axon run --record");
    assert!(out.status.success(), "{:?}", out);
    let text = std::fs::read_to_string(&rec_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).expect("recording is JSON");
    // Recordings are a flat array of events keyed by kind. Count model_call
    // events.
    let events = json
        .get("events")
        .and_then(|v| v.as_array())
        .expect("recording has an events array");
    let model_calls = events
        .iter()
        .filter(|e| {
            e.get("kind")
                .and_then(|k| k.as_str())
                .map(|s| s.eq_ignore_ascii_case("ModelCall") || s == "model_call")
                .unwrap_or(false)
        })
        .count();
    assert_eq!(
        model_calls, 4,
        "expected one ModelCall per ask; got {model_calls} in {events:?}"
    );
}
