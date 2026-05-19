//! Model providers for the Axon runtime.
//!
//! `ask`, `generate<S>`, and `plan` all bottom out in a call to a
//! [`ModelProvider`]. Stage 6 ships two implementations:
//!
//!   * [`AnthropicProvider`] — speaks the Anthropic Messages API over HTTPS.
//!     Requires `ANTHROPIC_API_KEY` in the host environment.
//!   * [`MockProvider`] — deterministic, no network. Used by tests and by
//!     CI/local development without an API key.
//!
//! Both implement the same [`ModelProvider`] trait, so the runtime doesn't
//! care which one it's talking to. Test programs can construct mock
//! providers with canned responses; production programs construct
//! Anthropic providers via the `anthropic("claude-...")` built-in.

pub mod anthropic;
pub mod mock;
pub mod schema;

use std::fmt;

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ChatRequest {
    /// Model id (e.g. `claude-opus-4-7`). Empty for providers that pin the
    /// model at construction time.
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    pub temperature: Option<f64>,
    pub stop_sequences: Vec<String>,
    /// If set, the provider will be asked to return structured output
    /// matching this JSON Schema. The Anthropic provider implements this
    /// by issuing a single tool-use round-trip with `tool_choice` forcing
    /// the synthesized tool.
    pub output_schema: Option<serde_json::Value>,
    /// Name to use for the output tool when `output_schema` is set.
    /// Defaults to "structured_output" if empty.
    pub output_schema_name: Option<String>,
    /// User-declared tools the model may invoke. The runtime executes the
    /// chosen tool, feeds the result back as a `ToolResult` block, and
    /// loops until the model returns final text.
    pub tools: Vec<ToolSpec>,
}

/// One message in a conversation. Content is a list of *blocks* so we can
/// faithfully round-trip Anthropic's multi-block protocol (text + tool_use
/// + tool_result interleaved).
#[derive(Clone, Debug)]
pub struct Message {
    pub role: Role,
    pub blocks: Vec<ContentBlock>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            blocks: vec![ContentBlock::Text(text.into())],
        }
    }
    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            blocks: vec![ContentBlock::Text(text.into())],
        }
    }
    /// Concatenate every Text block into a single string. Useful for
    /// providers/tests that work with plain text.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for b in &self.blocks {
            if let ContentBlock::Text(t) = b {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
        out
    }
}

/// One content block inside a [`Message`]. Mirrors the Anthropic content-
/// block taxonomy: a turn can carry a mix of plain text, tool invocations
/// the model has decided to make, and the results the host fed back.
#[derive(Clone, Debug)]
pub enum ContentBlock {
    Text(String),
    /// A tool the model wants the host to execute.
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// The result of a tool the host has executed, fed back to the model.
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

/// Declaration of a tool the model may invoke during a turn. Mirrors the
/// Anthropic `tools` array entries (name + description + input_schema).
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// One tool invocation pulled out of a [`ChatResponse`]. The host runs the
/// named tool with `input`, then sends a [`ContentBlock::ToolResult`] in
/// the next user message keyed by the same `id`.
#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, Default)]
pub struct ChatResponse {
    /// Plain text from the model's final assistant turn (concatenation of
    /// every [`ContentBlock::Text`] in `blocks`). Empty when the model
    /// produced only tool calls.
    pub content: String,
    /// Raw block stream, suitable for re-sending as the assistant message
    /// in a follow-up request (the tool-use protocol requires this).
    pub blocks: Vec<ContentBlock>,
    /// JSON object returned by the model when the request supplied an
    /// `output_schema`. None for plain ask/plan calls.
    pub structured: Option<serde_json::Value>,
    /// Tool invocations the model has decided to make. The runtime executes
    /// each one, builds tool_result blocks, and continues the conversation.
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
    pub stop_reason: StopReason,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Estimated dollar cost for the call. Providers fill this in based on
    /// their published per-token rates; mock providers leave it at 0.0.
    pub cost_usd: f64,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum StopReason {
    #[default]
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ProviderError {
    AuthMissing,
    Network(String),
    Api { status: u16, message: String },
    InvalidResponse(String),
    NotSupported(&'static str),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::AuthMissing => {
                f.write_str("ANTHROPIC_API_KEY is not set in the environment")
            }
            ProviderError::Network(s) => write!(f, "network error: {s}"),
            ProviderError::Api { status, message } => {
                write!(f, "Anthropic API returned {status}: {message}")
            }
            ProviderError::InvalidResponse(s) => write!(f, "invalid provider response: {s}"),
            ProviderError::NotSupported(s) => write!(f, "provider does not support: {s}"),
        }
    }
}

impl std::error::Error for ProviderError {}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// One provider can serve many calls. Implementations are expected to be
/// thread-safe (`Send + Sync`) so the host can share a single provider
/// across actors when the future scheduler arrives — but Stage 6 only uses
/// it from the single-threaded interpreter, so `Sync` isn't enforced yet.
pub trait ModelProvider: Send {
    /// Human-readable name for diagnostics (`"anthropic:claude-..."`,
    /// `"mock"`).
    fn name(&self) -> &str;

    /// One synchronous call. Returns either a chat response or a
    /// provider-specific error.
    fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError>;
}

pub use anthropic::AnthropicProvider;
pub use mock::{MockBehavior, MockProvider, MockTurn};
pub use schema::ast_type_to_json_schema;
