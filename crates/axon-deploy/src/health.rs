//! Health-check trait + built-ins.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub ok: bool,
    #[serde(default)]
    pub detail: String,
}

impl CheckResult {
    pub fn ok() -> Self {
        Self {
            ok: true,
            detail: String::new(),
        }
    }
    pub fn fail(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
        }
    }
}

pub trait HealthCheck: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self) -> CheckResult;
}

/// Always-OK check. Useful as a smoke test that the server is up at all
/// (separate from readiness, which should be more discerning).
pub struct Liveness;
impl HealthCheck for Liveness {
    fn name(&self) -> &str {
        "liveness"
    }
    fn check(&self) -> CheckResult {
        CheckResult::ok()
    }
}

/// Convenience: a check that always returns OK with a custom name.
pub struct AlwaysHealthy(pub &'static str);
impl HealthCheck for AlwaysHealthy {
    fn name(&self) -> &str {
        self.0
    }
    fn check(&self) -> CheckResult {
        CheckResult::ok()
    }
}
