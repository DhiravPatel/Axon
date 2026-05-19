//! Deterministic mock provider.
//!
//! Used by tests and by programs that haven't been granted Net.
//! Constructed with a [`MockBehavior`] that decides what to return for each
//! `complete` call. Flavors:
//!
//!   * `Echo` — return the concatenation of the request's user-message text.
//!   * `Fixed(text)` — always return the same text.
//!   * `Script(items)` — round-robin through a list of canned text strings.
//!   * `Turns(turns)` — most-faithful: each successive call returns the
//!     next [`MockTurn`], which can be either text or a sequence of tool
//!     calls. Lets tests drive a full multi-turn tool-use loop
//!     deterministically.
//!
//! Structured-output requests get a stub JSON object built from the
//! schema (field names → empty defaults).

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    ChatRequest, ChatResponse, ContentBlock, ModelProvider, ProviderError, Role, StopReason,
    ToolCall, TokenUsage,
};

pub struct MockProvider {
    name: String,
    behavior: MockBehavior,
    counter: AtomicUsize,
}

#[derive(Clone, Debug)]
pub enum MockBehavior {
    Echo,
    Fixed(String),
    Script(Vec<String>),
    /// Sequence of full turns — each call to `complete` returns the next
    /// one in order, looping back to the start when exhausted.
    Turns(Vec<MockTurn>),
}

#[derive(Clone, Debug)]
pub enum MockTurn {
    Text(String),
    /// `(text_before, tool_calls)` — both are emitted. Stop reason is
    /// `ToolUse` so the runtime will iterate.
    Tools {
        text: String,
        calls: Vec<ToolCall>,
    },
}

impl MockProvider {
    pub fn new(behavior: MockBehavior) -> Self {
        Self {
            name: "mock".to_string(),
            behavior,
            counter: AtomicUsize::new(0),
        }
    }

    pub fn echo() -> Self {
        Self::new(MockBehavior::Echo)
    }
}

impl ModelProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let structured = req.output_schema.as_ref().map(stub_for_schema);

        let (content, tool_calls, blocks, stop_reason) = match &self.behavior {
            MockBehavior::Echo => {
                let text = req
                    .messages
                    .iter()
                    .filter(|m| matches!(m.role, Role::User))
                    .map(|m| m.text())
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    text.clone(),
                    Vec::new(),
                    vec![ContentBlock::Text(text)],
                    StopReason::EndTurn,
                )
            }
            MockBehavior::Fixed(s) => (
                s.clone(),
                Vec::new(),
                vec![ContentBlock::Text(s.clone())],
                StopReason::EndTurn,
            ),
            MockBehavior::Script(items) => {
                let i = self.counter.fetch_add(1, Ordering::SeqCst);
                let s = items.get(i % items.len()).cloned().unwrap_or_default();
                (
                    s.clone(),
                    Vec::new(),
                    vec![ContentBlock::Text(s)],
                    StopReason::EndTurn,
                )
            }
            MockBehavior::Turns(turns) => {
                if turns.is_empty() {
                    (String::new(), Vec::new(), Vec::new(), StopReason::EndTurn)
                } else {
                    let i = self.counter.fetch_add(1, Ordering::SeqCst);
                    let turn = &turns[i % turns.len()];
                    match turn {
                        MockTurn::Text(s) => (
                            s.clone(),
                            Vec::new(),
                            vec![ContentBlock::Text(s.clone())],
                            StopReason::EndTurn,
                        ),
                        MockTurn::Tools { text, calls } => {
                            let mut blocks: Vec<ContentBlock> = Vec::new();
                            if !text.is_empty() {
                                blocks.push(ContentBlock::Text(text.clone()));
                            }
                            for c in calls {
                                blocks.push(ContentBlock::ToolUse {
                                    id: c.id.clone(),
                                    name: c.name.clone(),
                                    input: c.input.clone(),
                                });
                            }
                            (
                                text.clone(),
                                calls.clone(),
                                blocks,
                                StopReason::ToolUse,
                            )
                        }
                    }
                }
            }
        };

        Ok(ChatResponse {
            content,
            blocks,
            structured,
            tool_calls,
            usage: TokenUsage {
                input_tokens: req
                    .messages
                    .iter()
                    .map(|m| m.text().len() as u32)
                    .sum(),
                output_tokens: 0,
                cost_usd: 0.0,
            },
            stop_reason,
        })
    }
}

fn stub_for_schema(schema: &serde_json::Value) -> serde_json::Value {
    if let Some(obj) = schema.as_object() {
        if obj.get("type").and_then(|v| v.as_str()) == Some("object") {
            let mut out = serde_json::Map::new();
            if let Some(props) = obj.get("properties").and_then(|v| v.as_object()) {
                for (key, sub) in props {
                    out.insert(key.clone(), stub_for_schema(sub));
                }
            }
            return serde_json::Value::Object(out);
        }
        match obj.get("type").and_then(|v| v.as_str()) {
            Some("string") => return serde_json::Value::String(String::new()),
            Some("integer") => return serde_json::Value::Number(0.into()),
            Some("number") => return serde_json::json!(0.0),
            Some("boolean") => return serde_json::Value::Bool(false),
            Some("array") => return serde_json::Value::Array(Vec::new()),
            _ => {}
        }
    }
    serde_json::Value::Null
}
