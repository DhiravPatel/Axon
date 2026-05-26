//! Stage 21 — `axon login`, TLS serve, graceful shutdown via the binary.

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
    p.push(format!("axon-stage21-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

// ---------- axon login --------------------------------------------------

#[test]
fn axon_login_with_key_arg_writes_owner_only_vault() {
    build_axon();
    let dir = temp_dir("login_key");
    let vault_path = dir.join("vault.json");
    let out = Command::new(axon_bin())
        .args([
            "login",
            "anthropic",
            "--vault",
            vault_path.to_str().unwrap(),
            "--key",
            "sk-ant-test-value",
        ])
        .output()
        .expect("axon login");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("saved `ANTHROPIC_API_KEY`"),
        "expected confirmation: {stdout}"
    );

    // The vault file should exist with the key inside.
    let raw = std::fs::read_to_string(&vault_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        v["secrets"]["ANTHROPIC_API_KEY"].as_str().unwrap(),
        "sk-ant-test-value"
    );

    // On Unix the file should be 0600.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&vault_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "vault perms should be 0600, got {mode:o}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn axon_login_picks_up_env_var_when_no_key_arg() {
    build_axon();
    let dir = temp_dir("login_env");
    let vault_path = dir.join("vault.json");
    let out = Command::new(axon_bin())
        .args([
            "login",
            "openai",
            "--vault",
            vault_path.to_str().unwrap(),
        ])
        .env("OPENAI_API_KEY", "sk-env-value")
        .output()
        .expect("axon login");
    assert!(out.status.success(), "{:?}", out);
    let raw = std::fs::read_to_string(&vault_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        v["secrets"]["OPENAI_API_KEY"].as_str().unwrap(),
        "sk-env-value"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn axon_login_appends_to_existing_vault() {
    build_axon();
    let dir = temp_dir("login_append");
    let vault_path = dir.join("vault.json");

    // First key.
    Command::new(axon_bin())
        .args([
            "login",
            "anthropic",
            "--vault",
            vault_path.to_str().unwrap(),
            "--key",
            "ant-one",
        ])
        .output()
        .expect("axon login 1");
    // Second key in the same vault.
    Command::new(axon_bin())
        .args([
            "login",
            "openai",
            "--vault",
            vault_path.to_str().unwrap(),
            "--key",
            "oai-two",
        ])
        .output()
        .expect("axon login 2");

    let raw = std::fs::read_to_string(&vault_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(v["secrets"]["ANTHROPIC_API_KEY"].as_str().is_some());
    assert!(v["secrets"]["OPENAI_API_KEY"].as_str().is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- serve --tls -------------------------------------------------

#[test]
fn axon_serve_tls_flag_pair_validation() {
    build_axon();
    let dir = temp_dir("tls_flag");
    let src = dir.join("p.ax");
    std::fs::write(
        &src,
        r#"
fn handle(body: String) -> String { body }
fn main() uses { Console } {
    serve_run("127.0.0.1:0", handle)
}
"#,
    )
    .unwrap();
    // Only --tls-cert without --tls-key should fail fast.
    let out = Command::new(axon_bin())
        .args([
            "serve",
            "--tls-cert",
            "/tmp/nonexistent.pem",
            src.to_str().unwrap(),
        ])
        .output()
        .expect("axon serve");
    assert!(!out.status.success(), "should reject mismatched TLS flags");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("must be used together"),
        "expected pair-validation error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn axon_serve_help_text_documents_tls_and_login() {
    // Sanity check that the user-facing help mentions the new surface.
    // Detailed graceful-shutdown behaviour is unit-tested inside
    // axon-deploy (`graceful_shutdown_drains_in_flight_handler`); the
    // SIGINT-driven integration version was too flaky to run reliably.
    build_axon();
    let out = Command::new(axon_bin())
        .arg("help")
        .output()
        .expect("axon help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for needle in ["--tls-cert", "--tls-key", "graceful shutdown", "login"] {
        assert!(
            stdout.contains(needle),
            "help text should mention `{needle}`: {stdout}"
        );
    }
}
