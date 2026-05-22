//! Suite report — JSON + JUnit XML output.

use serde::{Deserialize, Serialize};

use crate::metric::MetricResult;
use crate::scenario::RunResult;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioReport {
    pub scenario_name: String,
    pub result: RunResult,
    pub metrics: Vec<MetricResult>,
}

impl ScenarioReport {
    pub fn all_passed(&self) -> bool {
        self.metrics.iter().all(|m| m.passed()) && !self.result.error
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuiteReport {
    pub suite_name: String,
    pub per_scenario: Vec<ScenarioReport>,
    pub aggregates: Vec<MetricResult>,
    pub total_runs: usize,
    pub passed_runs: usize,
}

impl SuiteReport {
    pub fn pass_rate(&self) -> f64 {
        if self.total_runs == 0 {
            return 0.0;
        }
        self.passed_runs as f64 / self.total_runs as f64
    }

    pub fn all_passed(&self) -> bool {
        self.total_runs > 0
            && self.passed_runs == self.total_runs
            && self.aggregates.iter().all(|m| m.passed())
    }

    pub fn to_junit_xml(&self) -> String {
        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        let failures = self.total_runs - self.passed_runs;
        out.push_str(&format!(
            "<testsuite name=\"{}\" tests=\"{}\" failures=\"{}\">\n",
            xml_escape(&self.suite_name),
            self.total_runs,
            failures
        ));
        for s in &self.per_scenario {
            let passed = s.all_passed();
            out.push_str(&format!(
                "  <testcase name=\"{}\" time=\"{}\">\n",
                xml_escape(&s.scenario_name),
                s.result.latency_ms as f64 / 1000.0
            ));
            if !passed {
                let reasons: Vec<String> = s
                    .metrics
                    .iter()
                    .filter(|m| !m.passed())
                    .map(|m| format!("{}: {}", m.name, m.detail))
                    .collect();
                out.push_str(&format!(
                    "    <failure message=\"{}\"/>\n",
                    xml_escape(&reasons.join("; "))
                ));
            }
            out.push_str("  </testcase>\n");
        }
        out.push_str("</testsuite>\n");
        out
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
