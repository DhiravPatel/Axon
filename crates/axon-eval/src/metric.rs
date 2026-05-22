//! Metrics — score a single trajectory.
//!
//! Each metric is object-safe so a `Suite` can hold a `Vec<Box<dyn Metric>>`.
//! Returning a `MetricResult` (rather than a bare bool) lets the report
//! show fractional scores for aggregate metrics like latency-percentile.

use serde::{Deserialize, Serialize};

use crate::scenario::{RunResult, Scenario};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricResult {
    pub name: String,
    /// 0.0..=1.0 — fractional pass score. 1.0 = pass, 0.0 = fail, anything
    /// in between = partial credit (e.g. similarity).
    pub score: f64,
    /// Free-form per-result detail surfaced into the report.
    #[serde(default)]
    pub detail: String,
}

impl MetricResult {
    pub fn pass(name: &str) -> Self {
        Self {
            name: name.into(),
            score: 1.0,
            detail: String::new(),
        }
    }
    pub fn fail(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            score: 0.0,
            detail: detail.into(),
        }
    }
    pub fn partial(name: &str, score: f64, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            score: score.clamp(0.0, 1.0),
            detail: detail.into(),
        }
    }
    pub fn passed(&self) -> bool {
        self.score >= 0.999
    }
}

pub trait Metric: Send + Sync {
    fn name(&self) -> &str;
    /// Score a single (scenario, result) pair. Some metrics ignore the
    /// scenario (e.g. latency); others use `scenario.expected` heavily.
    fn score(&self, scenario: &Scenario, result: &RunResult) -> MetricResult;
    /// For aggregate metrics (latency p95). Default is the mean of
    /// per-scenario scores; override if your metric uses a different
    /// aggregation policy.
    fn aggregate(&self, per_scenario: &[MetricResult]) -> MetricResult {
        if per_scenario.is_empty() {
            return MetricResult::fail(self.name(), "no scenarios");
        }
        let sum: f64 = per_scenario.iter().map(|r| r.score).sum();
        let mean = sum / per_scenario.len() as f64;
        MetricResult::partial(self.name(), mean, format!("mean of {} runs", per_scenario.len()))
    }
}

// ---- ExactMatch --------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ExactMatch;
impl Metric for ExactMatch {
    fn name(&self) -> &str {
        "exact_match"
    }
    fn score(&self, scenario: &Scenario, result: &RunResult) -> MetricResult {
        if result.error {
            return MetricResult::fail(self.name(), format!("step error: {}", result.output));
        }
        if result.output == scenario.expected {
            MetricResult::pass(self.name())
        } else {
            MetricResult::fail(
                self.name(),
                format!("expected `{}`, got `{}`", scenario.expected, result.output),
            )
        }
    }
}

// ---- Contains ----------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Contains;
impl Metric for Contains {
    fn name(&self) -> &str {
        "contains"
    }
    fn score(&self, scenario: &Scenario, result: &RunResult) -> MetricResult {
        if result.error {
            return MetricResult::fail(self.name(), format!("step error: {}", result.output));
        }
        if result.output.contains(&scenario.expected) {
            MetricResult::pass(self.name())
        } else {
            MetricResult::fail(
                self.name(),
                format!("output missing substring `{}`", scenario.expected),
            )
        }
    }
}

// ---- RegexLike (anchored wildcard) -------------------------------------

#[derive(Clone, Debug, Default)]
pub struct RegexLike;
impl Metric for RegexLike {
    fn name(&self) -> &str {
        "regex_like"
    }
    fn score(&self, scenario: &Scenario, result: &RunResult) -> MetricResult {
        if result.error {
            return MetricResult::fail(self.name(), format!("step error: {}", result.output));
        }
        // Anchored wildcard semantics shared with axon-guard's policy: `*`
        // matches any run, `?` matches one char. Wrap with `*pattern*` for
        // substring semantics.
        if wildcard_match(&scenario.expected, &result.output) {
            MetricResult::pass(self.name())
        } else {
            MetricResult::fail(
                self.name(),
                format!(
                    "output `{}` does not match pattern `{}`",
                    result.output, scenario.expected
                ),
            )
        }
    }
}

fn wildcard_match(pattern: &str, input: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = input.chars().collect();
    wm(&p, &s)
}
fn wm(p: &[char], s: &[char]) -> bool {
    if p.is_empty() {
        return s.is_empty();
    }
    match p[0] {
        '*' => {
            let rest = &p[1..];
            if wm(rest, s) {
                return true;
            }
            if s.is_empty() {
                return false;
            }
            wm(p, &s[1..])
        }
        '?' => {
            if s.is_empty() {
                return false;
            }
            wm(&p[1..], &s[1..])
        }
        c => !s.is_empty() && s[0] == c && wm(&p[1..], &s[1..]),
    }
}

// ---- JsonPath ---------------------------------------------------------

/// Reads `scenario.expected` as `path=value` and checks that the result's
/// structured `data` field has a matching value at the JSON Pointer-style
/// path. Path syntax: `/foo/bar/0` (slash-separated, integer indices for
/// arrays).
#[derive(Clone, Debug, Default)]
pub struct JsonPath;
impl Metric for JsonPath {
    fn name(&self) -> &str {
        "json_path"
    }
    fn score(&self, scenario: &Scenario, result: &RunResult) -> MetricResult {
        if result.error {
            return MetricResult::fail(self.name(), format!("step error: {}", result.output));
        }
        let (path, expected_val) = match scenario.expected.split_once('=') {
            Some(x) => x,
            None => {
                return MetricResult::fail(
                    self.name(),
                    "expected `path=value` format in scenario.expected",
                );
            }
        };
        let found = match resolve_pointer(&result.data, path) {
            Some(v) => v,
            None => {
                return MetricResult::fail(
                    self.name(),
                    format!("no value at path `{path}`"),
                )
            }
        };
        let found_s = match found {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if found_s == expected_val {
            MetricResult::pass(self.name())
        } else {
            MetricResult::fail(
                self.name(),
                format!("at `{path}`: expected `{expected_val}`, got `{found_s}`"),
            )
        }
    }
}

fn resolve_pointer<'a>(v: &'a serde_json::Value, ptr: &str) -> Option<&'a serde_json::Value> {
    let mut cur = v;
    for seg in ptr.split('/').filter(|s| !s.is_empty()) {
        cur = match cur {
            serde_json::Value::Object(m) => m.get(seg)?,
            serde_json::Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

// ---- LatencyP95 -------------------------------------------------------

/// Pass iff the 95th-percentile latency across the suite is at or below
/// `budget_ms`. Per-scenario score is always 1.0 (the metric is aggregate);
/// the suite-level aggregate computes the real verdict.
#[derive(Clone, Debug)]
pub struct LatencyP95 {
    pub budget_ms: u64,
}

impl LatencyP95 {
    pub fn new(budget_ms: u64) -> Self {
        Self { budget_ms }
    }
}

impl Metric for LatencyP95 {
    fn name(&self) -> &str {
        "latency_p95"
    }
    fn score(&self, _scenario: &Scenario, _result: &RunResult) -> MetricResult {
        // Per-scenario passes; aggregate is what matters.
        MetricResult::pass(self.name())
    }
    fn aggregate(&self, _per_scenario: &[MetricResult]) -> MetricResult {
        // The suite passes the latencies of all runs to us separately.
        // Default impl computes a mean — but we want p95 over the
        // ORIGINAL latencies, not the unit scores. The suite calls
        // `aggregate_with_latencies` for us.
        MetricResult::pass(self.name())
    }
}

impl LatencyP95 {
    pub fn aggregate_with_latencies(&self, latencies_ms: &[u64]) -> MetricResult {
        if latencies_ms.is_empty() {
            return MetricResult::fail(self.name(), "no samples");
        }
        let mut sorted: Vec<u64> = latencies_ms.to_vec();
        sorted.sort_unstable();
        // p95 index = ceil(0.95 * n) - 1, clamped to [0, n-1].
        let idx = ((sorted.len() as f64 * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(sorted.len() - 1);
        let p95 = sorted[idx];
        if p95 <= self.budget_ms {
            MetricResult::partial(
                self.name(),
                1.0,
                format!("p95={p95}ms ≤ {budget}ms", budget = self.budget_ms),
            )
        } else {
            MetricResult::partial(
                self.name(),
                0.0,
                format!("p95={p95}ms > {budget}ms", budget = self.budget_ms),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(expected: &str) -> Scenario {
        Scenario {
            name: "t".into(),
            input: "irrelevant".into(),
            expected: expected.into(),
            tags: vec![],
        }
    }

    #[test]
    fn exact_match_passes_only_on_byte_equality() {
        let m = ExactMatch;
        assert!(m.score(&s("foo"), &RunResult::ok("foo", 0)).passed());
        assert!(!m.score(&s("foo"), &RunResult::ok("foo!", 0)).passed());
    }

    #[test]
    fn contains_finds_substring() {
        let m = Contains;
        assert!(m
            .score(&s("hello"), &RunResult::ok("hello, world", 0))
            .passed());
        assert!(!m.score(&s("hi"), &RunResult::ok("hello", 0)).passed());
    }

    #[test]
    fn regex_like_anchored_wildcard() {
        let m = RegexLike;
        assert!(m
            .score(&s("hi *"), &RunResult::ok("hi there", 0))
            .passed());
        assert!(!m.score(&s("hi *"), &RunResult::ok("say hi", 0)).passed());
        assert!(m
            .score(&s("?id-*"), &RunResult::ok("Aid-123", 0))
            .passed());
    }

    #[test]
    fn json_path_extracts_nested_value() {
        let m = JsonPath;
        let mut r = RunResult::ok("", 0);
        r.data = serde_json::json!({ "user": { "name": "alice" }, "score": 7 });
        assert!(m
            .score(&s("/user/name=alice"), &r)
            .passed());
        assert!(m.score(&s("/score=7"), &r).passed());
        assert!(!m.score(&s("/user/name=bob"), &r).passed());
        assert!(!m.score(&s("/missing=anything"), &r).passed());
    }

    #[test]
    fn latency_p95_passes_when_under_budget() {
        let m = LatencyP95::new(100);
        let lats = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 99];
        let agg = m.aggregate_with_latencies(&lats);
        assert!(agg.passed(), "got: {agg:?}");
    }

    #[test]
    fn latency_p95_fails_when_over_budget() {
        let m = LatencyP95::new(50);
        let lats = vec![10, 20, 30, 40, 200];
        let agg = m.aggregate_with_latencies(&lats);
        assert!(!agg.passed(), "got: {agg:?}");
    }

    #[test]
    fn step_error_fails_every_metric() {
        for metric in [
            &ExactMatch as &dyn Metric,
            &Contains as &dyn Metric,
            &RegexLike as &dyn Metric,
        ] {
            let r = RunResult::err("oh no", 50);
            assert!(!metric.score(&s("anything"), &r).passed());
        }
    }
}
