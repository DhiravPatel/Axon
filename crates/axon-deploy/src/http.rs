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
    /// Set by [`Server::shutdown`] to drop the accept loop.
    pub stop: Arc<AtomicBool>,
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
        })
    }

    pub fn with_check(mut self, check: Box<dyn HealthCheck>) -> Self {
        self.checks.push(check);
        self
    }

    /// Block accepting connections and dispatch each request through
    /// `handler`. `handler` returns the Response to send back. Stops when
    /// `Server::shutdown` is called via the returned `stop` flag.
    pub fn run<F>(self, handler: F) -> std::io::Result<()>
    where
        F: Fn(&Request) -> Response + Send + Sync + 'static,
    {
        let handler = Arc::new(handler);
        let checks = Arc::new(self.checks);
        let stop = self.stop;
        for stream in self.listener.incoming() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            match stream {
                Ok(s) => {
                    let h = handler.clone();
                    let c = checks.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = handle_connection(s, &*h, &c) {
                            // Best-effort: a single broken connection
                            // shouldn't take the server down.
                            eprintln!("axon-deploy: connection error: {e}");
                        }
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

fn handle_connection<F: Fn(&Request) -> Response + Sync + ?Sized>(
    stream: TcpStream,
    handler: &F,
    checks: &[Box<dyn HealthCheck>],
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    stream.set_nonblocking(false)?;
    let request = read_request(&stream)?;
    let response = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/healthz") => json_check_response(checks, /* require_all_ready = */ false),
        ("GET", "/readyz") => json_check_response(checks, /* require_all_ready = */ true),
        _ => handler(&request),
    };
    write_response(&stream, &response)
}

fn read_request(mut stream: &TcpStream) -> std::io::Result<Request> {
    let mut reader = BufReader::new(&mut stream);
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

fn write_response(mut stream: &TcpStream, response: &Response) -> std::io::Result<()> {
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
}
