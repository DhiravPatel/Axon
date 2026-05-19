//! Stage 22 — `sandbox_run_with_profile` + Ed25519 A2A through the binary.

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
    p.push(format!("axon-stage22-{name}-{pid}-{ts}"));
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

// ---------- Platform sandbox --------------------------------------------

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sandbox_run_with_strict_profile_succeeds_on_pure_compute() {
    build_axon();
    let dir = temp_dir("strict_compute");
    let prog = r#"
fn main() uses { Console } {
    let r = sandbox_run_with_profile(
        "/bin/echo", list_new("hello"),
        5, 64, 5, "strict"
    )
    print_int(r.exit_code)
    print(str_trim(r.stdout))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "0");
    assert_eq!(lines[1], "hello");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sandbox_run_with_profile_rejects_unknown_profile() {
    build_axon();
    let dir = temp_dir("bad_profile");
    let prog = r#"
fn main() {
    let _ = sandbox_run_with_profile(
        "/bin/echo", list_new("x"),
        5, 64, 5, "made-up-profile"
    )
}
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "should have failed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown profile"),
        "expected unknown-profile error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- Ed25519 identity --------------------------------------------

#[test]
fn a2a_sign_and_verify_round_trip_through_disk() {
    build_axon();
    let dir = temp_dir("a2a_sign_rt");
    let unsigned_path = dir.join("card.json");
    let signed_path = dir.join("signed.json");

    // Write an unsigned AgentCard.
    std::fs::write(
        &unsigned_path,
        r#"{
            "format_version": 1,
            "agent_id": "researcher-1",
            "name": "Research",
            "version": "1.0.0",
            "description": "demo",
            "endpoint": "https://example.com/agent",
            "capabilities": [
                { "name": "Research", "input_schema_url": null, "output_schema_url": null, "description": "" }
            ],
            "auth": { "scheme": "none" },
            "pricing": null,
            "rate_limits": null,
            "metadata": {}
        }"#,
    )
    .unwrap();

    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    let kp = a2a_keypair_generate()
    let pubkey = a2a_sign_card("{unsigned}", kp.seed_hex, "{signed}")
    a2a_trust_store_new("partners", list_new(pubkey))
    let card = a2a_verify_signed_card("{signed}", "partners")
    print(card.agent_id)
    print(card.name)
}}
"#,
        unsigned = unsigned_path.display(),
        signed = signed_path.display()
    );
    let out = run_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "researcher-1");
    assert_eq!(lines[1], "Research");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a2a_verify_rejects_card_signed_by_untrusted_key() {
    build_axon();
    let dir = temp_dir("a2a_untrusted");
    let unsigned_path = dir.join("card.json");
    let signed_path = dir.join("signed.json");
    std::fs::write(
        &unsigned_path,
        r#"{
            "format_version": 1,
            "agent_id": "x",
            "name": "X",
            "version": "1.0.0",
            "description": "",
            "endpoint": "https://x.example.com",
            "capabilities": [],
            "auth": { "scheme": "none" },
            "pricing": null,
            "rate_limits": null,
            "metadata": {}
        }"#,
    )
    .unwrap();

    // Signer is one keypair; the trust store only allows a DIFFERENT one.
    let prog = format!(
        r#"
fn main() {{
    let signer = a2a_keypair_generate()
    let _ = a2a_sign_card("{unsigned}", signer.seed_hex, "{signed}")

    let attacker = a2a_keypair_generate()
    a2a_trust_store_new("only-attacker", list_new(attacker.pubkey_hex))
    a2a_verify_signed_card("{signed}", "only-attacker")
}}
"#,
        unsigned = unsigned_path.display(),
        signed = signed_path.display()
    );
    let out = run_in(&dir, &prog);
    assert!(!out.status.success(), "should have rejected the signer");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not in the trust store") || stderr.contains("Untrusted"),
        "expected untrusted-signer error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a2a_keypair_from_seed_is_deterministic() {
    build_axon();
    let dir = temp_dir("a2a_seed");
    let prog = r#"
fn main() uses { Console } {
    let seed = "11223344556677889900aabbccddeeff11223344556677889900aabbccddeeff"
    let a = a2a_keypair_from_seed(seed)
    let b = a2a_keypair_from_seed(seed)
    print(a.pubkey_hex)
    print(b.pubkey_hex)
    print(bool(a.pubkey_hex == b.pubkey_hex))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], lines[1], "same seed → same pubkey");
    assert_eq!(lines[2], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a2a_keypair_from_seed_rejects_bad_hex_length() {
    build_axon();
    let dir = temp_dir("a2a_bad_seed");
    let prog = r#"
fn main() {
    a2a_keypair_from_seed("abcd")
}
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "should have failed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("32 bytes") || stderr.contains("64 hex"),
        "expected length error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
