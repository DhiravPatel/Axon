use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostEntry {
    /// Provider name as it appears in `ProviderProfile.name`.
    pub provider: String,
    /// Model name (e.g. `claude-opus-4`).
    #[serde(default)]
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Cached-input tokens that get a price discount (some providers
    /// charge half-rate for these).
    #[serde(default)]
    pub cached_input_tokens: u32,
    pub latency_ms: u64,
    /// Nanoseconds since epoch — used for time-bucket reports.
    pub timestamp_ns: i64,
    /// Free-form tag for grouping (e.g. trace_id, agent_name).
    #[serde(default)]
    pub tag: String,
}
