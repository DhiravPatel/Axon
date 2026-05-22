//! Scenario + run result types.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub input: String,
    /// What the scenario expects. Free-form because different metrics use
    /// it differently — `ExactMatch` reads it literally, `JsonPath` reads
    /// it as `path=value`, etc.
    #[serde(default)]
    pub expected: String,
    /// Optional tag set for grouping.
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub output: String,
    pub latency_ms: u64,
    /// Optional structured payload — JSON-Path metrics read from here.
    #[serde(default)]
    pub data: serde_json::Value,
    /// True if the underlying step itself errored (vs. just produced a
    /// "wrong" answer). Metrics typically treat this as a failure
    /// regardless of expectation.
    #[serde(default)]
    pub error: bool,
}

impl RunResult {
    pub fn ok(output: impl Into<String>, latency_ms: u64) -> Self {
        Self {
            output: output.into(),
            latency_ms,
            data: serde_json::Value::Null,
            error: false,
        }
    }
    pub fn err(message: impl Into<String>, latency_ms: u64) -> Self {
        Self {
            output: message.into(),
            latency_ms,
            data: serde_json::Value::Null,
            error: true,
        }
    }
}
