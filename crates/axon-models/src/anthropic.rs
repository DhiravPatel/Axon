//! Anthropic Messages API provider.
//!
//! Three call shapes:
//!
//!   * **Plain** — `tools` empty, `output_schema` unset. Just sends the
//!     conversation; returns concatenated text.
//!   * **Structured** — `output_schema` set. Wraps the schema in a single
//!     forced tool so the model has to emit it as a tool call. We pluck
//!     the call's input as the structured response.
//!   * **Tool-use** — `tools` non-empty. The model may produce one or more
//!     `tool_use` blocks; the host runs them and posts the results back.
//!     The runtime drives the loop.
//!
//! Messages are sent as block arrays whenever a message has anything other
//! than a single text block, matching the Anthropic API's content-block
//! protocol.

use std::env;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    ChatRequest, ChatResponse, ContentBlock, ModelProvider, ProviderError, Role, StopReason,
    ToolCall, TokenUsage,
};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 1024;

pub struct AnthropicProvider {
    api_key: String,
    model: String,
    agent: ureq::Agent,
    name: String,
}

impl AnthropicProvider {
    pub fn from_env(model: impl Into<String>) -> Result<Self, ProviderError> {
        let api_key = env::var("ANTHROPIC_API_KEY").map_err(|_| ProviderError::AuthMissing)?;
        Ok(Self::new(api_key, model))
    }

    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(30))
            .timeout(Duration::from_secs(120))
            .build();
        Self {
            api_key: api_key.into(),
            name: format!("anthropic:{model}"),
            model,
            agent,
        }
    }
}

impl ModelProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let model = if req.model.is_empty() {
            &self.model
        } else {
            &req.model
        };
        let max_tokens = if req.max_tokens == 0 {
            DEFAULT_MAX_TOKENS
        } else {
            req.max_tokens
        };

        let messages: Vec<ApiMessage> = req
            .messages
            .iter()
            .map(|m| ApiMessage {
                role: match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                },
                content: encode_blocks(&m.blocks),
            })
            .collect();

        // Build tools list: user-declared tools + (if output_schema is set)
        // a forced synthetic tool. The synthetic tool is its own
        // `tool_choice`, so when both are present the user's tools are
        // unreachable on that turn — which is fine, because generate<S>
        // requests don't pass user tools today.
        let mut tools: Vec<ApiTool> = req
            .tools
            .iter()
            .map(|t| ApiTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
            })
            .collect();
        let mut tool_choice: Option<ApiToolChoice> = None;
        if let Some(schema) = &req.output_schema {
            let name = req
                .output_schema_name
                .clone()
                .unwrap_or_else(|| "structured_output".to_string());
            tools.push(ApiTool {
                name: name.clone(),
                description: "Return the answer using this exact schema.".to_string(),
                input_schema: schema.clone(),
            });
            tool_choice = Some(ApiToolChoice {
                kind: "tool",
                name: Some(name),
            });
        }

        let body = ApiRequest {
            model: model.clone(),
            max_tokens,
            system: req.system.clone(),
            temperature: req.temperature,
            stop_sequences: if req.stop_sequences.is_empty() {
                None
            } else {
                Some(req.stop_sequences.clone())
            },
            messages,
            tools: if tools.is_empty() { None } else { Some(tools) },
            tool_choice,
        };

        let json_body =
            serde_json::to_value(&body).map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;

        let res = self
            .agent
            .post(API_URL)
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", ANTHROPIC_VERSION)
            .set("content-type", "application/json")
            .send_json(json_body);

        let resp = match res {
            Ok(r) => r,
            Err(ureq::Error::Status(status, r)) => {
                let body = r
                    .into_string()
                    .unwrap_or_else(|_| "<unreadable error body>".to_string());
                return Err(ProviderError::Api {
                    status,
                    message: body,
                });
            }
            Err(e) => return Err(ProviderError::Network(e.to_string())),
        };

        let api: ApiResponse = resp
            .into_json()
            .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;

        Ok(decode_response(api, req.output_schema_name.as_deref()))
    }
}

fn encode_blocks(blocks: &[ContentBlock]) -> serde_json::Value {
    // Single-text shortcut keeps wire payloads small and easier to read in
    // logs. Multi-block content always serializes as an array.
    if blocks.len() == 1 {
        if let ContentBlock::Text(t) = &blocks[0] {
            return serde_json::Value::String(t.clone());
        }
    }
    let arr: Vec<serde_json::Value> = blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Text(t) => serde_json::json!({ "type": "text", "text": t }),
            ContentBlock::ToolUse { id, name, input } => serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            }),
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error,
            }),
        })
        .collect();
    serde_json::Value::Array(arr)
}

fn decode_response(api: ApiResponse, output_schema_name: Option<&str>) -> ChatResponse {
    let cost_usd = estimate_cost_usd(
        api.model.as_deref().unwrap_or(""),
        api.usage.input_tokens,
        api.usage.output_tokens,
    );
    let mut content = String::new();
    let mut blocks: Vec<ContentBlock> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut structured: Option<serde_json::Value> = None;
    for block in api.content {
        match block.kind.as_str() {
            "text" => {
                if let Some(t) = block.text {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&t);
                    blocks.push(ContentBlock::Text(t));
                }
            }
            "tool_use" => {
                let id = block.id.clone().unwrap_or_default();
                let name = block.name.clone().unwrap_or_default();
                let input = block.input.clone().unwrap_or(serde_json::Value::Null);
                // If this tool_use is the forced structured-output tool,
                // stash its input as the structured field instead of as a
                // tool_call (we don't want the runtime to "execute" the
                // synthetic tool).
                if output_schema_name.map_or(false, |n| n == name) {
                    structured = Some(input.clone());
                } else {
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    });
                }
                blocks.push(ContentBlock::ToolUse { id, name, input });
            }
            _ => {}
        }
    }
    let stop_reason = match api.stop_reason.as_deref() {
        Some("end_turn") => StopReason::EndTurn,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        Some("tool_use") => StopReason::ToolUse,
        _ => StopReason::EndTurn,
    };
    ChatResponse {
        content,
        blocks,
        structured,
        tool_calls,
        usage: TokenUsage {
            input_tokens: api.usage.input_tokens,
            output_tokens: api.usage.output_tokens,
            cost_usd,
        },
        stop_reason,
    }
}

/// Rough per-million-token pricing for cost estimation. Numbers track the
/// public Anthropic price list at the time of writing; users with a custom
/// pricing arrangement can ignore the estimate (`cost_usd` is informational
/// — the budget enforcement also tracks raw token counts).
fn estimate_cost_usd(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    let (input_per_m, output_per_m) = if model.contains("opus") {
        (15.0, 75.0)
    } else if model.contains("sonnet") {
        (3.0, 15.0)
    } else if model.contains("haiku") {
        (0.80, 4.0)
    } else {
        // Unknown model — best to surface zero than to mislead.
        (0.0, 0.0)
    };
    (input_tokens as f64 * input_per_m + output_tokens as f64 * output_per_m) / 1_000_000.0
}

// ---------------------------------------------------------------------------
// On-the-wire JSON shapes
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<ApiToolChoice>,
}

#[derive(Serialize)]
struct ApiMessage {
    role: &'static str,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Serialize)]
struct ApiToolChoice {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ApiContentBlock>,
    stop_reason: Option<String>,
    usage: ApiUsage,
    model: Option<String>,
}

#[derive(Deserialize)]
struct ApiContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
    id: Option<String>,
    name: Option<String>,
    input: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ApiUsage {
    input_tokens: u32,
    output_tokens: u32,
}
