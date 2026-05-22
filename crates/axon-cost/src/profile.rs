use serde::{Deserialize, Serialize};

/// Per-provider pricing. All amounts in **cents** (so the integer math
/// inside the ledger doesn't have to handle fractional pennies). Use
/// `_per_million` for token rates because cents-per-token would round
/// down to zero for cheap models.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub name: String,
    #[serde(default)]
    pub model: String,
    /// Cents per million **input** tokens.
    pub input_cents_per_million: u64,
    /// Cents per million **output** tokens.
    pub output_cents_per_million: u64,
    /// Optional discount for cached input tokens (cents per million).
    #[serde(default)]
    pub cached_input_cents_per_million: u64,
    /// Per-call fixed cost in cents (rare; some providers charge a flat
    /// fee on top of token usage).
    #[serde(default)]
    pub per_call_cents: u32,
}

impl ProviderProfile {
    /// Cost of a single call in cents, given the token counts on the
    /// matching [`crate::CostEntry`].
    pub fn cost_cents(
        &self,
        input_tokens: u32,
        output_tokens: u32,
        cached_input_tokens: u32,
    ) -> u64 {
        // Use checked arithmetic so we never wrap silently for huge
        // token counts.
        let input = (input_tokens as u64)
            .saturating_mul(self.input_cents_per_million)
            / 1_000_000;
        let output = (output_tokens as u64)
            .saturating_mul(self.output_cents_per_million)
            / 1_000_000;
        let cached = (cached_input_tokens as u64)
            .saturating_mul(self.cached_input_cents_per_million)
            / 1_000_000;
        input
            .saturating_add(output)
            .saturating_add(cached)
            .saturating_add(self.per_call_cents as u64)
    }
}
