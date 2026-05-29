//! Real measured numbers for the §32.1 async I/O slice.
//!
//! Runs three Axon programs end-to-end (`noop.ax`, `serial.ax`,
//! `parallel.ax`) and reports wall times. The sum→max drop on the
//! parallel batch is the published §32.1 acceptance number, in
//! production form: a real `axon run` invocation, not a Rust unit test.
//!
//! Numbers print to stdout in a `cargo test -- --nocapture` friendly
//! shape. The runner asserts the parallel batch is at least 2× faster
//! than serial — that's the loose floor; a healthy run is closer to
//! 4× since batch size is 4.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

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

fn bench_file(name: &str) -> PathBuf {
    workspace_root()
        .join("examples")
        .join("dogfood_triage")
        .join("benches")
        .join(name)
}

/// Run one Axon program and return its wall time + stdout. Run-to-run
/// variance comes mostly from the OS thread scheduler; we run each test
/// `REPEATS` times and take the minimum, which biases away from
/// outliers caused by other processes on the box.
fn time_axon_run(path: &std::path::Path, repeats: usize) -> Duration {
    let mut best = Duration::from_secs(60);
    for _ in 0..repeats {
        let t0 = Instant::now();
        let out = Command::new(axon_bin())
            .args(["run", path.to_str().unwrap()])
            .output()
            .expect("axon run");
        let dt = t0.elapsed();
        assert!(
            out.status.success(),
            "{path:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if dt < best {
            best = dt;
        }
    }
    best
}

#[test]
fn parallel_asks_beats_serial_by_at_least_2x() {
    build_axon();
    // Baseline first so the noise from process startup gets out of the
    // way before we start the work-bearing runs.
    let noop = time_axon_run(&bench_file("noop.ax"), 3);
    let serial = time_axon_run(&bench_file("serial.ax"), 3);
    let parallel = time_axon_run(&bench_file("parallel.ax"), 3);

    let serial_work = serial.saturating_sub(noop);
    let parallel_work = parallel.saturating_sub(noop);

    println!();
    println!("============= dogfood benchmarks =============");
    println!("workload : 4 mock_model_slow(200ms) asks × 5 repeats");
    println!();
    println!("axon-run overhead (noop)          : {:?}", noop);
    println!("serial    total wall              : {:?}", serial);
    println!("parallel  total wall              : {:?}", parallel);
    println!();
    println!("serial    work (minus baseline)   : {:?}", serial_work);
    println!("parallel  work (minus baseline)   : {:?}", parallel_work);
    let speedup_x10 = if parallel_work.as_millis() > 0 {
        (serial_work.as_millis() * 10) / parallel_work.as_millis()
    } else {
        0
    };
    println!(
        "speedup                           : {}.{}×",
        speedup_x10 / 10,
        speedup_x10 % 10
    );
    println!("==============================================");
    println!();

    // The acceptance gate. 4 calls of 200ms in serial = ~4s of work.
    // In parallel they should overlap to ~1× their max = ~200ms of work
    // per batch, repeated 5 times = ~1s. So the lower bound is ~4×; we
    // assert a soft 2× to absorb worker-pool startup and scheduler noise.
    assert!(
        parallel_work.as_millis() * 2 < serial_work.as_millis(),
        "parallel ({parallel_work:?}) should be at least 2× faster than serial ({serial_work:?})"
    );
}

#[test]
fn triage_agent_replay_is_byte_identical() {
    build_axon();
    // Run the dogfood triage agent twice: once with --record, once
    // with --replay. Same source, same fixtures, same fixed seed = byte-
    // identical stdout. This is the spec's hard guarantee on a non-
    // trivial program (parallel classification + memory + policy).
    let project = workspace_root().join("examples").join("dogfood_triage");
    let rec = project.join("recordings").join("bench-run.json");
    let _ = std::fs::remove_file(&rec);

    let record = Command::new(axon_bin())
        .args([
            "run",
            "--record",
            rec.to_str().unwrap(),
            project.to_str().unwrap(),
        ])
        .output()
        .expect("axon run --record");
    assert!(record.status.success(), "{:?}", record);
    let recorded_stdout = record.stdout;

    let replay = Command::new(axon_bin())
        .args([
            "run",
            "--replay",
            rec.to_str().unwrap(),
            project.to_str().unwrap(),
        ])
        .output()
        .expect("axon run --replay");
    assert!(replay.status.success(), "{:?}", replay);
    let replayed_stdout = replay.stdout;

    assert_eq!(
        recorded_stdout, replayed_stdout,
        "replay diverged from record on the dogfood triage run"
    );
    let _ = std::fs::remove_file(&rec);
}
