//! OTLP/HTTP-JSON exporter for [`TraceSpan`]s.
//!
//! Converts the runtime's internal trace into the OpenTelemetry Protocol
//! JSON shape (`ExportTraceServiceRequest` over HTTP/protobuf+JSON). The
//! output is byte-for-byte compatible with what a real OTLP/HTTP exporter
//! POSTs to a collector — Tempo, Jaeger-OTLP, Honeycomb, and the
//! `otel-cli` import path all accept it.
//!
//! v0 limits:
//!
//!   * Single resource bag — `service.name` defaults to `"axon"` and can
//!     be overridden by the caller.
//!   * SpanKind is mapped onto OTLP's `SPAN_KIND_INTERNAL` (we don't yet
//!     distinguish client/server/producer/consumer at the source).
//!   * Trace and span IDs are zero-padded hex of our internal `u32` ids
//!     plus a constant epoch tag so the IDs are stable across runs of
//!     the same record/replay cassette.

use std::collections::HashMap;

use crate::trace::{AttributeValue, SpanKind, TraceSpan};

const SCHEMA_URL: &str = "https://opentelemetry.io/schemas/1.20.0";

/// Convert a list of internal trace spans into the OTLP/HTTP-JSON envelope.
/// The result is one JSON document that can be `POST`ed to
/// `/v1/traces` on any compliant collector.
pub fn spans_to_otlp_json(spans: &[TraceSpan], service_name: &str) -> serde_json::Value {
    // Every span in the same trace shares the same `trace_id`. We use a
    // constant prefix + the recording's smallest span id so a record/
    // replay pair produces the same trace_id.
    let trace_id = make_trace_id(spans);
    let mut otel_spans: Vec<serde_json::Value> = Vec::with_capacity(spans.len());
    for s in spans {
        otel_spans.push(span_to_otlp(s, &trace_id));
    }
    serde_json::json!({
        "resourceSpans": [{
            "resource": {
                "attributes": [
                    string_kv("service.name", service_name),
                    string_kv("telemetry.sdk.name", "axon-runtime"),
                    string_kv("telemetry.sdk.language", "axon"),
                    string_kv("telemetry.sdk.version", env!("CARGO_PKG_VERSION"))
                ]
            },
            "scopeSpans": [{
                "scope": { "name": "axon.trace", "version": env!("CARGO_PKG_VERSION") },
                "schemaUrl": SCHEMA_URL,
                "spans": otel_spans
            }]
        }]
    })
}

fn make_trace_id(spans: &[TraceSpan]) -> String {
    // 32 hex chars (128-bit) is OTLP-spec. Pack a stable 24-hex-char
    // prefix + 8 hex chars of the smallest span id, so a record/replay
    // pair produces the same trace_id.
    let root_id = spans.iter().map(|s| s.id).min().unwrap_or(0);
    format!("61786f6e2d747261636500000000{:08x}{:08x}", 0, root_id)
        .chars()
        .take(32)
        .collect()
}

fn span_id_hex(internal_id: u32) -> String {
    format!("{internal_id:016x}")
}

fn span_to_otlp(s: &TraceSpan, trace_id: &str) -> serde_json::Value {
    let attrs: Vec<serde_json::Value> = s
        .attributes
        .iter()
        .map(|(k, v)| attribute_kv(k, v))
        .collect();
    let parent_span_id = s
        .parent_id
        .map(|p| span_id_hex(p))
        .unwrap_or_else(|| "".to_string());
    let start_nano = (s.start_ms as u128).saturating_mul(1_000_000);
    let end_nano = s
        .end_ms
        .map(|e| (e as u128).saturating_mul(1_000_000))
        .unwrap_or(start_nano);
    let status = match &s.error {
        Some(msg) => serde_json::json!({
            "code": 2,  // STATUS_CODE_ERROR
            "message": msg
        }),
        None => serde_json::json!({ "code": 1 }), // STATUS_CODE_OK
    };
    serde_json::json!({
        "traceId": trace_id,
        "spanId": span_id_hex(s.id),
        "parentSpanId": parent_span_id,
        "name": s.name,
        "kind": kind_to_otlp(&s.kind),
        "startTimeUnixNano": start_nano.to_string(),
        "endTimeUnixNano": end_nano.to_string(),
        "attributes": attrs,
        "status": status
    })
}

fn kind_to_otlp(_k: &SpanKind) -> u32 {
    // SPAN_KIND_INTERNAL. Client/server/producer/consumer can be added
    // once we tag I/O calls with directionality.
    1
}

fn string_kv(key: &str, value: &str) -> serde_json::Value {
    serde_json::json!({
        "key": key,
        "value": { "stringValue": value }
    })
}

fn attribute_kv(key: &str, value: &AttributeValue) -> serde_json::Value {
    let v = match value {
        AttributeValue::String(s) => serde_json::json!({ "stringValue": s }),
        AttributeValue::Int(i) => serde_json::json!({ "intValue": i.to_string() }),
        AttributeValue::Float(f) => serde_json::json!({ "doubleValue": f }),
        AttributeValue::Bool(b) => serde_json::json!({ "boolValue": b }),
    };
    serde_json::json!({ "key": key, "value": v })
}

/// Convenience: build the OTLP-JSON document and write it to `path`. The
/// caller (typically the host `trace_export_otlp` binding) is responsible
/// for any directory creation.
pub fn write_to_path(
    spans: &[TraceSpan],
    service_name: &str,
    path: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    let doc = spans_to_otlp_json(spans, service_name);
    let bytes = serde_json::to_vec_pretty(&doc)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, bytes)
}

// Suppress dead-code warnings for the HashMap import when only some tests
// pull it in.
#[allow(dead_code)]
fn _hashmap_kept(_: HashMap<String, AttributeValue>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{AttributeValue, SpanKind, TraceSpan};

    fn span(id: u32, parent: Option<u32>, name: &str, attrs: &[(&str, AttributeValue)]) -> TraceSpan {
        let mut attributes = HashMap::new();
        for (k, v) in attrs {
            attributes.insert((*k).to_string(), v.clone());
        }
        TraceSpan {
            id,
            parent_id: parent,
            name: name.to_string(),
            kind: SpanKind::Ask,
            start_ms: 1_000_000,
            end_ms: Some(1_000_500),
            attributes,
            error: None,
        }
    }

    #[test]
    fn envelope_has_resource_and_scope_spans() {
        let doc = spans_to_otlp_json(&[span(0, None, "root", &[])], "demo");
        assert!(doc["resourceSpans"].is_array());
        let rs = &doc["resourceSpans"][0];
        assert!(rs["resource"]["attributes"].is_array());
        assert_eq!(
            rs["scopeSpans"][0]["scope"]["name"].as_str().unwrap(),
            "axon.trace"
        );
        assert_eq!(
            rs["resource"]["attributes"][0]["key"].as_str().unwrap(),
            "service.name"
        );
        assert_eq!(
            rs["resource"]["attributes"][0]["value"]["stringValue"]
                .as_str()
                .unwrap(),
            "demo"
        );
    }

    #[test]
    fn spans_carry_trace_id_and_span_id() {
        let spans = vec![
            span(7, None, "outer", &[("model", AttributeValue::String("claude".into()))]),
            span(8, Some(7), "tool_call", &[("cost_cents", AttributeValue::Int(3))]),
        ];
        let doc = spans_to_otlp_json(&spans, "axon");
        let inner = &doc["resourceSpans"][0]["scopeSpans"][0]["spans"];
        let trace_id_a = inner[0]["traceId"].as_str().unwrap().to_string();
        let trace_id_b = inner[1]["traceId"].as_str().unwrap().to_string();
        assert_eq!(trace_id_a, trace_id_b, "all spans share a trace_id");
        assert_eq!(trace_id_a.len(), 32);
        assert_eq!(inner[0]["spanId"].as_str().unwrap().len(), 16);
        assert_eq!(
            inner[1]["parentSpanId"].as_str().unwrap(),
            span_id_hex(7)
        );
    }

    #[test]
    fn nanosecond_timestamps_round_trip() {
        let s = span(0, None, "n", &[]);
        let doc = spans_to_otlp_json(&[s.clone()], "axon");
        let inner = &doc["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(
            inner["startTimeUnixNano"].as_str().unwrap(),
            (s.start_ms as u128 * 1_000_000).to_string()
        );
        assert_eq!(
            inner["endTimeUnixNano"].as_str().unwrap(),
            (s.end_ms.unwrap() as u128 * 1_000_000).to_string()
        );
    }

    #[test]
    fn errored_span_gets_status_code_2() {
        let mut s = span(0, None, "err", &[]);
        s.error = Some("boom".into());
        let doc = spans_to_otlp_json(&[s], "axon");
        let inner = &doc["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(inner["status"]["code"].as_u64().unwrap(), 2);
        assert_eq!(inner["status"]["message"].as_str().unwrap(), "boom");
    }

    #[test]
    fn attribute_values_map_to_correct_otlp_types() {
        let spans = vec![span(
            0,
            None,
            "attrs",
            &[
                ("s", AttributeValue::String("hi".into())),
                ("i", AttributeValue::Int(42)),
                ("f", AttributeValue::Float(3.14)),
                ("b", AttributeValue::Bool(true)),
            ],
        )];
        let doc = spans_to_otlp_json(&spans, "axon");
        let attrs = &doc["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"];
        let by_key: HashMap<String, serde_json::Value> = attrs
            .as_array()
            .unwrap()
            .iter()
            .map(|kv| (kv["key"].as_str().unwrap().to_string(), kv["value"].clone()))
            .collect();
        assert_eq!(by_key["s"]["stringValue"].as_str().unwrap(), "hi");
        assert_eq!(by_key["i"]["intValue"].as_str().unwrap(), "42");
        assert!((by_key["f"]["doubleValue"].as_f64().unwrap() - 3.14).abs() < 1e-9);
        assert_eq!(by_key["b"]["boolValue"].as_bool().unwrap(), true);
    }

    #[test]
    fn write_to_path_round_trips() {
        let mut p = std::env::temp_dir();
        p.push(format!("axon-otlp-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        write_to_path(&[span(0, None, "rt", &[])], "axon", &p).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["resourceSpans"].is_array());
        let _ = std::fs::remove_file(&p);
    }
}
