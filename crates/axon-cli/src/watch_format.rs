//! §35.2 — `axon watch` line formatter.
//!
//! Pure functions that turn a `TraceSpan` into the one-line summary
//! shown by `axon watch`. Kept in its own module so it's unit-testable
//! without the JSON-RPC / tty plumbing. Color escape codes are emitted
//! conditionally; the CLI passes `use_color = stderr is a TTY`.

use axon_runtime::{AttributeValue, SpanKind, TraceSpan};
use std::time::{SystemTime, UNIX_EPOCH};

/// Format one closed span as a single line for the watch viewer.
/// `start_anchor_ms` is the wall-clock time of the watch session
/// start; we display relative offsets like `00:01.215` so the user
/// reads "1 second into the run, this happened."
pub fn format_span(
    span: &TraceSpan,
    start_anchor_ms: u128,
    use_color: bool,
) -> String {
    let (cyan, gray, red, yellow, green, reset) = if use_color {
        ("\x1b[36m", "\x1b[90m", "\x1b[31m", "\x1b[33m", "\x1b[32m", "\x1b[0m")
    } else {
        ("", "", "", "", "", "")
    };

    let rel_ms = span.start_ms.saturating_sub(start_anchor_ms);
    let secs = rel_ms / 1000;
    let frac = rel_ms % 1000;
    let mins = secs / 60;
    let rem_secs = secs % 60;
    let stamp = format!("{mins:02}:{rem_secs:02}.{frac:03}");

    let dur = span
        .end_ms
        .map(|e| e.saturating_sub(span.start_ms))
        .unwrap_or(0);

    let status_color = if span.error.is_some() { red } else { green };
    let status_label = if span.error.is_some() { "err" } else { "ok " };
    let kind_label = span.kind.as_str();

    // Build the attribute suffix — show the few we care about: model,
    // tokens, cost_usd. Others are visible via `--trace PATH`.
    let mut attrs = Vec::new();
    if let Some(v) = span.attributes.get("model") {
        attrs.push(format!("model={}", attr_value_str(v)));
    }
    if let Some(v) = span.attributes.get("tokens") {
        attrs.push(format!("{}tok", attr_value_str(v)));
    }
    if let Some(v) = span.attributes.get("cost_usd") {
        attrs.push(format!("${}", attr_value_str(v)));
    }
    let attrs_str = if attrs.is_empty() {
        String::new()
    } else {
        format!("  {gray}{}{reset}", attrs.join(" · "))
    };

    let err_suffix = if let Some(msg) = &span.error {
        format!("  {red}[error: {msg}]{reset}")
    } else {
        String::new()
    };

    // §35.6 verification fix M7 — pad each column to a fixed width so
    // the banner header columns actually line up with row output.
    // Names longer than 35 chars overflow (and push the rest of the
    // line rightward) — that's acceptable; truncation would hide info.
    format!(
        "{gray}{stamp}{reset}  {cyan}{kind_label:<10}{reset} {cyan}{:<35}{reset}  {status_color}{status_label}{reset}  {yellow}{dur:>6}ms{reset}{attrs_str}{err_suffix}",
        span.name
    )
}

fn attr_value_str(v: &AttributeValue) -> String {
    match v {
        AttributeValue::String(s) => s.clone(),
        AttributeValue::Int(n) => n.to_string(),
        AttributeValue::Float(f) => format!("{f:.4}"),
        AttributeValue::Bool(b) => b.to_string(),
    }
}

/// Wall-clock now in milliseconds since the Unix epoch — used as the
/// `start_anchor_ms` for a watch session.
pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn span(name: &str, kind: SpanKind, start_ms: u128, end_ms: u128) -> TraceSpan {
        TraceSpan {
            id: 0,
            parent_id: None,
            name: name.to_string(),
            kind,
            start_ms,
            end_ms: Some(end_ms),
            attributes: HashMap::new(),
            error: None,
        }
    }

    #[test]
    fn formats_basic_span_with_zero_anchor() {
        let s = span("greet", SpanKind::AgentHandler, 1_215, 1_320);
        let line = format_span(&s, 0, false);
        assert!(line.contains("00:01.215"), "{line}");
        assert!(line.contains("agent.handler"), "{line}");
        assert!(line.contains("greet"), "{line}");
        assert!(line.contains("ok"), "{line}");
        assert!(line.contains("105ms"), "{line}");
    }

    #[test]
    fn relative_offset_uses_anchor() {
        let s = span("ask", SpanKind::Ask, 10_500, 10_510);
        let line = format_span(&s, 10_000, false);
        assert!(line.contains("00:00.500"), "{line}");
    }

    #[test]
    fn shows_err_marker_when_span_carries_an_error() {
        let mut s = span("ask", SpanKind::Ask, 10, 20);
        s.error = Some("rate limited".into());
        let line = format_span(&s, 0, false);
        assert!(line.contains("err"), "{line}");
        assert!(line.contains("[error: rate limited]"), "{line}");
    }

    #[test]
    fn shows_model_tokens_cost_attrs_when_present() {
        let mut s = span("ask", SpanKind::Ask, 0, 100);
        s.attributes
            .insert("model".into(), AttributeValue::String("claude".into()));
        s.attributes
            .insert("tokens".into(), AttributeValue::Int(425));
        s.attributes
            .insert("cost_usd".into(), AttributeValue::Float(0.0072));
        let line = format_span(&s, 0, false);
        assert!(line.contains("model=claude"), "{line}");
        assert!(line.contains("425tok"), "{line}");
        assert!(line.contains("$0.0072"), "{line}");
    }

    #[test]
    fn no_color_codes_when_color_disabled() {
        let s = span("x", SpanKind::UserScope, 0, 1);
        let line = format_span(&s, 0, false);
        assert!(!line.contains("\x1b["), "{line}");
    }

    #[test]
    fn color_codes_present_when_color_enabled() {
        let s = span("x", SpanKind::UserScope, 0, 1);
        let line = format_span(&s, 0, true);
        assert!(line.contains("\x1b["), "{line}");
    }
}
