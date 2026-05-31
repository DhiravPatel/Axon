//! Stage 36 — record-then-replay byte identity for `parallel { }`.
//!
//! The Stage 32 hook contract says: parallel dispatchers must record
//! responses in *input order* on the calling thread so replay reproduces
//! a serial run. This test pins the contract for the new `parallel { }`
//! surface syntax:
//!
//!   1. Record a 2-arm parallel run with asymmetric latencies — the
//!      faster arm completes first, but the recording must list the
//!      slower arm's response *first* (input order, not completion order).
//!   2. Replay against null providers — the replay path must consume the
//!      events in input order and never touch the tokio runtime.
//!   3. Both record and replay produce byte-identical stdout.

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
    p.push(format!("axon-stage36rep-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn parallel_record_then_replay_yields_byte_identical_stdout() {
    build_axon();
    let dir = temp_dir("ri");
    let src = dir.join("p.ax");
    let rec = dir.join("rec.json");
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let m_slow = mock_model_slow("slow_a", 80)
    let m_fast = mock_model_slow("fast_b", 10)
    let xs = parallel {
        ask m_slow { user: "q1" },
        ask m_fast { user: "q2" },
    }
    print(list_get(xs, 0))
    print(list_get(xs, 1))
}
"#;
    std::fs::write(&src, prog).unwrap();

    // Record run.
    let rec_out = Command::new(axon_bin())
        .args([
            "run",
            "--record",
            rec.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .expect("axon run --record");
    assert!(rec_out.status.success(), "{:?}", rec_out);
    let rec_stdout = rec_out.stdout.clone();
    let lines: Vec<&str> = std::str::from_utf8(&rec_stdout)
        .unwrap()
        .lines()
        .collect();
    // Input-order contract: slow arm declared first, slow result printed first
    // even though fast finished first.
    assert_eq!(
        lines,
        ["slow_a", "fast_b"],
        "input-order record-stdout violated; got: {lines:?}"
    );

    // Replay run.
    let replay_out = Command::new(axon_bin())
        .args([
            "replay",
            rec.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .expect("axon replay");
    assert!(
        replay_out.status.success(),
        "replay failed: {}",
        String::from_utf8_lossy(&replay_out.stderr)
    );
    assert_eq!(
        rec_stdout, replay_out.stdout,
        "record vs replay stdout drift"
    );
}

#[test]
fn second_record_run_produces_byte_identical_recording_file() {
    build_axon();
    let dir = temp_dir("rr");
    let src = dir.join("p.ax");
    let rec1 = dir.join("rec1.json");
    let rec2 = dir.join("rec2.json");
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let m1 = mock_model_slow("alpha", 60)
    let m2 = mock_model_slow("beta", 20)
    let m3 = mock_model_slow("gamma", 100)
    let xs = parallel {
        ask m1 { user: "q1" },
        ask m2 { user: "q2" },
        ask m3 { user: "q3" },
    }
    print(list_get(xs, 0))
    print(list_get(xs, 1))
    print(list_get(xs, 2))
}
"#;
    std::fs::write(&src, prog).unwrap();
    for rec in [&rec1, &rec2] {
        let out = Command::new(axon_bin())
            .args([
                "run",
                "--record",
                rec.to_str().unwrap(),
                src.to_str().unwrap(),
            ])
            .output()
            .expect("axon run --record");
        assert!(out.status.success(), "record: {:?}", out);
    }
    let r1 = std::fs::read(&rec1).unwrap();
    let r2 = std::fs::read(&rec2).unwrap();
    assert_eq!(
        r1, r2,
        "two record runs of the same parallel block must produce \
         byte-identical recordings (parallel join is input-ordered, \
         model output is deterministic mock_model_slow)"
    );
}
