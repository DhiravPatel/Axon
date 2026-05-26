//! Protocol-shaped serve adapters (§35.3).
//!
//! `axon serve --protocol mcp | openai | grpc | a2a` exposes the same
//! agent under a different wire shape:
//!
//!   * `mcp`    — JSON-RPC 2.0 over HTTP at `POST /`. Handles
//!                `tools/list` and `tools/call`. Useful for editors
//!                that already speak Model Context Protocol.
//!   * `openai` — OpenAI-compatible chat endpoint at
//!                `POST /v1/chat/completions`. Translates `messages[]`
//!                into a single user prompt and wraps the agent's
//!                reply as a `choices[0].message` payload.
//!   * `grpc`   — emits `agents.proto` next to the deploy bundle. The
//!                actual HTTP/2 server is operator-supplied (the proto
//!                is enough to wire up `tonic`/`grpc-go`/etc.).
//!   * `a2a`    — serves `/.well-known/agent-card.json` (Stage 25
//!                auto-publish) and dispatches `POST /agent` to the
//!                handler.
//!
//! This module ships **pure functions** that take an incoming HTTP
//! request (`method`, `path`, `body`) and produce either:
//!
//!   * `ProtocolAction::Reply(status, body)` — return this directly to
//!     the client (e.g. `/tools/list`, malformed input).
//!   * `ProtocolAction::Dispatch { handler, prompt }` — pass the
//!     translated prompt to the agent handler; the adapter wraps the
//!     handler's reply into the wire-protocol response.
//!
//! Keeping the protocol logic pure lets us test it without a running
//! server and lets the existing `axon-deploy::Server` reuse one
//! request/response loop across every protocol.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServeProtocol {
    /// The default — `POST /invoke` with raw body, mirrors Stage 17.
    Plain,
    Mcp,
    Openai,
    Grpc,
    A2a,
}

impl ServeProtocol {
    pub fn from_flag(s: &str) -> Option<Self> {
        Some(match s {
            "plain" => ServeProtocol::Plain,
            "mcp" => ServeProtocol::Mcp,
            "openai" => ServeProtocol::Openai,
            "grpc" => ServeProtocol::Grpc,
            "a2a" => ServeProtocol::A2a,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProtocolAction {
    /// Return this response unchanged.
    Reply { status: u16, body: String, content_type: String },
    /// Route to the agent handler with the supplied prompt; wrap the
    /// reply per the protocol on the way back.
    Dispatch { handler: String, prompt: String, jsonrpc_id: serde_json::Value },
}

#[derive(Clone, Debug, PartialEq)]
pub struct IncomingRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub body: &'a str,
}

/// Route an incoming request under the configured protocol.
///
/// `well_known_card_body` is the body returned for
/// `/.well-known/agent-card.json` under the `a2a` protocol; pass an
/// empty string when not in `a2a` mode.
pub fn route(
    proto: ServeProtocol,
    req: &IncomingRequest<'_>,
    default_handler: &str,
    well_known_card_body: &str,
) -> ProtocolAction {
    match proto {
        ServeProtocol::Plain => route_plain(req, default_handler),
        ServeProtocol::Mcp => route_mcp(req),
        ServeProtocol::Openai => route_openai(req, default_handler),
        ServeProtocol::Grpc => route_grpc(req, default_handler),
        ServeProtocol::A2a => route_a2a(req, default_handler, well_known_card_body),
    }
}

fn route_plain(req: &IncomingRequest<'_>, handler: &str) -> ProtocolAction {
    if req.method == "POST" && req.path == "/invoke" {
        return ProtocolAction::Dispatch {
            handler: handler.into(),
            prompt: req.body.to_string(),
            jsonrpc_id: serde_json::Value::Null,
        };
    }
    ProtocolAction::Reply {
        status: 404,
        body: "not found".into(),
        content_type: "text/plain".into(),
    }
}

fn route_mcp(req: &IncomingRequest<'_>) -> ProtocolAction {
    if req.method != "POST" {
        return jsonrpc_error(-32600, "method not allowed", serde_json::Value::Null);
    }
    let body: serde_json::Value = match serde_json::from_str(req.body) {
        Ok(v) => v,
        Err(e) => return jsonrpc_error(-32700, &format!("parse error: {e}"), serde_json::Value::Null),
    };
    let id = body.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
    match method {
        "tools/list" => ProtocolAction::Reply {
            status: 200,
            body: json_rpc_ok(&id, serde_json::json!({"tools": []})),
            content_type: "application/json".into(),
        },
        "tools/call" => {
            let tool_name = body
                .get("params")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let args = body
                .get("params")
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            ProtocolAction::Dispatch {
                handler: tool_name,
                prompt: args.to_string(),
                jsonrpc_id: id,
            }
        }
        other => jsonrpc_error(
            -32601,
            &format!("method not found: {other}"),
            id,
        ),
    }
}

fn route_openai(req: &IncomingRequest<'_>, handler: &str) -> ProtocolAction {
    if req.method != "POST" || req.path != "/v1/chat/completions" {
        return ProtocolAction::Reply {
            status: 404,
            body: "{\"error\":\"unknown route\"}".into(),
            content_type: "application/json".into(),
        };
    }
    let body: serde_json::Value = match serde_json::from_str(req.body) {
        Ok(v) => v,
        Err(e) => {
            return ProtocolAction::Reply {
                status: 400,
                body: format!("{{\"error\":\"parse: {e}\"}}"),
                content_type: "application/json".into(),
            }
        }
    };
    let prompt = body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                    let content = m.get("content").and_then(|v| v.as_str())?;
                    Some(format!("{role}: {content}"))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    ProtocolAction::Dispatch {
        handler: handler.into(),
        prompt,
        jsonrpc_id: serde_json::Value::Null,
    }
}

fn route_grpc(req: &IncomingRequest<'_>, handler: &str) -> ProtocolAction {
    // HTTP-fronted gRPC: tonic / envoy translate proto messages into
    // HTTP/2 framing before they reach us; for v0 we accept plain JSON
    // on `POST /<service>/<method>` so the protocol shape is
    // exercisable without a full HTTP/2 stack.
    if req.method != "POST" {
        return ProtocolAction::Reply {
            status: 405,
            body: "grpc: method not allowed".into(),
            content_type: "text/plain".into(),
        };
    }
    let _ = handler;
    let segments: Vec<&str> = req.path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() != 2 {
        return ProtocolAction::Reply {
            status: 400,
            body: "grpc: path must be /Service/Method".into(),
            content_type: "text/plain".into(),
        };
    }
    let svc_method = format!("{}.{}", segments[0], segments[1]);
    ProtocolAction::Dispatch {
        handler: svc_method,
        prompt: req.body.to_string(),
        jsonrpc_id: serde_json::Value::Null,
    }
}

fn route_a2a(
    req: &IncomingRequest<'_>,
    handler: &str,
    well_known_card_body: &str,
) -> ProtocolAction {
    if req.method == "GET" && req.path == "/.well-known/agent-card.json" {
        return ProtocolAction::Reply {
            status: 200,
            body: well_known_card_body.to_string(),
            content_type: "application/json".into(),
        };
    }
    if req.method == "POST" && req.path == "/agent" {
        return ProtocolAction::Dispatch {
            handler: handler.into(),
            prompt: req.body.to_string(),
            jsonrpc_id: serde_json::Value::Null,
        };
    }
    ProtocolAction::Reply {
        status: 404,
        body: "a2a: unknown route (expected GET /.well-known/agent-card.json or POST /agent)".into(),
        content_type: "text/plain".into(),
    }
}

/// Wrap a handler reply into the wire-protocol response.
pub fn wrap_response(proto: ServeProtocol, handler_reply: &str, jsonrpc_id: &serde_json::Value) -> (u16, String, &'static str) {
    match proto {
        ServeProtocol::Plain | ServeProtocol::A2a => {
            (200, handler_reply.to_string(), "application/json")
        }
        ServeProtocol::Mcp => (
            200,
            json_rpc_ok(
                jsonrpc_id,
                serde_json::json!({
                    "content": [{"type": "text", "text": handler_reply}]
                }),
            ),
            "application/json",
        ),
        ServeProtocol::Openai => {
            let body = serde_json::json!({
                "id": "chatcmpl-axon",
                "object": "chat.completion",
                "model": "axon-agent",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": handler_reply},
                    "finish_reason": "stop"
                }]
            });
            (200, body.to_string(), "application/json")
        }
        ServeProtocol::Grpc => (200, handler_reply.to_string(), "application/json"),
    }
}

fn jsonrpc_error(code: i64, message: &str, id: serde_json::Value) -> ProtocolAction {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "error": {"code": code, "message": message},
        "id": id,
    });
    ProtocolAction::Reply {
        status: 200,
        body: body.to_string(),
        content_type: "application/json".into(),
    }
}

fn json_rpc_ok(id: &serde_json::Value, result: serde_json::Value) -> String {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "result": result,
        "id": id,
    });
    body.to_string()
}

/// Render the `.proto` file for a list of handler names so callers can
/// drop it next to their deploy bundle.
pub fn render_grpc_proto(service_name: &str, handlers: &[String]) -> String {
    let mut out = String::new();
    out.push_str("syntax = \"proto3\";\n");
    out.push_str("package axon;\n\n");
    out.push_str("message JsonPayload { string body = 1; }\n\n");
    out.push_str(&format!("service {service_name} {{\n"));
    for h in handlers {
        out.push_str(&format!(
            "  rpc {h}(JsonPayload) returns (JsonPayload);\n"
        ));
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req<'a>(method: &'a str, path: &'a str, body: &'a str) -> IncomingRequest<'a> {
        IncomingRequest { method, path, body }
    }

    #[test]
    fn from_flag_round_trips() {
        for s in ["plain", "mcp", "openai", "grpc", "a2a"] {
            assert!(ServeProtocol::from_flag(s).is_some(), "{s}");
        }
        assert!(ServeProtocol::from_flag("websocket").is_none());
    }

    #[test]
    fn plain_dispatches_on_invoke_only() {
        let r = req("POST", "/invoke", "hello");
        match route(ServeProtocol::Plain, &r, "main", "") {
            ProtocolAction::Dispatch { handler, prompt, .. } => {
                assert_eq!(handler, "main");
                assert_eq!(prompt, "hello");
            }
            other => panic!("expected Dispatch, got {other:?}"),
        }
        let r2 = req("POST", "/elsewhere", "hi");
        match route(ServeProtocol::Plain, &r2, "main", "") {
            ProtocolAction::Reply { status, .. } => assert_eq!(status, 404),
            _ => panic!("expected 404"),
        }
    }

    #[test]
    fn mcp_tools_list_returns_empty_array_for_now() {
        let body = r#"{"jsonrpc":"2.0","method":"tools/list","id":1}"#;
        let r = req("POST", "/", body);
        match route(ServeProtocol::Mcp, &r, "main", "") {
            ProtocolAction::Reply { status, body, .. } => {
                assert_eq!(status, 200);
                assert!(body.contains("\"tools\":[]"));
                assert!(body.contains("\"id\":1"));
            }
            _ => panic!("expected Reply"),
        }
    }

    #[test]
    fn mcp_tools_call_dispatches_with_tool_name() {
        let body = r#"{"jsonrpc":"2.0","method":"tools/call","id":7,"params":{"name":"sentiment","arguments":{"text":"hi"}}}"#;
        let r = req("POST", "/", body);
        match route(ServeProtocol::Mcp, &r, "main", "") {
            ProtocolAction::Dispatch { handler, prompt, jsonrpc_id } => {
                assert_eq!(handler, "sentiment");
                assert!(prompt.contains("\"text\":\"hi\""));
                assert_eq!(jsonrpc_id, serde_json::json!(7));
            }
            _ => panic!("expected Dispatch"),
        }
    }

    #[test]
    fn mcp_unknown_method_returns_jsonrpc_error() {
        let body = r#"{"jsonrpc":"2.0","method":"nope","id":2}"#;
        let r = req("POST", "/", body);
        match route(ServeProtocol::Mcp, &r, "main", "") {
            ProtocolAction::Reply { body, .. } => {
                assert!(body.contains("method not found"));
                assert!(body.contains("\"code\":-32601"));
            }
            _ => panic!("expected Reply"),
        }
    }

    #[test]
    fn openai_chat_translates_messages_to_prompt() {
        let body = r#"{"messages":[{"role":"system","content":"be concise"},{"role":"user","content":"hi"}]}"#;
        let r = req("POST", "/v1/chat/completions", body);
        match route(ServeProtocol::Openai, &r, "main", "") {
            ProtocolAction::Dispatch { prompt, .. } => {
                assert!(prompt.contains("system: be concise"));
                assert!(prompt.contains("user: hi"));
            }
            _ => panic!("expected Dispatch"),
        }
    }

    #[test]
    fn openai_wrap_response_emits_choices() {
        let (status, body, ct) = wrap_response(ServeProtocol::Openai, "the answer", &serde_json::Value::Null);
        assert_eq!(status, 200);
        assert_eq!(ct, "application/json");
        assert!(body.contains("\"role\":\"assistant\""));
        assert!(body.contains("\"content\":\"the answer\""));
    }

    #[test]
    fn a2a_well_known_returns_card_body() {
        let r = req("GET", "/.well-known/agent-card.json", "");
        match route(ServeProtocol::A2a, &r, "main", "{\"name\":\"R\"}") {
            ProtocolAction::Reply { body, status, .. } => {
                assert_eq!(status, 200);
                assert_eq!(body, "{\"name\":\"R\"}");
            }
            _ => panic!("expected Reply"),
        }
    }

    #[test]
    fn a2a_post_agent_dispatches() {
        let r = req("POST", "/agent", "hi");
        match route(ServeProtocol::A2a, &r, "main", "") {
            ProtocolAction::Dispatch { handler, prompt, .. } => {
                assert_eq!(handler, "main");
                assert_eq!(prompt, "hi");
            }
            _ => panic!("expected Dispatch"),
        }
    }

    #[test]
    fn grpc_path_parses_service_method() {
        let r = req("POST", "/Research/Triage", "{}");
        match route(ServeProtocol::Grpc, &r, "main", "") {
            ProtocolAction::Dispatch { handler, .. } => {
                assert_eq!(handler, "Research.Triage");
            }
            _ => panic!("expected Dispatch"),
        }
    }

    #[test]
    fn render_grpc_proto_lists_every_handler() {
        let p = render_grpc_proto("Support", &["Triage".into(), "Resolve".into()]);
        assert!(p.contains("service Support {"));
        assert!(p.contains("rpc Triage(JsonPayload) returns (JsonPayload);"));
        assert!(p.contains("rpc Resolve(JsonPayload) returns (JsonPayload);"));
    }
}
