//! Stage 17 — `env_*`, `serve_run`, `axon serve`, `axon deploy` through the binary.

use std::io::{Read, Write};
use std::net::TcpStream;
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
    p.push(format!("axon-stage17-{name}-{pid}-{ts}"));
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

// ---------- env / dotenv -----------------------------------------------

#[test]
fn env_get_or_returns_default_when_unset() {
    build_axon();
    let dir = temp_dir("env_default");
    let unique_name = format!("AXON_TEST_NEVER_SET_{}", std::process::id());
    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    let v = env_get_or("{name}", "fallback")
    print(v)
}}
"#,
        name = unique_name
    );
    let out = run_program_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim().ends_with("fallback"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn env_load_dotenv_sets_then_reads_back() {
    build_axon();
    let dir = temp_dir("env_dotenv");
    let env_path = dir.join(".env");
    let unique = format!("AXON_TEST_DOTENV_KEY_{}", std::process::id());
    std::fs::write(&env_path, format!("{unique}=loaded-value\n")).unwrap();

    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    let n = env_load_dotenv("{path}", false)
    print_int(n)
    print(env_get_or("{key}", "absent"))
}}
"#,
        path = env_path.display(),
        key = unique
    );
    let out = run_program_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].parse::<i64>().unwrap() >= 1);
    assert_eq!(lines[1], "loaded-value");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- axon serve --------------------------------------------------

#[test]
fn axon_serve_routes_invoke_through_handler() {
    build_axon();
    let dir = temp_dir("serve_invoke");
    let src = dir.join("server.ax");
    std::fs::write(
        &src,
        r#"
fn handle(body: String) -> String {
    str_join("", list_new("you sent: ", body))
}
fn main() uses { Console } {
    serve_run("127.0.0.1:0", handle)
}
"#,
    )
    .unwrap();

    // We can't pre-bind 127.0.0.1:0 (the OS assigns the port at bind
    // time), so this test uses a fixed port window and retries. For
    // reproducibility we pick a pseudo-random port from the test's PID.
    let port = 18000 + (std::process::id() % 4000) as u16;
    std::fs::write(
        &src,
        format!(
            r#"
fn handle(body: String) -> String {{
    str_join("", list_new("you sent: ", body))
}}
fn main() uses {{ Console }} {{
    serve_run("127.0.0.1:{port}", handle)
}}
"#
        ),
    )
    .unwrap();

    let mut server = Command::new(axon_bin())
        .args(["run", src.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn axon run");

    // Poll until the port is open or give up after a few seconds.
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut connected = None;
    while std::time::Instant::now() < deadline {
        if let Ok(s) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
            connected = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut s = connected.expect("server never came up");

    let body = b"hello";
    write!(s, "POST /invoke HTTP/1.1\r\n").unwrap();
    write!(s, "Host: {addr}\r\n").unwrap();
    write!(s, "Content-Length: {}\r\n", body.len()).unwrap();
    write!(s, "Connection: close\r\n\r\n").unwrap();
    s.write_all(body).unwrap();

    let mut response = Vec::new();
    s.read_to_end(&mut response).unwrap();
    let text = String::from_utf8_lossy(&response).to_string();
    assert!(text.starts_with("HTTP/1.1 200"), "got: {text:?}");
    assert!(text.contains("you sent: hello"), "got: {text:?}");

    // Hit healthz too.
    let mut s = TcpStream::connect(addr).unwrap();
    write!(s, "GET /healthz HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").unwrap();
    let mut hresp = Vec::new();
    s.read_to_end(&mut hresp).unwrap();
    let htext = String::from_utf8_lossy(&hresp).to_string();
    assert!(htext.starts_with("HTTP/1.1 200"), "healthz: {htext:?}");
    assert!(htext.contains("\"ok\":true"));

    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- axon deploy -------------------------------------------------

#[test]
fn axon_deploy_writes_skill_and_manifest() {
    build_axon();
    let dir = temp_dir("deploy");
    let src_dir = dir.join("src_project");
    let out_dir = dir.join("dist");
    std::fs::create_dir_all(src_dir.join("src")).unwrap();
    std::fs::write(
        src_dir.join("manifest.json"),
        r#"{
            "name": "demo-svc",
            "version": "1.0.0",
            "description": "demo",
            "entrypoint": "src/main.ax",
            "capabilities": ["Console"],
            "dependencies": [],
            "authors": []
        }"#,
    )
    .unwrap();
    std::fs::write(
        src_dir.join("src/main.ax"),
        "fn main() uses { Console } { print(\"ok\") }\n",
    )
    .unwrap();

    let out = Command::new(axon_bin())
        .args([
            "deploy",
            src_dir.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
            "--port",
            "9090",
            "--handler",
            "main",
        ])
        .output()
        .expect("axon deploy");
    assert!(out.status.success(), "{:?}", out);

    let skill_path = out_dir.join("demo-svc.axskill");
    let manifest_path = out_dir.join("deploy.json");
    assert!(skill_path.exists(), "skill not written");
    assert!(manifest_path.exists(), "manifest not written");

    let manifest_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest_json["name"], "demo-svc");
    assert_eq!(manifest_json["port"], 9090);
    assert_eq!(manifest_json["entrypoint_handler"], "main");

    let _ = std::fs::remove_dir_all(&dir);
}
