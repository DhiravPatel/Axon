//! Stage 34 — `axon fix --interactive` (non-TTY fallback path) and
//! `axon fix --watch` (one-pass auto-apply) end-to-end tests through
//! the binary.
//!
//! TTY-dependent paths (real y/n/a/q prompt handling) aren't exercised
//! here — they need a PTY harness which we don't have in v0. The
//! coverage we do get:
//!   * dry-run output labels each fix with its `[safe]` / `[suggested]`
//!     tier.
//!   * `--interactive` over a piped stdin falls back to dry-run cleanly.
//!   * Mode flags (--apply, --interactive, --watch) are mutually
//!     exclusive — picking two is an exit-2 user error.
//!   * `--watch` runs its startup pass against a single file, applies
//!     every Safe fix once, and exits cleanly when killed via SIGINT.
//!   * `--watch` on a project tree routes cross-file P0010 fixes the
//!     same way the dry-run / `--apply` path does.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

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
    p.push(format!("axon-stage34iw-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write(dir: &std::path::Path, name: &str, src: &str) -> PathBuf {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, src).unwrap();
    p
}

// ---------------------------------------------------------------------------
// Dry-run output now includes tier labels (§34.1)
// ---------------------------------------------------------------------------

#[test]
fn dry_run_labels_each_hunk_with_safe_or_suggested() {
    build_axon();
    let dir = temp_dir("tier_labels");
    let path = write(
        &dir,
        "p.ax",
        "fn add(a: Int, b: Int) -> Int { a + b }\n\
         fn main() uses { Console } {\n\
         \x20   let greeting = \"hi\"\n\
         \x20   print(greetng)\n\
         \x20   print_int(add(1))\n\
         }\n",
    );
    let out = Command::new(axon_bin())
        .args(["fix", path.to_str().unwrap()])
        .output()
        .expect("axon fix");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("E0202, safe"),
        "expected `E0202, safe` tier label: {stdout}"
    );
    assert!(
        stdout.contains("E0205, suggested"),
        "expected `E0205, suggested` tier label: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// --interactive falls back cleanly when stdin is not a TTY.
// ---------------------------------------------------------------------------

#[test]
fn interactive_falls_back_to_dry_run_when_stdin_is_not_a_tty() {
    build_axon();
    let dir = temp_dir("interactive_pipe");
    let path = write(
        &dir,
        "p.ax",
        "fn main() uses { Console } { let greeting = \"hi\"\n  print(greetng) }\n",
    );
    let out = Command::new(axon_bin())
        .args(["fix", "--interactive", path.to_str().unwrap()])
        .stdin(Stdio::null()) // not a TTY → fallback path
        .output()
        .expect("axon fix");
    assert!(out.status.success(), "{:?}", out);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("falling back to dry-run"),
        "expected the non-TTY fallback note: {combined}"
    );
    // And the source file must be unchanged.
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("greetng"),
        "non-TTY --interactive must not write the file: {after}"
    );
}

// ---------------------------------------------------------------------------
// Mode flags are mutually exclusive.
// ---------------------------------------------------------------------------

#[test]
fn mode_flags_are_mutually_exclusive() {
    build_axon();
    let dir = temp_dir("mutex");
    let path = write(&dir, "p.ax", "fn main() { print(\"hi\") }\n");

    for combo in [
        vec!["--apply", "--interactive"],
        vec!["--apply", "--watch"],
        vec!["--interactive", "--watch"],
        vec!["--apply", "--interactive", "--watch"],
    ] {
        let mut args = vec!["fix"];
        args.extend(combo.iter().copied());
        args.push(path.to_str().unwrap());
        let out = Command::new(axon_bin())
            .args(&args)
            .output()
            .expect("axon fix");
        assert!(
            !out.status.success(),
            "expected combo {combo:?} to be rejected"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("mutually exclusive"),
            "expected mutex error for {combo:?}, got: {stderr}"
        );
    }
}

// ---------------------------------------------------------------------------
// --watch single-file: startup pass auto-applies Safe fixes.
// ---------------------------------------------------------------------------

#[test]
fn watch_single_file_startup_pass_auto_applies_safe_fixes() {
    build_axon();
    let dir = temp_dir("watch_file");
    let path = write(
        &dir,
        "p.ax",
        "fn main() uses { Console } {\n\
         \x20   let greeting = \"hi\"\n\
         \x20   print(greetng)\n\
         \x20   let body = read_file(\"r.txt\")\n\
         \x20   print(body)\n\
         }\n",
    );
    // Launch --watch as a child; let the startup pass run; SIGINT to
    // shut it down. The startup pass is synchronous before the event
    // loop, so killing 500 ms later is plenty.
    let mut child = Command::new(axon_bin())
        .args(["fix", "--watch", path.to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("axon fix --watch");
    std::thread::sleep(Duration::from_millis(700));
    // SIGINT — emulate Ctrl-C cleanly.
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let status = child.wait().expect("wait");
    assert!(status.success(), "watch exit: {status}");

    // The startup pass should have auto-applied the two Safe fixes
    // (E0202 typo + E0210 missing Fs.Read).
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("print(greeting)"),
        "E0202 should have auto-applied: {after}"
    );
    assert!(
        after.contains("uses { Console, Fs.Read }"),
        "E0210 should have auto-applied: {after}"
    );
}

// ---------------------------------------------------------------------------
// --watch single-file: Suggested-only file is left untouched + reported.
// ---------------------------------------------------------------------------

#[test]
fn watch_does_not_auto_apply_suggested_fixes() {
    build_axon();
    let dir = temp_dir("watch_suggested");
    // E0205 (nil padding) is Suggested. --watch must NOT rewrite it.
    let path = write(
        &dir,
        "p.ax",
        "fn add(a: Int, b: Int) -> Int { a + b }\n\
         fn main() uses { Console } { print_int(add(1)) }\n",
    );
    let mut child = Command::new(axon_bin())
        .args(["fix", "--watch", path.to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("axon fix --watch");
    std::thread::sleep(Duration::from_millis(700));
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let out = child.wait_with_output().expect("wait_with_output");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Suggested fix"),
        "watch should mention the Suggested fix is unactioned: {stdout}"
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("print_int(add(1))"),
        "Suggested fix must NOT auto-apply: {after}"
    );
}

// ---------------------------------------------------------------------------
// --watch project tree: cross-file P0010 fix lands in the right file.
// ---------------------------------------------------------------------------

#[test]
fn watch_project_mode_routes_cross_file_p0010() {
    build_axon();
    let dir = temp_dir("watch_project");
    write(
        &dir,
        "axon.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\n\
         [run]\nentry = \"main\"\nsrc = \"src\"\n\
         [caps]\ndefault = [\"Console\"]\n",
    );
    write(
        &dir,
        "src/main.ax",
        "use helpers.{greet}\nfn main() uses { Console } { print(greet(\"x\")) }\n",
    );
    let helper_path = write(
        &dir,
        "src/helpers.ax",
        "fn greet(name: String) -> String { \"hi, \" + name }\n",
    );
    // §34.6 verification fix C1 — temp dirs live under /tmp which
    // isn't a descendant of CWD; flip the override so this test
    // exercises just the project-mode P0010 routing, not the
    // CWD-descendant safety check (which has its own dedicated test
    // in stage34_verify_fixes.rs).
    let mut child = Command::new(axon_bin())
        .args(["fix", "--watch", dir.to_str().unwrap()])
        .env("AXON_FIX_WATCH_FORCE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("axon fix --watch");
    std::thread::sleep(Duration::from_millis(900));
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let _ = child.wait().expect("wait");

    let after = std::fs::read_to_string(&helper_path).unwrap();
    assert!(
        after.starts_with("pub fn greet"),
        "P0010 should have inserted `pub` in helpers.ax: {after}"
    );
}
