//! Record/replay for non-deterministic runtime operations.
//!
//! A recording captures every input that wasn't determined by the program
//! itself: model responses, `time_now()` results, random numbers. On
//! replay, the runtime returns the recorded values in order instead of
//! calling the real provider / clock / RNG. Programs that ran cleanly
//! against a captured recording re-execute *bit-exactly*.
//!
//! Storage format: a single JSON document with one event per step.
//! Compact enough to commit alongside source as a test fixture; not yet
//! versioned, but the outer object carries a "version" field so future
//! changes can stay backward-compatible.

use std::cell::RefCell;
use std::rc::Rc;

use axon_models::ChatResponse;

/// One non-deterministic observation captured during a recording run.
#[derive(Clone, Debug)]
pub enum RecordedEvent {
    ModelCall {
        provider: String,
        response: ChatResponse,
    },
    TimeNow {
        nanos: i64,
    },
    RandomInt {
        result: i64,
    },
    RandomFloat {
        result: f64,
    },
}

#[derive(Default)]
pub struct Recording {
    pub events: Vec<RecordedEvent>,
}

impl Recording {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, ev: RecordedEvent) {
        self.events.push(ev);
    }

    pub fn to_json(&self) -> serde_json::Value {
        let evs: Vec<serde_json::Value> = self
            .events
            .iter()
            .map(|e| match e {
                RecordedEvent::ModelCall { provider, response } => serde_json::json!({
                    "kind": "model_call",
                    "provider": provider,
                    "response": chat_response_to_json(response),
                }),
                RecordedEvent::TimeNow { nanos } => serde_json::json!({
                    "kind": "time_now",
                    "nanos": nanos,
                }),
                RecordedEvent::RandomInt { result } => serde_json::json!({
                    "kind": "random_int",
                    "result": result,
                }),
                RecordedEvent::RandomFloat { result } => serde_json::json!({
                    "kind": "random_float",
                    "result": result,
                }),
            })
            .collect();
        serde_json::json!({ "version": 1, "events": evs })
    }

    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let evs = v
            .get("events")
            .and_then(|e| e.as_array())
            .ok_or("recording: missing events array")?;
        let mut out = Recording::new();
        for ev in evs {
            let kind = ev.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            let parsed = match kind {
                "model_call" => RecordedEvent::ModelCall {
                    provider: ev
                        .get("provider")
                        .and_then(|p| p.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    response: chat_response_from_json(
                        ev.get("response").unwrap_or(&serde_json::Value::Null),
                    )?,
                },
                "time_now" => RecordedEvent::TimeNow {
                    nanos: ev.get("nanos").and_then(|n| n.as_i64()).unwrap_or(0),
                },
                "random_int" => RecordedEvent::RandomInt {
                    result: ev.get("result").and_then(|n| n.as_i64()).unwrap_or(0),
                },
                "random_float" => RecordedEvent::RandomFloat {
                    result: ev.get("result").and_then(|n| n.as_f64()).unwrap_or(0.0),
                },
                other => return Err(format!("recording: unknown event kind `{other}`")),
            };
            out.push(parsed);
        }
        Ok(out)
    }
}

/// Cursor-style reader the runtime consumes during replay.
pub struct Replay {
    events: Vec<RecordedEvent>,
    cursor: usize,
}

impl Replay {
    pub fn new(rec: Recording) -> Self {
        Self {
            events: rec.events,
            cursor: 0,
        }
    }

    /// Pop the next event off the front of the recording. Returns
    /// `Err` if we're past the end of the recording, signalling that the
    /// program is doing something different from what was recorded.
    pub fn next_event(&mut self) -> Result<RecordedEvent, String> {
        if self.cursor >= self.events.len() {
            return Err(
                "replay exhausted: the program issued more non-deterministic events than were recorded"
                    .into(),
            );
        }
        let ev = self.events[self.cursor].clone();
        self.cursor += 1;
        Ok(ev)
    }

    pub fn is_done(&self) -> bool {
        self.cursor >= self.events.len()
    }
}

// ---------------------------------------------------------------------------
// JSON (de)serialization for ChatResponse (avoids dragging serde into axon-models)
// ---------------------------------------------------------------------------

fn chat_response_to_json(r: &ChatResponse) -> serde_json::Value {
    let blocks: Vec<serde_json::Value> = r
        .blocks
        .iter()
        .map(|b| match b {
            axon_models::ContentBlock::Text(t) => serde_json::json!({ "kind": "text", "text": t }),
            axon_models::ContentBlock::ToolUse { id, name, input } => serde_json::json!({
                "kind": "tool_use", "id": id, "name": name, "input": input
            }),
            axon_models::ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => serde_json::json!({
                "kind": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error,
            }),
        })
        .collect();
    let tool_calls: Vec<serde_json::Value> = r
        .tool_calls
        .iter()
        .map(|t| {
            serde_json::json!({ "id": t.id, "name": t.name, "input": t.input })
        })
        .collect();
    serde_json::json!({
        "content": r.content,
        "blocks": blocks,
        "structured": r.structured,
        "tool_calls": tool_calls,
        "input_tokens": r.usage.input_tokens,
        "output_tokens": r.usage.output_tokens,
        "cost_usd": r.usage.cost_usd,
        "stop_reason": stop_reason_str(r.stop_reason),
    })
}

fn chat_response_from_json(v: &serde_json::Value) -> Result<ChatResponse, String> {
    let blocks: Vec<axon_models::ContentBlock> = v
        .get("blocks")
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    let kind = b.get("kind").and_then(|k| k.as_str()).unwrap_or("");
                    match kind {
                        "text" => Some(axon_models::ContentBlock::Text(
                            b.get("text").and_then(|t| t.as_str()).unwrap_or("").to_owned(),
                        )),
                        "tool_use" => Some(axon_models::ContentBlock::ToolUse {
                            id: b
                                .get("id")
                                .and_then(|s| s.as_str())
                                .unwrap_or("")
                                .to_owned(),
                            name: b
                                .get("name")
                                .and_then(|s| s.as_str())
                                .unwrap_or("")
                                .to_owned(),
                            input: b.get("input").cloned().unwrap_or(serde_json::Value::Null),
                        }),
                        "tool_result" => Some(axon_models::ContentBlock::ToolResult {
                            tool_use_id: b
                                .get("tool_use_id")
                                .and_then(|s| s.as_str())
                                .unwrap_or("")
                                .to_owned(),
                            content: b
                                .get("content")
                                .and_then(|s| s.as_str())
                                .unwrap_or("")
                                .to_owned(),
                            is_error: b.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false),
                        }),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let tool_calls = v
        .get("tool_calls")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .map(|t| axon_models::ToolCall {
                    id: t.get("id").and_then(|s| s.as_str()).unwrap_or("").to_owned(),
                    name: t
                        .get("name")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    input: t.get("input").cloned().unwrap_or(serde_json::Value::Null),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ChatResponse {
        content: v.get("content").and_then(|s| s.as_str()).unwrap_or("").to_owned(),
        blocks,
        structured: v.get("structured").cloned().filter(|v| !v.is_null()),
        tool_calls,
        usage: axon_models::TokenUsage {
            input_tokens: v.get("input_tokens").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
            output_tokens: v.get("output_tokens").and_then(|n| n.as_u64()).unwrap_or(0)
                as u32,
            cost_usd: v.get("cost_usd").and_then(|n| n.as_f64()).unwrap_or(0.0),
        },
        stop_reason: stop_reason_from_str(
            v.get("stop_reason").and_then(|s| s.as_str()).unwrap_or("end_turn"),
        ),
    })
}

fn stop_reason_str(s: axon_models::StopReason) -> &'static str {
    match s {
        axon_models::StopReason::EndTurn => "end_turn",
        axon_models::StopReason::MaxTokens => "max_tokens",
        axon_models::StopReason::StopSequence => "stop_sequence",
        axon_models::StopReason::ToolUse => "tool_use",
    }
}

fn stop_reason_from_str(s: &str) -> axon_models::StopReason {
    match s {
        "max_tokens" => axon_models::StopReason::MaxTokens,
        "stop_sequence" => axon_models::StopReason::StopSequence,
        "tool_use" => axon_models::StopReason::ToolUse,
        _ => axon_models::StopReason::EndTurn,
    }
}

#[allow(dead_code)]
fn _imports_kept(_: Rc<RefCell<Recording>>, _: Rc<RefCell<Replay>>) {}
