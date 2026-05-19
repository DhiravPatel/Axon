//! Stage 23 — dynamic-library FFI + delegated identity, end-to-end through
//! the `axon` binary.

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
    p.push(format!("axon-stage23-{name}-{pid}-{ts}"));
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

// ---------- Dynamic-library FFI -----------------------------------------

#[cfg(target_os = "macos")]
const LIBM_PATH: &str = "/usr/lib/libSystem.B.dylib";
#[cfg(target_os = "linux")]
const LIBM_PATH: &str = "libm.so.6";

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ffi_dlib_call_invokes_cos_from_libm() {
    build_axon();
    let dir = temp_dir("dlib_cos");
    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    let arg = {{ ty: "f64", v: 0.0 }}
    let argv = list_new(arg)
    let r = ffi_dlib_call("{lib}", "cos", argv, false)
    print(bool(r.ok))
    print(r.value)
}}
"#,
        lib = LIBM_PATH
    );
    let out = run_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true", "stdout was: {stdout}");
    // cos(0.0) == 1.0 — printed as "1" or "1.0" depending on the runtime
    // float formatter.
    let v = lines[1].trim();
    assert!(v == "1" || v == "1.0", "cos(0) should be 1, got `{v}`");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ffi_dlib_call_reports_missing_library() {
    build_axon();
    let dir = temp_dir("dlib_missing");
    let prog = r#"
fn main() uses { Console } {
    let argv = list_new()
    let r = ffi_dlib_call("/this/path/does/not/exist.so", "foo", argv, false)
    print(bool(r.ok))
    print(str_contains(r.error, "open"))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "false");
    assert_eq!(lines[1], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- Delegated identity ------------------------------------------

#[test]
fn delegation_round_trip_through_disk() {
    build_axon();
    let dir = temp_dir("deleg_rt");
    let signed_path = dir.join("deleg.json");

    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    let kp = a2a_keypair_generate()
    let scopes = list_new("Research", "Summarize")
    let pubkey = a2a_sign_delegation(
        kp.seed_hex,
        "user:alice",
        "research-agent-1",
        scopes,
        9999999999,
        "n-001",
        "{signed}"
    )
    a2a_trust_store_new("alice-delegates", list_new(pubkey))
    let d = a2a_verify_delegation("{signed}", "alice-delegates", "research-agent-1", 1700000000)
    print(d.principal)
    print(d.audience)
    print(d.nonce)
}}
"#,
        signed = signed_path.display()
    );
    let out = run_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "user:alice");
    assert_eq!(lines[1], "research-agent-1");
    assert_eq!(lines[2], "n-001");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delegation_verify_rejects_wrong_audience() {
    build_axon();
    let dir = temp_dir("deleg_aud");
    let signed_path = dir.join("deleg.json");
    let prog = format!(
        r#"
fn main() {{
    let kp = a2a_keypair_generate()
    let scopes = list_new("Research")
    let pubkey = a2a_sign_delegation(
        kp.seed_hex,
        "user:alice",
        "research-agent-1",
        scopes,
        9999999999,
        "n-002",
        "{signed}"
    )
    a2a_trust_store_new("trusted", list_new(pubkey))
    a2a_verify_delegation("{signed}", "trusted", "different-audience", 1700000000)
}}
"#,
        signed = signed_path.display()
    );
    let out = run_in(&dir, &prog);
    assert!(!out.status.success(), "should reject mismatched audience");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("audience"),
        "expected audience error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delegation_verify_rejects_expired() {
    build_axon();
    let dir = temp_dir("deleg_exp");
    let signed_path = dir.join("deleg.json");
    let prog = format!(
        r#"
fn main() {{
    let kp = a2a_keypair_generate()
    let scopes = list_new("Research")
    let pubkey = a2a_sign_delegation(
        kp.seed_hex,
        "user:bob",
        "any-agent",
        scopes,
        1700000000,
        "n-003",
        "{signed}"
    )
    a2a_trust_store_new("trusted", list_new(pubkey))
    a2a_verify_delegation("{signed}", "trusted", "any-agent", 1800000000)
}}
"#,
        signed = signed_path.display()
    );
    let out = run_in(&dir, &prog);
    assert!(!out.status.success(), "should reject expired delegation");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("expired"),
        "expected expiry error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delegation_verify_rejects_untrusted_signer() {
    build_axon();
    let dir = temp_dir("deleg_untrusted");
    let signed_path = dir.join("deleg.json");
    let prog = format!(
        r#"
fn main() {{
    let signer = a2a_keypair_generate()
    let scopes = list_new("Research")
    let _ = a2a_sign_delegation(
        signer.seed_hex,
        "user:carol",
        "any-agent",
        scopes,
        9999999999,
        "n-004",
        "{signed}"
    )
    let other = a2a_keypair_generate()
    a2a_trust_store_new("only-other", list_new(other.pubkey_hex))
    a2a_verify_delegation("{signed}", "only-other", "any-agent", 1700000000)
}}
"#,
        signed = signed_path.display()
    );
    let out = run_in(&dir, &prog);
    assert!(!out.status.success(), "should reject untrusted signer");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("trust") || stderr.contains("Untrusted") || stderr.contains("not in"),
        "expected untrusted-signer error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
