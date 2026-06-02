//! Runtime tracing.
//!
//! Every meaningful step the interpreter takes — a model call, a tool
//! invocation, an agent handler dispatch, an explicit `with span("...")` —
//! opens a [`TraceSpan`]. Spans nest by parent id; when the step finishes,
//! the span closes and gains end timestamps + summary attributes (token
//! usage, cost, error info).
//!
//! The tracer is *off by default* — the runtime only allocates a [`Tracer`]
//! when the caller explicitly enables it via [`Interpreter::set_tracer`].
//! Tests use the in-memory snapshot; the CLI writes JSON Lines to a file.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

/// §35.2 — streaming sink type. `axon watch` installs a closure that
/// fires every time a span closes; the closure prints the span to the
/// terminal in real time. `Send` so the host can spawn the printer on
/// a worker if it ever needs to.
pub type StreamSink = Box<dyn FnMut(&TraceSpan) + Send>;

/// In-memory tracing collector.
#[derive(Default)]
pub struct Tracer {
    spans: Vec<TraceSpan>,
    open: Vec<u32>,
    next_id: u32,
    /// §35.2 — optional streaming sink. When set, `close` invokes it
    /// with a clone of the just-closed span. The sink is INSTEAD-OF
    /// not IN-ADDITION-TO the buffered `spans` vec — both still
    /// populate so `axon watch` can also write `--trace PATH` at end
    /// of run for archival.
    on_close: Option<StreamSink>,
}

impl Tracer {
    pub fn new() -> Self {
        Self::default()
    }

    /// §35.2 — install a streaming sink. The sink fires every time a
    /// span closes, with a snapshot of the closed span (cloned to
    /// avoid borrow conflicts with `spans`/`open`).
    pub fn with_sink(mut self, sink: StreamSink) -> Self {
        self.on_close = Some(sink);
        self
    }

    /// Variant that takes `&mut self` so a caller can attach a sink to
    /// an already-allocated Tracer.
    pub fn set_sink(&mut self, sink: StreamSink) {
        self.on_close = Some(sink);
    }

    /// Open a new span as a child of the currently-open span (if any).
    /// Returns the span id for use with [`Self::close`].
    pub fn open(&mut self, name: impl Into<String>, kind: SpanKind) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let parent_id = self.open.last().copied();
        self.spans.push(TraceSpan {
            id,
            parent_id,
            name: name.into(),
            kind,
            start_ms: now_ms(),
            end_ms: None,
            attributes: HashMap::new(),
            error: None,
        });
        self.open.push(id);
        id
    }

    /// Record an attribute on the currently-open span at `id`. No-op if
    /// the span was already closed (which shouldn't happen if callers
    /// pair open/close correctly).
    pub fn attribute(&mut self, id: u32, key: impl Into<String>, value: AttributeValue) {
        if let Some(span) = self.spans.iter_mut().find(|s| s.id == id) {
            span.attributes.insert(key.into(), value);
        }
    }

    /// Close the span at `id`, recording its end time. Pops the open
    /// stack; in well-formed code the closed span is always the topmost.
    /// §35.2 — if a streaming sink is installed, fires it with a clone
    /// of the just-closed span (taking the sink out of `self` for the
    /// duration of the call so the sink itself can re-enter the tracer
    /// without a borrow conflict).
    pub fn close(&mut self, id: u32) {
        if let Some(span) = self.spans.iter_mut().find(|s| s.id == id) {
            span.end_ms = Some(now_ms());
        }
        if let Some(pos) = self.open.iter().rposition(|&i| i == id) {
            self.open.remove(pos);
        }
        if self.on_close.is_some() {
            // Clone the just-closed span so the sink doesn't need to
            // hold a borrow on `self.spans` (which would conflict with
            // any tracer mutation the sink could trigger).
            let closed = self
                .spans
                .iter()
                .find(|s| s.id == id)
                .cloned();
            if let Some(span) = closed {
                let mut sink = self.on_close.take().unwrap();
                sink(&span);
                self.on_close = Some(sink);
            }
        }
    }

    /// Record an error message on the span at `id` (typically a failed
    /// model call, tool exception, or handler panic). The error stays
    /// attached when the span closes.
    pub fn record_error(&mut self, id: u32, message: impl Into<String>) {
        if let Some(span) = self.spans.iter_mut().find(|s| s.id == id) {
            span.error = Some(message.into());
        }
    }

    pub fn spans(&self) -> &[TraceSpan] {
        &self.spans
    }

    /// Render every span as one JSON object per line — the standard format
    /// observability tools ingest.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for s in &self.spans {
            out.push_str(&s.to_json());
            out.push('\n');
        }
        out
    }
}

/// One trace span. Times are milliseconds since the Unix epoch; nesting is
/// expressed by parent_id back-references.
#[derive(Clone, Debug)]
pub struct TraceSpan {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub name: String,
    pub kind: SpanKind,
    pub start_ms: u128,
    pub end_ms: Option<u128>,
    pub attributes: HashMap<String, AttributeValue>,
    pub error: Option<String>,
}

impl TraceSpan {
    pub fn duration_ms(&self) -> Option<u128> {
        self.end_ms.map(|e| e.saturating_sub(self.start_ms))
    }

    pub fn to_json(&self) -> String {
        // Hand-rolled to keep this module dependency-light. Spans are
        // simple enough to serialize without bringing serde into
        // axon-runtime.
        let mut buf = String::new();
        buf.push('{');
        write_kv(&mut buf, "id", &serde_json_num(self.id as i64), false);
        write_kv(
            &mut buf,
            "parent_id",
            &self
                .parent_id
                .map(|p| serde_json_num(p as i64))
                .unwrap_or_else(|| "null".into()),
            true,
        );
        write_kv(&mut buf, "name", &serde_json_str(&self.name), true);
        write_kv(
            &mut buf,
            "kind",
            &serde_json_str(self.kind.as_str()),
            true,
        );
        write_kv(
            &mut buf,
            "start_ms",
            &self.start_ms.to_string(),
            true,
        );
        write_kv(
            &mut buf,
            "end_ms",
            &self
                .end_ms
                .map(|e| e.to_string())
                .unwrap_or_else(|| "null".into()),
            true,
        );
        write_kv(
            &mut buf,
            "duration_ms",
            &self
                .duration_ms()
                .map(|d| d.to_string())
                .unwrap_or_else(|| "null".into()),
            true,
        );
        // Attributes — sorted by key for stable output.
        let mut keys: Vec<&String> = self.attributes.keys().collect();
        keys.sort();
        buf.push_str(",\"attributes\":{");
        for (i, k) in keys.iter().enumerate() {
            if i > 0 {
                buf.push(',');
            }
            buf.push_str(&serde_json_str(k));
            buf.push(':');
            buf.push_str(&self.attributes[*k].to_json());
        }
        buf.push('}');
        if let Some(err) = &self.error {
            write_kv(&mut buf, "error", &serde_json_str(err), true);
        }
        buf.push('}');
        buf
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SpanKind {
    Ask,
    Plan,
    Generate,
    Tool,
    AgentHandler,
    AgentSpawn,
    UserScope,
}

impl SpanKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SpanKind::Ask => "ask",
            SpanKind::Plan => "plan",
            SpanKind::Generate => "generate",
            SpanKind::Tool => "tool",
            SpanKind::AgentHandler => "agent.handler",
            SpanKind::AgentSpawn => "agent.spawn",
            SpanKind::UserScope => "scope",
        }
    }
}

#[derive(Clone, Debug)]
pub enum AttributeValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl AttributeValue {
    fn to_json(&self) -> String {
        match self {
            AttributeValue::String(s) => serde_json_str(s),
            AttributeValue::Int(i) => i.to_string(),
            AttributeValue::Float(f) => {
                if f.is_finite() {
                    format!("{f}")
                } else {
                    "null".into()
                }
            }
            AttributeValue::Bool(b) => b.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn write_kv(buf: &mut String, key: &str, value: &str, leading_comma: bool) {
    if leading_comma {
        buf.push(',');
    }
    buf.push_str(&serde_json_str(key));
    buf.push(':');
    buf.push_str(value);
}

fn serde_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn serde_json_num(n: i64) -> String {
    n.to_string()
}

// The Rc/RefCell handle alias was removed; the interpreter wraps the
// tracer in a plain RefCell on its own struct.
#[allow(dead_code)]
fn _imports_kept(_: Rc<RefCell<Tracer>>) {}
