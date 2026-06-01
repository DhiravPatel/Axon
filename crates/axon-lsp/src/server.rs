//! LSP message loop — drives JSON-RPC over stdio and dispatches each
//! request to the analysis layer.

use std::collections::HashMap;

use axon_diag::Severity;
use lsp_server::{Connection, ExtractError, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    notification::{DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics},
    request::{CodeLensRequest, Completion, GotoDefinition, HoverRequest, Initialize, Request as _, Shutdown},
    CodeLens, CodeLensOptions, CodeLensParams, Command,
    CompletionItem as LspCompletionItem, CompletionItemKind, CompletionList, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeResult, Location, MarkupContent, MarkupKind, OneOf, PublishDiagnosticsParams, ServerCapabilities,
    ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};

use crate::{analyze, query};

const SERVER_NAME: &str = "axon-lsp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run the LSP server on stdin/stdout. Blocks until the editor sends
/// `exit`.
pub fn run() -> std::io::Result<()> {
    let (connection, io_threads) = Connection::stdio();

    let server_caps = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::FULL,
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".into(), ":".into()]),
            ..Default::default()
        }),
        code_lens_provider: Some(CodeLensOptions {
            resolve_provider: Some(false),
        }),
        ..Default::default()
    })
    .expect("server capabilities serialize");

    let initialize_id = match connection.initialize_start() {
        Ok((id, _params)) => id,
        Err(e) => {
            io_threads.join().ok();
            return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
        }
    };

    let init_result = InitializeResult {
        capabilities: serde_json::from_value(server_caps).unwrap(),
        server_info: Some(ServerInfo {
            name: SERVER_NAME.into(),
            version: Some(SERVER_VERSION.into()),
        }),
    };
    if let Err(e) = connection.initialize_finish(initialize_id, serde_json::to_value(init_result).unwrap()) {
        io_threads.join().ok();
        return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
    }

    let mut docs: HashMap<Url, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req).unwrap_or(false) {
                    break;
                }
                let resp = handle_request(req, &docs);
                let _ = connection.sender.send(Message::Response(resp));
            }
            Message::Notification(note) => {
                handle_notification(note, &connection, &mut docs);
            }
            Message::Response(_) => {
                // Server-initiated requests aren't used yet.
            }
        }
    }

    io_threads.join().ok();
    Ok(())
}

fn handle_request(req: Request, docs: &HashMap<Url, String>) -> Response {
    let id = req.id.clone();
    let method = req.method.clone();
    match method.as_str() {
        HoverRequest::METHOD => {
            let result = extract::<HoverParams>(req).and_then(|(id, p)| hover(id, p, docs));
            into_resp(id, result)
        }
        GotoDefinition::METHOD => {
            let result = extract::<GotoDefinitionParams>(req)
                .and_then(|(id, p)| definition(id, p, docs));
            into_resp(id, result)
        }
        Completion::METHOD => {
            let result = extract::<CompletionParams>(req).and_then(|(id, p)| completion(id, p, docs));
            into_resp(id, result)
        }
        CodeLensRequest::METHOD => {
            let result = extract::<CodeLensParams>(req).and_then(|(id, p)| code_lens(id, p, docs));
            into_resp(id, result)
        }
        Initialize::METHOD => {
            // We already handled initialize at startup. Editors shouldn't
            // re-issue it but we reply with an empty success just in case.
            Response::new_ok(id, serde_json::Value::Null)
        }
        Shutdown::METHOD => Response::new_ok(id, serde_json::Value::Null),
        _ => Response::new_err(
            id,
            lsp_server::ErrorCode::MethodNotFound as i32,
            format!("axon-lsp: method `{method}` not implemented"),
        ),
    }
}

fn extract<P: serde::de::DeserializeOwned>(
    req: Request,
) -> Result<(RequestId, P), Response> {
    let id = req.id.clone();
    let method = req.method.clone();
    match req.extract::<P>(method.as_str()) {
        Ok((id, params)) => Ok((id, params)),
        Err(ExtractError::JsonError { method, error }) => Err(Response::new_err(
            id,
            lsp_server::ErrorCode::InvalidParams as i32,
            format!("axon-lsp: failed to parse params for `{method}`: {error}"),
        )),
        Err(ExtractError::MethodMismatch(req)) => Err(Response::new_err(
            req.id,
            lsp_server::ErrorCode::MethodNotFound as i32,
            "axon-lsp: method mismatch".into(),
        )),
    }
}

fn into_resp<T: serde::Serialize>(
    id: RequestId,
    result: Result<(RequestId, Option<T>), Response>,
) -> Response {
    match result {
        Ok((id, payload)) => {
            let v = payload
                .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null))
                .unwrap_or(serde_json::Value::Null);
            Response::new_ok(id, v)
        }
        Err(err) => Response {
            id,
            ..err
        },
    }
}

// ---------------------------------------------------------------------------
// Request handlers — all return Option<Result>.
// ---------------------------------------------------------------------------

fn hover(
    id: RequestId,
    params: HoverParams,
    docs: &HashMap<Url, String>,
) -> Result<(RequestId, Option<Hover>), Response> {
    let uri = params.text_document_position_params.text_document.uri.clone();
    let text = match docs.get(&uri) {
        Some(t) => t.clone(),
        None => return Ok((id, None)),
    };
    let pos = params.text_document_position_params.position;
    let offset = crate::position::position_to_offset(&text, pos);
    let analysis = analyze(uri.as_str(), &text);
    let info = query::hover_at_offset(&analysis, offset);
    Ok((
        id,
        info.map(|info| build_hover(&analysis, &info, &text)),
    ))
}

fn build_hover(analysis: &Analysis, info: &query::HoverInfo, text: &str) -> Hover {
    let (msg, span) = match info {
        query::HoverInfo::NameRef { name, span } => {
            let body = match analysis.ctx.lookup(name).and_then(|id| analysis.ctx.get(id)) {
                Some(sig) => format!(
                    "**{name}** — {}",
                    crate::query::completions(analysis)
                        .iter()
                        .find(|c| &c.label == name)
                        .and_then(|c| c.detail.clone())
                        .unwrap_or_else(|| format!("{:?}", std::mem::discriminant(&sig.kind)))
                ),
                None => format!("**{name}** — _not in scope_"),
            };
            (body, *span)
        }
        query::HoverInfo::ItemDecl { name, kind, span } => {
            (format!("**{name}** — {kind}"), *span)
        }
        query::HoverInfo::Literal { ty, span } => (format!("_{ty}_ literal"), *span),
    };
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: msg,
        }),
        range: Some(crate::position::span_to_range(text, span)),
    }
}

use crate::Analysis;

fn definition(
    id: RequestId,
    params: GotoDefinitionParams,
    docs: &HashMap<Url, String>,
) -> Result<(RequestId, Option<GotoDefinitionResponse>), Response> {
    let uri = params.text_document_position_params.text_document.uri.clone();
    let text = match docs.get(&uri) {
        Some(t) => t.clone(),
        None => return Ok((id, None)),
    };
    let pos = params.text_document_position_params.position;
    let offset = crate::position::position_to_offset(&text, pos);
    let analysis = analyze(uri.as_str(), &text);
    let info = match query::hover_at_offset(&analysis, offset) {
        Some(i) => i,
        None => return Ok((id, None)),
    };
    let span = match query::definition_for(&analysis, &info) {
        Some(s) => s,
        None => return Ok((id, None)),
    };
    let location = Location {
        uri,
        range: crate::position::span_to_range(&text, span),
    };
    Ok((id, Some(GotoDefinitionResponse::Scalar(location))))
}

fn completion(
    id: RequestId,
    params: CompletionParams,
    docs: &HashMap<Url, String>,
) -> Result<(RequestId, Option<CompletionResponse>), Response> {
    let uri = params.text_document_position.text_document.uri.clone();
    let text = match docs.get(&uri) {
        Some(t) => t.clone(),
        None => return Ok((id, None)),
    };
    let analysis = analyze(uri.as_str(), &text);
    let items: Vec<LspCompletionItem> = query::completions(&analysis)
        .into_iter()
        .map(|c| LspCompletionItem {
            label: c.label,
            kind: Some(CompletionItemKind::FUNCTION),
            detail: c.detail,
            ..Default::default()
        })
        .collect();
    Ok((
        id,
        Some(CompletionResponse::List(CompletionList {
            is_incomplete: false,
            items,
        })),
    ))
}

/// §32 LSP cost lens — emits a non-clickable code lens above each
/// `ask` / `generate` / `plan` showing an *estimated* per-call cost,
/// latency, and token counts. No API call is performed; the estimate is
/// derived purely from prompt source-text length. See `cost_lens`
/// module docs for the heuristic. The `Command` carries an empty
/// command string so editors render the title as an inline label only
/// (clicking is a no-op — by design).
fn code_lens(
    id: RequestId,
    params: CodeLensParams,
    docs: &HashMap<Url, String>,
) -> Result<(RequestId, Option<Vec<CodeLens>>), Response> {
    let uri = params.text_document.uri.clone();
    let text = match docs.get(&uri) {
        Some(t) => t.clone(),
        None => return Ok((id, None)),
    };
    let analysis = analyze(uri.as_str(), &text);
    let lenses = crate::cost_lens::lenses_for(&analysis.program)
        .into_iter()
        .map(|l| CodeLens {
            range: crate::position::span_to_range(&text, l.span),
            command: Some(Command {
                title: l.label,
                command: String::new(),
                arguments: None,
            }),
            data: None,
        })
        .collect();
    Ok((id, Some(lenses)))
}

// ---------------------------------------------------------------------------
// Notification handlers
// ---------------------------------------------------------------------------

fn handle_notification(
    note: Notification,
    connection: &Connection,
    docs: &mut HashMap<Url, String>,
) {
    match note.method.as_str() {
        DidOpenTextDocument::METHOD => {
            if let Ok(p) =
                serde_json::from_value::<lsp_types::DidOpenTextDocumentParams>(note.params)
            {
                docs.insert(p.text_document.uri.clone(), p.text_document.text.clone());
                publish_diagnostics_for(&p.text_document.uri, docs, connection);
            }
        }
        DidChangeTextDocument::METHOD => {
            if let Ok(p) =
                serde_json::from_value::<lsp_types::DidChangeTextDocumentParams>(note.params)
            {
                // We advertise FULL sync, so changes always include the
                // entire new document text.
                if let Some(change) = p.content_changes.into_iter().last() {
                    docs.insert(p.text_document.uri.clone(), change.text);
                    publish_diagnostics_for(&p.text_document.uri, docs, connection);
                }
            }
        }
        DidCloseTextDocument::METHOD => {
            if let Ok(p) =
                serde_json::from_value::<lsp_types::DidCloseTextDocumentParams>(note.params)
            {
                docs.remove(&p.text_document.uri);
                // Clear diagnostics on close so the editor doesn't leave
                // stale red squiggles around.
                let _ = connection.sender.send(Message::Notification(Notification {
                    method: PublishDiagnostics::METHOD.into(),
                    params: serde_json::to_value(PublishDiagnosticsParams {
                        uri: p.text_document.uri,
                        diagnostics: Vec::new(),
                        version: None,
                    })
                    .unwrap(),
                }));
            }
        }
        _ => {
            // Unknown notifications: silently ignore (LSP spec recommendation).
        }
    }
}

fn publish_diagnostics_for(uri: &Url, docs: &HashMap<Url, String>, connection: &Connection) {
    let text = match docs.get(uri) {
        Some(t) => t.clone(),
        None => return,
    };
    let analysis = analyze(uri.as_str(), &text);
    let diags: Vec<Diagnostic> = analysis
        .diagnostics
        .iter()
        .map(|d| to_lsp_diagnostic(d, &text))
        .collect();
    let _ = connection.sender.send(Message::Notification(Notification {
        method: PublishDiagnostics::METHOD.into(),
        params: serde_json::to_value(PublishDiagnosticsParams {
            uri: uri.clone(),
            diagnostics: diags,
            version: None,
        })
        .unwrap(),
    }));
}

pub fn to_lsp_diagnostic(d: &axon_diag::Diagnostic, text: &str) -> Diagnostic {
    let severity = match d.severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Note => DiagnosticSeverity::INFORMATION,
        Severity::Help => DiagnosticSeverity::HINT,
    };
    Diagnostic {
        range: crate::position::span_to_range(text, d.primary.span),
        severity: Some(severity),
        code: d
            .code
            .map(|c| lsp_types::NumberOrString::String(c.to_string())),
        source: Some(SERVER_NAME.to_string()),
        message: d.message.clone(),
        ..Default::default()
    }
}
