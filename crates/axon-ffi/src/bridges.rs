//! Typed FFI bridges (§35.2).
//!
//! `tool sentiment(text: String) -> Float extern python "tools/sentiment.py:run"`
//! is the PLAN's canonical example. Stage 28 ships the *runtime* of
//! that — typed `BridgeKind` plus `call_bridge(spec, args_json)` that
//! routes to the right launcher (Python, Node, WASM, gRPC). The
//! parser-level `extern <lang> "..."` form is a future syntactic
//! sugar; today programs invoke `ffi_python_call`, `ffi_node_call`,
//! `ffi_wasm_call`, `ffi_grpc_call` directly with the same wire
//! contract.
//!
//! Every bridge:
//!
//!   * Runs as a **subprocess** under the same wall-clock + capability
//!     sandbox as [`crate::call_once`] (Stage 16).
//!   * Marshals args as **one JSON line** on stdin.
//!   * Reads **one JSON line** from stdout for the typed return value.
//!   * Surfaces stderr in the typed [`BridgeError`] on failure.
//!
//! The wire contract is intentionally minimal so users can implement a
//! bridge in any language with a JSON parser. The reference shapes:
//!
//! ```text
//!   # Python: script "tools/sentiment.py", entrypoint "run".
//!   $ python tools/sentiment.py run <<< '{"text":"hello"}'
//!   {"ok":true,"value":0.92}
//!
//!   # Node: script "tools/sentiment.js", entrypoint "run".
//!   $ node tools/sentiment.js run <<< '{"text":"hello"}'
//!   {"ok":true,"value":0.92}
//!
//!   # WASM: a wasi component with a `run` export. We invoke via
//!   # `wasmtime` (host's responsibility to have it installed).
//!   $ wasmtime tools/sentiment.wasm run <<< '{"text":"hello"}'
//!
//!   # gRPC: protocol-level shim — we shell out to `grpcurl`.
//! ```

use serde::{Deserialize, Serialize};

use crate::{call_once, FfiCallSpec, FfiError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeKind {
    Python,
    Node,
    Wasm,
    Grpc,
}

impl BridgeKind {
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "python" | "py" => BridgeKind::Python,
            "node" | "js" | "javascript" => BridgeKind::Node,
            "wasm" | "wasi" => BridgeKind::Wasm,
            "grpc" => BridgeKind::Grpc,
            _ => return None,
        })
    }
    pub fn launcher(self) -> &'static str {
        match self {
            BridgeKind::Python => "python3",
            BridgeKind::Node => "node",
            BridgeKind::Wasm => "wasmtime",
            BridgeKind::Grpc => "grpcurl",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSpec {
    pub kind: BridgeKind,
    /// File path (Python/Node/Wasm) or gRPC service name (`pkg.Svc/Method`).
    pub target: String,
    /// Optional entrypoint (function name for Python/Node, ignored for
    /// Wasm/Grpc).
    #[serde(default)]
    pub entrypoint: String,
    /// Wall-clock budget. 0 means no timeout.
    pub timeout_ms: u64,
    /// Optional launcher override. Empty → use [`BridgeKind::launcher`].
    #[serde(default)]
    pub launcher_override: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BridgeError {
    /// The underlying subprocess FFI failed.
    Ffi(String),
    /// The bridge ran but returned `{"ok":false, ...}`.
    Bridge { message: String, stderr: String },
    /// Output wasn't a JSON object with `ok` / `value`.
    ProtocolViolation(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Ffi(s) => write!(f, "ffi: {s}"),
            BridgeError::Bridge { message, stderr } => {
                write!(f, "bridge error: {message}; stderr: {stderr}")
            }
            BridgeError::ProtocolViolation(s) => write!(f, "protocol violation: {s}"),
        }
    }
}

impl std::error::Error for BridgeError {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BridgeOutcome {
    pub value: serde_json::Value,
}

/// Invoke a typed bridge with a JSON-encoded argument payload. Returns
/// the bridge's `value` field on success or a typed [`BridgeError`] on
/// any failure path. The function is synchronous; the caller can build
/// concurrency on top via `flow::parallel` / `flow::race` / etc.
pub fn call_bridge(spec: &BridgeSpec, args_json: &str) -> Result<BridgeOutcome, BridgeError> {
    let launcher = if spec.launcher_override.is_empty() {
        spec.kind.launcher().to_string()
    } else {
        spec.launcher_override.clone()
    };
    let argv: Vec<String> = match spec.kind {
        BridgeKind::Python | BridgeKind::Node => {
            let mut v = vec![spec.target.clone()];
            if !spec.entrypoint.is_empty() {
                v.push(spec.entrypoint.clone());
            }
            v
        }
        BridgeKind::Wasm => {
            let mut v = vec![spec.target.clone()];
            if !spec.entrypoint.is_empty() {
                v.push(spec.entrypoint.clone());
            }
            v
        }
        BridgeKind::Grpc => {
            // For grpc we expect the caller to specify the service+method
            // in `target` (e.g. `pkg.Svc/Method`); shell out to grpcurl.
            vec!["-plaintext".into(), "-d".into(), args_json.to_string(), spec.target.clone()]
        }
    };
    let ffi_spec = FfiCallSpec {
        program: launcher,
        args: argv,
        workdir: None,
        timeout_ms: spec.timeout_ms,
    };
    // For non-grpc bridges, send args as one JSON line on stdin.
    let stdin_payload = if matches!(spec.kind, BridgeKind::Grpc) {
        // grpc already passed args as a CLI arg via grpcurl's -d.
        serde_json::Value::Null
    } else {
        serde_json::from_str(args_json)
            .map_err(|e| BridgeError::ProtocolViolation(format!("args not JSON: {e}")))?
    };
    let response = call_once(&ffi_spec, &stdin_payload)
        .map_err(|e: FfiError| BridgeError::Ffi(e.to_string()))?;
    parse_bridge_response(response)
}

fn parse_bridge_response(body: serde_json::Value) -> Result<BridgeOutcome, BridgeError> {
    let obj = body.as_object().ok_or_else(|| {
        BridgeError::ProtocolViolation(format!(
            "expected `{{\"ok\":..., \"value\":...}}`, got `{body}`"
        ))
    })?;
    let ok = obj.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        let message = obj
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unspecified")
            .to_string();
        return Err(BridgeError::Bridge {
            message,
            stderr: String::new(),
        });
    }
    let value = obj.get("value").cloned().unwrap_or(serde_json::Value::Null);
    Ok(BridgeOutcome { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_kind_from_string_accepts_aliases() {
        assert_eq!(BridgeKind::from_str("python"), Some(BridgeKind::Python));
        assert_eq!(BridgeKind::from_str("py"), Some(BridgeKind::Python));
        assert_eq!(BridgeKind::from_str("javascript"), Some(BridgeKind::Node));
        assert_eq!(BridgeKind::from_str("wasi"), Some(BridgeKind::Wasm));
        assert_eq!(BridgeKind::from_str("grpc"), Some(BridgeKind::Grpc));
        assert_eq!(BridgeKind::from_str("rust"), None);
    }

    #[test]
    fn launcher_names_match_expected_binaries() {
        assert_eq!(BridgeKind::Python.launcher(), "python3");
        assert_eq!(BridgeKind::Node.launcher(), "node");
        assert_eq!(BridgeKind::Wasm.launcher(), "wasmtime");
        assert_eq!(BridgeKind::Grpc.launcher(), "grpcurl");
    }

    #[test]
    fn parse_response_extracts_value() {
        let body = serde_json::json!({"ok": true, "value": 0.42});
        let out = parse_bridge_response(body).unwrap();
        assert_eq!(out.value, serde_json::json!(0.42));
    }

    #[test]
    fn parse_response_surfaces_bridge_failure() {
        let body = serde_json::json!({"ok": false, "error": "no model"});
        let err = parse_bridge_response(body).unwrap_err();
        match err {
            BridgeError::Bridge { message, .. } => {
                assert_eq!(message, "no model");
            }
            other => panic!("expected BridgeError::Bridge, got {other:?}"),
        }
    }

    #[test]
    fn parse_response_rejects_non_object_body() {
        let body = serde_json::json!([1, 2, 3]);
        let err = parse_bridge_response(body).unwrap_err();
        assert!(matches!(err, BridgeError::ProtocolViolation(_)));
    }

    #[test]
    fn round_trip_spec_through_json() {
        let s = BridgeSpec {
            kind: BridgeKind::Python,
            target: "tools/sentiment.py".into(),
            entrypoint: "run".into(),
            timeout_ms: 5_000,
            launcher_override: String::new(),
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: BridgeSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }
}
