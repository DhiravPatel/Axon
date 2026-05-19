//! Report builder — aggregates a `Ledger` against per-provider profiles.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::entry::CostEntry;
use crate::ledger::Ledger;
use crate::profile::ProviderProfile;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSummary {
    pub provider: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_cents: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopCall {
    pub provider: String,
    pub model: String,
    pub tag: String,
    pub cents: u64,
    pub latency_ms: u64,
    pub timestamp_ns: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub total_calls: u64,
    pub total_cents: u64,
    pub providers: Vec<ProviderSummary>,
    pub p50_latency_ms: u64,
    pub p95_latency_ms: u64,
    pub top_calls: Vec<TopCall>,
}

impl Report {
    /// Build a report. Unknown providers (no matching profile) contribute
    /// 0 cents but are still counted in call totals — that way the report
    /// surface remains useful even if the user forgot to register a profile.
    pub fn build(ledger: &Ledger, profiles: &[ProviderProfile], top_n: usize) -> Self {
        let mut by_provider: HashMap<String, ProviderSummary> = HashMap::new();
        let mut latencies: Vec<u64> = Vec::with_capacity(ledger.entries.len());
        let mut all_calls: Vec<(u64, &CostEntry)> = Vec::with_capacity(ledger.entries.len());

        for e in &ledger.entries {
            let profile = profiles
                .iter()
                .find(|p| p.name == e.provider && (p.model.is_empty() || p.model == e.model));
            let cents = profile
                .map(|p| p.cost_cents(e.input_tokens, e.output_tokens, e.cached_input_tokens))
                .unwrap_or(0);
            let s = by_provider
                .entry(e.provider.clone())
                .or_insert(ProviderSummary {
                    provider: e.provider.clone(),
                    calls: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    total_cents: 0,
                });
            s.calls += 1;
            s.input_tokens += e.input_tokens as u64;
            s.output_tokens += e.output_tokens as u64;
            s.total_cents += cents;
            latencies.push(e.latency_ms);
            all_calls.push((cents, e));
        }

        latencies.sort_unstable();
        let p50 = percentile(&latencies, 50.0);
        let p95 = percentile(&latencies, 95.0);

        all_calls.sort_by(|a, b| b.0.cmp(&a.0));
        let top_calls: Vec<TopCall> = all_calls
            .iter()
            .take(top_n)
            .map(|(c, e)| TopCall {
                provider: e.provider.clone(),
                model: e.model.clone(),
                tag: e.tag.clone(),
                cents: *c,
                latency_ms: e.latency_ms,
                timestamp_ns: e.timestamp_ns,
            })
            .collect();

        let total_cents: u64 = by_provider.values().map(|s| s.total_cents).sum();
        let mut providers: Vec<ProviderSummary> = by_provider.into_values().collect();
        providers.sort_by(|a, b| b.total_cents.cmp(&a.total_cents));

        Report {
            total_calls: ledger.entries.len() as u64,
            total_cents,
            providers,
            p50_latency_ms: p50,
            p95_latency_ms: p95,
            top_calls,
        }
    }
}

fn percentile(sorted: &[u64], pct: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 * pct / 100.0).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider: &str, model: &str, input: u32, output: u32, lat: u64) -> CostEntry {
        CostEntry {
            provider: provider.into(),
            model: model.into(),
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: 0,
            latency_ms: lat,
            timestamp_ns: 0,
            tag: String::new(),
        }
    }

    fn profile(name: &str, input_per_m: u64, output_per_m: u64) -> ProviderProfile {
        ProviderProfile {
            name: name.into(),
            model: String::new(),
            input_cents_per_million: input_per_m,
            output_cents_per_million: output_per_m,
            cached_input_cents_per_million: 0,
            per_call_cents: 0,
        }
    }

    #[test]
    fn cost_computation_matches_token_rates() {
        let p = profile("acme", /* $3/M = */ 300, /* $15/M = */ 1500);
        // 1k input + 2k output = 0.3¢ input + 3¢ output = 3¢ total.
        // With integer cents math: 1000 * 300 / 1_000_000 = 0, 2000 * 1500 / 1_000_000 = 3.
        // So total = 3¢.
        assert_eq!(p.cost_cents(1_000, 2_000, 0), 3);
    }

    #[test]
    fn report_aggregates_by_provider() {
        let mut l = Ledger::new();
        l.record(entry("anthropic", "opus", 1000, 1000, 200));
        l.record(entry("anthropic", "opus", 2000, 3000, 500));
        l.record(entry("openai", "gpt-4", 1000, 1000, 800));
        let r = Report::build(
            &l,
            &[profile("anthropic", 300, 1500), profile("openai", 250, 1000)],
            10,
        );
        assert_eq!(r.total_calls, 3);
        assert_eq!(r.providers.len(), 2);
        let anth = r.providers.iter().find(|p| p.provider == "anthropic").unwrap();
        assert_eq!(anth.calls, 2);
    }

    #[test]
    fn top_calls_sorted_by_cost_descending() {
        let mut l = Ledger::new();
        l.record(entry("acme", "small", 1_000, 1_000, 10));
        l.record(entry("acme", "big", 10_000_000, 10_000_000, 10));
        l.record(entry("acme", "medium", 100_000, 100_000, 10));
        let r = Report::build(&l, &[profile("acme", 100, 100)], 3);
        assert_eq!(r.top_calls.len(), 3);
        assert!(r.top_calls[0].cents >= r.top_calls[1].cents);
        assert!(r.top_calls[1].cents >= r.top_calls[2].cents);
        assert_eq!(r.top_calls[0].model, "big");
    }

    #[test]
    fn percentiles_are_sensible() {
        let mut l = Ledger::new();
        for ms in [10, 20, 30, 40, 50, 60, 70, 80, 90, 100] {
            l.record(entry("acme", "", 0, 0, ms));
        }
        let r = Report::build(&l, &[], 0);
        // p95 of 10 evenly spaced samples is the 9.5 → ceil = 10th element
        // (index 9) → 100ms. p50 is the 5th element → 50ms.
        assert_eq!(r.p95_latency_ms, 100);
        assert_eq!(r.p50_latency_ms, 50);
    }

    #[test]
    fn unknown_provider_costs_zero_but_still_counted() {
        let mut l = Ledger::new();
        l.record(entry("mystery", "x", 1_000_000, 1_000_000, 5));
        let r = Report::build(&l, &[profile("acme", 300, 1500)], 10);
        assert_eq!(r.total_calls, 1);
        assert_eq!(r.total_cents, 0);
        // The provider still appears in the summary.
        assert_eq!(r.providers[0].provider, "mystery");
    }

    #[test]
    fn ledger_round_trip_through_disk() {
        let mut p = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!("axon-ledger-{}-{ts}.json", std::process::id()));

        let mut l = Ledger::new();
        l.record(entry("acme", "x", 100, 200, 50));
        l.save(&p).unwrap();
        let back = Ledger::load(&p).unwrap();
        assert_eq!(back.entries, l.entries);
        let _ = std::fs::remove_file(&p);
    }
}
