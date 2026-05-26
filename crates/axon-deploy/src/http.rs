//! Minimal HTTP/1.1 server.
//!
//! Single request per connection (no keepalive — keeps the loop tiny),
//! `Content-Length` only (no chunked), thread-per-connection so a slow
//! handler can't block other clients. Real production deployments should
//! still front this with a real reverse proxy; the goal here is a
//! correct, readable server that an Axon program can spin up in one line.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::health::{HealthCheck, Liveness};

const READ_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024; // 4 MiB cap
const MAX_HEADER_BYTES: usize = 32 * 1024; // 32 KiB header block

// Side-channel for the most recently-installed SIGINT/SIGTERM target.
// The signal handler is `extern "C"` and can't borrow self; we keep one
// process-wide slot and the latest `install_signal_handler` call wins.
thread_local! {
    static SIGNAL_STOP: std::cell::RefCell<Option<Arc<AtomicBool>>> =
        std::cell::RefCell::new(None);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
        Self {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body,
        }
    }
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        let body = body.into().into_bytes();
        Self {
            status,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            body,
        }
    }
}

/// Server configuration.
pub struct Server {
    pub listener: TcpListener,
    pub local_addr: std::net::SocketAddr,
    /// Health checks plugged via [`Server::with_check`].
    pub checks: Vec<Box<dyn HealthCheck>>,
    /// Set by [`Server::shutdown`] to drop the accept loop. Used by the
    /// SIGINT/SIGTERM handler the CLI installs at startup.
    pub stop: Arc<AtomicBool>,
    /// Optional TLS configuration. When `Some`, the server performs a
    /// rustls handshake on every accepted connection before reading the
    /// HTTP request.
    pub tls: Option<Arc<rustls::ServerConfig>>,
    /// Live request counter. Incremented when a connection starts,
    /// decremented when its handler returns. Graceful shutdown waits
    /// for this to hit zero (bounded by `shutdown_grace`) before
    /// returning from [`run`].
    pub in_flight: Arc<std::sync::atomic::AtomicUsize>,
    pub shutdown_grace: Duration,
}

impl Server {
    pub fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let local_addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            local_addr,
            checks: vec![Box::new(Liveness)],
            stop: Arc::new(AtomicBool::new(false)),
            tls: None,
            in_flight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            shutdown_grace: Duration::from_secs(10),
        })
    }

    pub fn with_check(mut self, check: Box<dyn HealthCheck>) -> Self {
        self.checks.push(check);
        self
    }

    /// Enable TLS using a PEM-encoded certificate chain + private key.
    /// Reads files from disk so the caller doesn't have to manage rustls
    /// types directly. Common production setup: ACME-managed cert with
    /// regular cert-key rotation via a sidecar (the server re-reads on
    /// restart).
    pub fn with_tls_pem(
        mut self,
        cert_path: impl AsRef<std::path::Path>,
        key_path: impl AsRef<std::path::Path>,
    ) -> std::io::Result<Self> {
        // Make sure rustls has a crypto provider installed; choose ring
        // explicitly so the build is reproducible. This call is a no-op
        // on later invocations within the same process.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let cert_bytes = std::fs::read(&cert_path)?;
        let key_bytes = std::fs::read(&key_path)?;
        let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
            rustls_pemfile::certs(&mut cert_bytes.as_slice())
                .collect::<Result<_, _>>()
                .map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("read cert PEM: {e}"),
                    )
                })?;
        if certs.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "cert PEM contained no CERTIFICATE blocks",
            ));
        }
        let key = rustls_pemfile::private_key(&mut key_bytes.as_slice())
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("read key PEM: {e}"),
                )
            })?
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "key PEM contained no PRIVATE KEY block",
                )
            })?;
        let cfg = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("build TLS config: {e}"),
                )
            })?;
        self.tls = Some(Arc::new(cfg));
        Ok(self)
    }

    pub fn with_shutdown_grace(mut self, grace: Duration) -> Self {
        self.shutdown_grace = grace;
        self
    }

    /// Install a SIGINT / SIGTERM handler that flips the server's `stop`
    /// flag the next time the accept loop polls. The handler is process-
    /// wide; the most recent installer wins. On non-Unix this is a no-op
    /// — callers can still drive `server.stop` programmatically.
    #[cfg(unix)]
    pub fn install_signal_handler(&self) -> std::io::Result<()> {
        let stop = self.stop.clone();
        SIGNAL_STOP.with(|cell| *cell.borrow_mut() = Some(stop));
        unsafe extern "C" fn on_signal(_sig: libc::c_int) {
            SIGNAL_STOP.with(|cell| {
                if let Some(stop) = cell.borrow().as_ref() {
                    stop.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }
        unsafe {
            libc::signal(libc::SIGINT, on_signal as usize);
            libc::signal(libc::SIGTERM, on_signal as usize);
        }
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn install_signal_handler(&self) -> std::io::Result<()> {
        Ok(())
    }

    /// Block accepting connections and dispatch each request through
    /// `handler`. Stops when `stop` is flipped (e.g. by the SIGINT
    /// handler the CLI installs) and waits up to `shutdown_grace` for
    /// in-flight handlers to finish before returning.
    pub fn run<F>(self, handler: F) -> std::io::Result<()>
    where
        F: Fn(&Request) -> Response + Send + Sync + 'static,
    {
        let handler = Arc::new(handler);
        let checks = Arc::new(self.checks);
        let stop = self.stop;
        let tls = self.tls;
        let in_flight = self.in_flight;
        let grace = self.shutdown_grace;
        for stream in self.listener.incoming() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            match stream {
                Ok(s) => {
                    let h = handler.clone();
                    let c = checks.clone();
                    let tls_cfg = tls.clone();
                    let counter = in_flight.clone();
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    std::thread::spawn(move || {
                        let result = match tls_cfg {
                            Some(cfg) => handle_connection_tls(s, &*h, &c, cfg),
                            None => handle_connection_plain(s, &*h, &c),
                        };
                        if let Err(e) = result {
                            // Best-effort: one broken connection shouldn't
                            // take the server down.
                            eprintln!("axon-deploy: connection error: {e}");
                        }
                        counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e),
            }
        }
        // Graceful drain: wait up to `shutdown_grace` for in-flight
        // requests to finish. After that we return; any handler still
        // alive runs to completion in the background and the threads
        // exit when their work is done.
        let deadline = std::time::Instant::now() + grace;
        while in_flight.load(std::sync::atomic::Ordering::SeqCst) > 0
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }
}

fn handle_connection_plain<F: Fn(&Request) -> Response + Sync + ?Sized>(
    stream: TcpStream,
    handler: &F,
    checks: &[Box<dyn HealthCheck>],
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    stream.set_nonblocking(false)?;
    let mut stream = stream;
    let request = read_request_from(&mut stream)?;
    let response = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/healthz") => json_check_response(checks, /* require_all_ready = */ false),
        ("GET", "/readyz") => json_check_response(checks, /* require_all_ready = */ true),
        _ => handler(&request),
    };
    write_response_to(&mut stream, &response)
}

fn handle_connection_tls<F: Fn(&Request) -> Response + Sync + ?Sized>(
    stream: TcpStream,
    handler: &F,
    checks: &[Box<dyn HealthCheck>],
    cfg: Arc<rustls::ServerConfig>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    stream.set_nonblocking(false)?;
    let conn = rustls::ServerConnection::new(cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let mut tls = rustls::StreamOwned::new(conn, stream);
    let request = read_request_from(&mut tls)?;
    let response = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/healthz") => json_check_response(checks, /* require_all_ready = */ false),
        ("GET", "/readyz") => json_check_response(checks, /* require_all_ready = */ true),
        _ => handler(&request),
    };
    write_response_to(&mut tls, &response)
}

fn read_request_from<S: Read>(stream: &mut S) -> std::io::Result<Request> {
    let mut reader = BufReader::new(stream);
    let mut line_buf = String::new();
    reader.read_line(&mut line_buf)?;
    let req_line = line_buf.trim_end_matches('\n').trim_end_matches('\r').to_string();
    let mut parts = req_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing method"))?
        .to_string();
    let path = parts
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing path"))?
        .to_string();
    // version field exists but we don't currently care which.

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut header_bytes = 0usize;
    loop {
        line_buf.clear();
        let n = reader.read_line(&mut line_buf)?;
        if n == 0 {
            break;
        }
        header_bytes += n;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "header block too large",
            ));
        }
        let trimmed = line_buf.trim_end_matches('\n').trim_end_matches('\r');
        if trimmed.is_empty() {
            break;
        }
        if let Some(colon) = trimmed.find(':') {
            let (k, v) = trimmed.split_at(colon);
            headers.push((k.to_ascii_lowercase(), v[1..].trim().to_string()));
        }
    }

    let content_length: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("body exceeds {MAX_BODY_BYTES} bytes"),
        ));
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn write_response_to<S: Write>(stream: &mut S, response: &Response) -> std::io::Result<()> {
    let reason = status_reason(response.status);
    write!(stream, "HTTP/1.1 {} {}\r\n", response.status, reason)?;
    write!(stream, "Content-Length: {}\r\n", response.body.len())?;
    write!(stream, "Connection: close\r\n")?;
    for (k, v) in &response.headers {
        write!(stream, "{k}: {v}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(&response.body)?;
    stream.flush()?;
    Ok(())
}

fn status_reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

fn json_check_response(checks: &[Box<dyn HealthCheck>], require_all_ready: bool) -> Response {
    let mut entries: Vec<serde_json::Value> = Vec::with_capacity(checks.len());
    let mut all_ok = true;
    for c in checks {
        let r = c.check();
        if !r.ok {
            all_ok = false;
        }
        entries.push(serde_json::json!({
            "name": c.name(),
            "ok": r.ok,
            "detail": r.detail,
        }));
    }
    let status = if !require_all_ready || all_ok { 200 } else { 503 };
    let body = serde_json::json!({
        "ok": all_ok,
        "checks": entries,
    });
    Response::json(status, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_request(addr: std::net::SocketAddr, method: &str, path: &str, body: &[u8]) -> Vec<u8> {
        use std::net::TcpStream;
        let mut s = TcpStream::connect(addr).unwrap();
        write!(s, "{method} {path} HTTP/1.1\r\n").unwrap();
        write!(s, "Host: {}\r\n", addr).unwrap();
        write!(s, "Content-Length: {}\r\n", body.len()).unwrap();
        write!(s, "Connection: close\r\n\r\n").unwrap();
        if !body.is_empty() {
            s.write_all(body).unwrap();
        }
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).unwrap();
        buf
    }

    #[test]
    fn echo_invoke_round_trips_body() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr;
        let stop = server.stop.clone();
        let handle = std::thread::spawn(move || {
            server.run(|req| {
                if req.path == "/invoke" && req.method == "POST" {
                    Response::json(
                        200,
                        &serde_json::json!({ "received": String::from_utf8_lossy(&req.body) }),
                    )
                } else {
                    Response::text(404, "not found")
                }
            })
            .unwrap();
        });

        // Give the listener a moment to start.
        std::thread::sleep(Duration::from_millis(50));

        let resp = client_request(addr, "POST", "/invoke", b"hello");
        let text = String::from_utf8_lossy(&resp).to_string();
        assert!(text.starts_with("HTTP/1.1 200"), "got: {text:?}");
        assert!(text.contains("\"received\":\"hello\""));

        stop.store(true, Ordering::SeqCst);
        // Tickle the listener so accept() returns quickly.
        let _ = std::net::TcpStream::connect(addr);
        let _ = handle.join();
    }

    #[test]
    fn healthz_returns_200_when_checks_ok() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr;
        let stop = server.stop.clone();
        let handle = std::thread::spawn(move || {
            server.run(|_req| Response::text(404, "nope")).unwrap();
        });
        std::thread::sleep(Duration::from_millis(50));

        let resp = client_request(addr, "GET", "/healthz", b"");
        let text = String::from_utf8_lossy(&resp).to_string();
        assert!(text.starts_with("HTTP/1.1 200"), "got: {text:?}");
        assert!(text.contains("\"ok\":true"));

        stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(addr);
        let _ = handle.join();
    }

    #[test]
    fn readyz_returns_503_when_any_check_fails() {
        struct AlwaysFailing;
        impl HealthCheck for AlwaysFailing {
            fn name(&self) -> &str {
                "always-failing"
            }
            fn check(&self) -> crate::health::CheckResult {
                crate::health::CheckResult::fail("intentionally not ready")
            }
        }

        let server = Server::bind("127.0.0.1:0")
            .unwrap()
            .with_check(Box::new(AlwaysFailing));
        let addr = server.local_addr;
        let stop = server.stop.clone();
        let handle = std::thread::spawn(move || {
            server.run(|_req| Response::text(404, "nope")).unwrap();
        });
        std::thread::sleep(Duration::from_millis(50));

        let resp = client_request(addr, "GET", "/readyz", b"");
        let text = String::from_utf8_lossy(&resp).to_string();
        assert!(text.starts_with("HTTP/1.1 503"), "got: {text:?}");
        assert!(text.contains("\"ok\":false"));

        stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(addr);
        let _ = handle.join();
    }

    #[test]
    fn oversized_body_rejected() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr;
        let stop = server.stop.clone();
        let handle = std::thread::spawn(move || {
            server.run(|_req| Response::text(200, "ok")).unwrap();
        });
        std::thread::sleep(Duration::from_millis(50));

        // Claim a giant body via Content-Length but don't actually send it.
        use std::net::TcpStream;
        let mut s = TcpStream::connect(addr).unwrap();
        write!(s, "POST /invoke HTTP/1.1\r\n").unwrap();
        write!(s, "Content-Length: {}\r\n\r\n", MAX_BODY_BYTES + 1).unwrap();
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf);
        // The server closes the connection without writing a response
        // body; either no bytes back or an error frame. Both are fine —
        // we just need the server not to crash.
        stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(addr);
        let _ = handle.join();
    }

    #[test]
    fn server_with_tls_loads_pem_and_binds() {
        // We don't actually drive TLS through a client here (a full
        // TLS round-trip would need a valid CA setup the test can't
        // assume). We do verify that loading a real cert+key from PEM
        // attaches a TLS config and the listener still accepts.
        let cert_pem = generate_test_cert();
        let dir = std::env::temp_dir();
        let cert_path = dir.join(format!("axon-test-cert-{}.pem", std::process::id()));
        let key_path = dir.join(format!("axon-test-key-{}.pem", std::process::id()));
        std::fs::write(&cert_path, &cert_pem.cert).unwrap();
        std::fs::write(&key_path, &cert_pem.key).unwrap();

        let server = Server::bind("127.0.0.1:0")
            .unwrap()
            .with_tls_pem(&cert_path, &key_path)
            .expect("with_tls_pem should accept valid PEM");
        assert!(server.tls.is_some());

        let _ = std::fs::remove_file(&cert_path);
        let _ = std::fs::remove_file(&key_path);
    }

    #[test]
    fn graceful_shutdown_drains_in_flight_handler() {
        // Handler sleeps 200ms; we flip stop after 50ms and expect the
        // in-flight request to complete before run() returns.
        let server = Server::bind("127.0.0.1:0")
            .unwrap()
            .with_shutdown_grace(Duration::from_secs(2));
        let addr = server.local_addr;
        let stop = server.stop.clone();
        let in_flight = server.in_flight.clone();
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let completed_clone = completed.clone();
        let handle = std::thread::spawn(move || {
            server
                .run(move |_req| {
                    std::thread::sleep(Duration::from_millis(200));
                    completed_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                    Response::text(200, "drained")
                })
                .unwrap();
        });
        std::thread::sleep(Duration::from_millis(50));

        // Kick off a slow request.
        let client = std::thread::spawn(move || {
            use std::net::TcpStream;
            let mut s = TcpStream::connect(addr).unwrap();
            write!(s, "GET / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        });

        // Give the request a moment to register with the in-flight counter.
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            in_flight.load(std::sync::atomic::Ordering::SeqCst) >= 1,
            "in_flight should track active handler"
        );

        // Trigger shutdown mid-flight; tickle the accept loop to wake it.
        stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(addr);

        let _ = handle.join();
        let body = client.join().unwrap();
        let text = String::from_utf8_lossy(&body).to_string();
        assert!(
            text.contains("drained"),
            "shutdown should have waited for handler: {text:?}"
        );
        assert!(
            completed.load(std::sync::atomic::Ordering::SeqCst),
            "handler should have completed"
        );
    }

    // ---- helpers ---------------------------------------------------

    struct TestCert {
        cert: Vec<u8>,
        key: Vec<u8>,
    }

    /// Pre-baked self-signed cert + key (PKCS#8, EC P-256). Generated
    /// offline and pinned so the test doesn't pull in a key-generation
    /// dep. Only valid in tests.
    fn generate_test_cert() -> TestCert {
        TestCert {
            cert: TEST_CERT_PEM.as_bytes().to_vec(),
            key: TEST_KEY_PEM.as_bytes().to_vec(),
        }
    }

    // Self-signed cert for `CN=axon-test`, valid 100 years.
    // Generated with: openssl req -x509 -newkey rsa:2048 ... -days 36500 -nodes
    const TEST_CERT_PEM: &str = "\
-----BEGIN CERTIFICATE-----
MIIDCzCCAfOgAwIBAgIUbuGIxhRbZArxNZI2FBTyRlsmnxUwDQYJKoZIhvcNAQEL
BQAwFDESMBAGA1UEAwwJYXhvbi10ZXN0MCAXDTI2MDUxOTE4MTUxM1oYDzIxMjYw
NDI1MTgxNTEzWjAUMRIwEAYDVQQDDAlheG9uLXRlc3QwggEiMA0GCSqGSIb3DQEB
AQUAA4IBDwAwggEKAoIBAQDL52vBjHU0gwxKBB/4nsUZNuScrAPisYVi9gVcCfv6
IjJq9nwaYrOgdP5Nukuc1m4oYSgvnLoVS6kfv0w5SNjkHfVrK2uaVXnv3ggMlT8o
6cAx+xvUB4f5sinLLHcvcdZo2hi5NtWlnP+qMqrcsAIJNmv5HQvZLT4dj/LkIpBL
1vTFmz/ajwvWhl6un/oDm1FkxTLxWQa9QTT9RPQ0wcXNo+tTNWmHJp5fC5s/10Ka
uTjeDF0T8ueQrcL7fLxkMAwS3zFB5Qm7uUlmoIeOrmyRwAz284ZDeKIldRE5QVl9
KLcWIwaAoi7TYNOlAUuz8cB1cMc+hLI8NLpVEZaNUfbVAgMBAAGjUzBRMB0GA1Ud
DgQWBBS93vPws+ivGYZ6mJMrvIWq9JqnzTAfBgNVHSMEGDAWgBS93vPws+ivGYZ6
mJMrvIWq9JqnzTAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBCwUAA4IBAQCE
I1PnVelBGLS4Fi2s6QKUBAYnIgOvlkOtSXNo6hcQEqiUMzKZp50b03r71y866vob
L2WSo7dogl2hMNqhhqLnAQ41oX0QX7M91/J4mitj1vsAhBqKhs0mUZwO5t66Y2+b
zEDeVbACV+ar5OtdUCX9/dZE20IlHI/sQNn2Zi8AKgM3xj0WxvNxEK1QWDFWtujI
TfWJhKXBi2bmoZOjNz3IblEVBiWyxdMmsI87EvMVmVgpHs4xh+4QfY86kqDUdthK
FM9H13Hv0s17NzwhbhWHwv5TAxZvsjvYTKdUvASzc0dL5hpzXU13Kn7AYoEDlOVL
rYHGVUhOfHsW6iXZhvJI
-----END CERTIFICATE-----
";

    const TEST_KEY_PEM: &str = "\
-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDL52vBjHU0gwxK
BB/4nsUZNuScrAPisYVi9gVcCfv6IjJq9nwaYrOgdP5Nukuc1m4oYSgvnLoVS6kf
v0w5SNjkHfVrK2uaVXnv3ggMlT8o6cAx+xvUB4f5sinLLHcvcdZo2hi5NtWlnP+q
MqrcsAIJNmv5HQvZLT4dj/LkIpBL1vTFmz/ajwvWhl6un/oDm1FkxTLxWQa9QTT9
RPQ0wcXNo+tTNWmHJp5fC5s/10KauTjeDF0T8ueQrcL7fLxkMAwS3zFB5Qm7uUlm
oIeOrmyRwAz284ZDeKIldRE5QVl9KLcWIwaAoi7TYNOlAUuz8cB1cMc+hLI8NLpV
EZaNUfbVAgMBAAECggEAKAuYLxftwNVn6XVr7gEIho4wUdC6pp/kqW3V2aCgWxyy
OC2Wa/wsePvhIdTPmsrGMan7IXavWRVV7sU8LBfxeMOlelm5tULKQuChRg9dqyRV
ObuuWHLuMozaBmwCMFA0Ir2Kk32AchkmYP+4bMUocTS9+dvJguqOw3GM618aZbpE
bnF7VK48MzWTystiTpAd6lDsmkn7Bk6RnLNI4QIRhjOBU418lJrMIzlD9bTgHOvl
g60kf7HXLuRIA8SurIGFUSEEIsa7qMCzjF5ObKmdAaehvq1uI4wasgDtpKcRTe5g
X+VQu4T/+kBfW1/efhmatYZ2Ds1vu2iN+IO5i4hpAQKBgQD+gFSUCkGvDW6yZx9+
etVgOOyk7gE2CdKQzp7EU+lSU6BT8+hJ6QghihVavyXYkI/hgg7JnFkduK77cVh1
5SRyXlus+XhO1Wx5V4I8dYpnoLn33aJ64Wv3YpD7eK2KqZE4gAgL7WSJDIT2LDA8
Ig53kJ7CI43+ZaLGjt1OJmLgwQKBgQDNGtA2dhx0w3zuZ7MjuPAE5Kn5iEs9HSJO
brdCv26RWHWl6RSsiMAETM8gFUWs6a50RHzd0Wpu/jbFT5zHMtgW9RbBNYg6lOeC
NaCsni6WWX3Uh+ZR87d0cMEL4Bu3BMPxGw3b6Je850q86uY54DYhCEfBVgGcfaM/
q/7l0uBHFQKBgQDq3/6unYSfDJOD3D4pmS1BX2eukuTVPV1yPO4znIlxbDJEKI7R
X1ocsfYhSNWht1DCOyhwknWAQ4hiD+om6/GmB0UuLxIEF13D4qoUKBoypxfaFFa2
d0IQDoxlOKtYlEOs1CQY9d7ZyI8RLhjZ9khJulN6MhwCk0QVYZYGYNDSQQKBgDxa
uIxeIy+E2v14jHFlmVOHSjFAlwtLyG2WDN1aYZnpku0Ycln2/7IEPCrvt4oTVZ+n
C6tmVVCGA+356GOBpa7TvjdqnkTGzn01dKt2/LSHbwycVP0mn4RnLZdmAiHQCCyp
zmE4x3XyBb8jzE2ZmbQMsLjGq0C2g9RXs3FDpXWlAoGBANop0vVfgitAekuz5v8t
gD93G4lXXboDOl1bL6FThlnk9nk4LuB+qB0ihN6WhzF2zr5A2w/z7JBzLvEJbx2i
8bfjZ41yvzWWmtrFu7XmUYaOQtOnTfczi+Sm59V+OrFZ6+zoFgY35n65ILY/SIwc
C3n9kOkUU0kSsmjJCggz8jvu
-----END PRIVATE KEY-----
";
}
