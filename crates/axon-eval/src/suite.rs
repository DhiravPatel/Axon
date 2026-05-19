//! Suite runner.

use crate::metric::{LatencyP95, Metric, MetricResult};
use crate::report::{ScenarioReport, SuiteReport};
use crate::scenario::{RunResult, Scenario};

pub struct Suite {
    pub name: String,
    pub scenarios: Vec<Scenario>,
    pub metrics: Vec<Box<dyn Metric>>,
    /// Optional latency-percentile metric; carried separately because it
    /// uses a custom aggregator (`aggregate_with_latencies`).
    pub latency_metric: Option<LatencyP95>,
}

impl Suite {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            scenarios: Vec::new(),
            metrics: Vec::new(),
            latency_metric: None,
        }
    }

    pub fn add_scenario(mut self, s: Scenario) -> Self {
        self.scenarios.push(s);
        self
    }

    pub fn add_metric(mut self, m: Box<dyn Metric>) -> Self {
        self.metrics.push(m);
        self
    }

    pub fn with_latency_p95(mut self, budget_ms: u64) -> Self {
        self.latency_metric = Some(LatencyP95::new(budget_ms));
        self
    }

    /// Run every scenario through `step`, score it through every metric,
    /// and assemble a [`SuiteReport`]. `step` is `FnMut` so the host can
    /// capture &mut state (e.g. an interpreter handle) into the closure.
    pub fn run(&self, mut step: impl FnMut(&Scenario) -> RunResult) -> SuiteReport {
        let mut per_scenario: Vec<ScenarioReport> = Vec::with_capacity(self.scenarios.len());
        let mut latencies: Vec<u64> = Vec::with_capacity(self.scenarios.len());
        let mut passed_runs = 0usize;

        for sc in &self.scenarios {
            let result = step(sc);
            latencies.push(result.latency_ms);
            let metrics: Vec<MetricResult> =
                self.metrics.iter().map(|m| m.score(sc, &result)).collect();
            let passed = metrics.iter().all(|m| m.passed()) && !result.error;
            if passed {
                passed_runs += 1;
            }
            per_scenario.push(ScenarioReport {
                scenario_name: sc.name.clone(),
                result,
                metrics,
            });
        }

        let mut aggregates: Vec<MetricResult> = Vec::new();
        for metric in &self.metrics {
            let per_metric: Vec<MetricResult> = per_scenario
                .iter()
                .filter_map(|s| s.metrics.iter().find(|m| m.name == metric.name()).cloned())
                .collect();
            aggregates.push(metric.aggregate(&per_metric));
        }
        if let Some(lm) = &self.latency_metric {
            aggregates.push(lm.aggregate_with_latencies(&latencies));
        }

        SuiteReport {
            suite_name: self.name.clone(),
            total_runs: self.scenarios.len(),
            passed_runs,
            per_scenario,
            aggregates,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metric::{Contains, ExactMatch};

    #[test]
    fn suite_runs_and_aggregates() {
        let suite = Suite::new("greeting")
            .add_scenario(Scenario {
                name: "exact".into(),
                input: "hi".into(),
                expected: "hello world".into(),
                tags: vec![],
            })
            .add_scenario(Scenario {
                name: "contains".into(),
                input: "hi".into(),
                expected: "hello".into(),
                tags: vec![],
            })
            .add_metric(Box::new(ExactMatch))
            .add_metric(Box::new(Contains));

        let report = suite.run(|sc| {
            // step always returns "hello world" regardless of input.
            RunResult::ok("hello world", 10)
        });
        assert_eq!(report.total_runs, 2);
        // First scenario: ExactMatch passes (output == expected),
        // Contains also passes. Second scenario: ExactMatch fails
        // (output `hello world` != expected `hello`), Contains passes.
        assert_eq!(report.passed_runs, 1);
    }

    #[test]
    fn latency_p95_threshold_in_aggregates() {
        let suite = Suite::new("latency")
            .add_scenario(Scenario {
                name: "a".into(),
                input: "".into(),
                expected: "".into(),
                tags: vec![],
            })
            .add_scenario(Scenario {
                name: "b".into(),
                input: "".into(),
                expected: "".into(),
                tags: vec![],
            })
            .with_latency_p95(50);
        let report = suite.run(|sc| match sc.name.as_str() {
            "a" => RunResult::ok("", 10),
            _ => RunResult::ok("", 200),
        });
        let agg = report
            .aggregates
            .iter()
            .find(|m| m.name == "latency_p95")
            .unwrap();
        assert!(!agg.passed(), "p95=200 > 50");
    }

    #[test]
    fn junit_xml_contains_failures() {
        let suite = Suite::new("junit")
            .add_scenario(Scenario {
                name: "ok".into(),
                input: "".into(),
                expected: "ok".into(),
                tags: vec![],
            })
            .add_scenario(Scenario {
                name: "fail".into(),
                input: "".into(),
                expected: "should-not-match".into(),
                tags: vec![],
            })
            .add_metric(Box::new(ExactMatch));
        let report = suite.run(|sc| RunResult::ok(sc.expected.clone(), 5));
        let xml = report.to_junit_xml();
        assert!(xml.contains("testsuite"));
        // ExactMatch ALSO matches the second scenario because the step
        // echoes scenario.expected. So both pass; let's instead force a
        // failure.
        let report2 = suite.run(|sc| RunResult::ok("never-the-expected", 5));
        let xml2 = report2.to_junit_xml();
        assert!(xml2.contains("<failure"));
        assert!(xml2.contains("failures=\"2\""));
    }
}
