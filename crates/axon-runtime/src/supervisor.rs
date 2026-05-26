//! Supervisor restart strategies.
//!
//! Implements the three classic Erlang-style policies on top of a
//! sliding-window failure counter. Each [`Supervisor`] owns:
//!
//!   * a [`RestartStrategy`] — what to do when a child fails.
//!   * a `max_restarts` + `within_ns` window — if more than
//!     `max_restarts` failures land inside one window, the supervisor
//!     **escalates** instead of restarting (Erlang's "max restart
//!     frequency").
//!   * an ordered list of child names so `RestForOne` knows which
//!     siblings come after the dead child.
//!
//! The runtime drives this via three calls — `record_failure(child)`,
//! `should_restart(child)`, and `restart_targets(child)` — so the
//! policy is testable in isolation without booting actors.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartStrategy {
    /// Only the failing child is restarted.
    OneForOne,
    /// Every child is restarted when any one fails.
    OneForAll,
    /// The failing child and every child *started after it* are restarted.
    RestForOne,
}

impl RestartStrategy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "one_for_one" | "OneForOne" => Some(Self::OneForOne),
            "one_for_all" | "OneForAll" => Some(Self::OneForAll),
            "rest_for_one" | "RestForOne" => Some(Self::RestForOne),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Restart the listed children.
    Restart(Vec<String>),
    /// Failure budget exhausted within the window — escalate up the tree.
    Escalate { reason: String },
    /// Child name isn't known to this supervisor — bug or stale message.
    Unknown(String),
}

#[derive(Clone, Debug)]
pub struct Supervisor {
    pub name: String,
    pub strategy: RestartStrategy,
    pub max_restarts: u32,
    pub within_ns: i64,
    /// Ordered child names. Position matters for `RestForOne`.
    pub children: Vec<String>,
    /// Failure timestamps (ns since epoch), one per recorded failure,
    /// kept sorted ascending.
    pub failures: Vec<i64>,
    /// Whether the supervisor has already escalated. Once escalated it
    /// refuses to restart anything further.
    pub escalated: bool,
}

impl Supervisor {
    pub fn new(
        name: impl Into<String>,
        strategy: RestartStrategy,
        max_restarts: u32,
        within_ns: i64,
    ) -> Self {
        assert!(within_ns > 0, "within_ns must be positive");
        Self {
            name: name.into(),
            strategy,
            max_restarts,
            within_ns,
            children: Vec::new(),
            failures: Vec::new(),
            escalated: false,
        }
    }

    pub fn add_child(&mut self, name: impl Into<String>) {
        self.children.push(name.into());
    }

    /// Record a fresh failure at `now_ns` and return the supervisor's
    /// decision. Prunes failures older than `now_ns - within_ns` so the
    /// counter is a true sliding window.
    pub fn on_failure(&mut self, child: &str, now_ns: i64) -> Decision {
        if !self.children.iter().any(|c| c == child) {
            return Decision::Unknown(child.into());
        }
        if self.escalated {
            return Decision::Escalate {
                reason: "supervisor previously escalated; rejecting further failures".into(),
            };
        }
        // Prune old failures, then record this one.
        let cutoff = now_ns.saturating_sub(self.within_ns);
        self.failures.retain(|t| *t >= cutoff);
        self.failures.push(now_ns);

        if self.failures.len() as u32 > self.max_restarts {
            self.escalated = true;
            return Decision::Escalate {
                reason: format!(
                    "max_restarts={} exceeded within {} ns ({} failures in window)",
                    self.max_restarts,
                    self.within_ns,
                    self.failures.len()
                ),
            };
        }

        let targets = match self.strategy {
            RestartStrategy::OneForOne => vec![child.to_string()],
            RestartStrategy::OneForAll => self.children.clone(),
            RestartStrategy::RestForOne => {
                let start = self
                    .children
                    .iter()
                    .position(|c| c == child)
                    .unwrap_or(self.children.len());
                self.children[start..].to_vec()
            }
        };
        Decision::Restart(targets)
    }

    pub fn is_escalated(&self) -> bool {
        self.escalated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sup(strategy: RestartStrategy, max: u32, within_ns: i64) -> Supervisor {
        let mut s = Supervisor::new("svc", strategy, max, within_ns);
        for c in ["a", "b", "c", "d"] {
            s.add_child(c);
        }
        s
    }

    #[test]
    fn one_for_one_restarts_only_the_failing_child() {
        let mut s = sup(RestartStrategy::OneForOne, 10, 1_000_000_000);
        match s.on_failure("b", 0) {
            Decision::Restart(v) => assert_eq!(v, vec!["b".to_string()]),
            other => panic!("expected restart, got {other:?}"),
        }
    }

    #[test]
    fn one_for_all_restarts_every_child() {
        let mut s = sup(RestartStrategy::OneForAll, 10, 1_000_000_000);
        match s.on_failure("b", 0) {
            Decision::Restart(v) => {
                assert_eq!(v, vec!["a", "b", "c", "d"].iter().map(|x| x.to_string()).collect::<Vec<_>>());
            }
            other => panic!("expected restart, got {other:?}"),
        }
    }

    #[test]
    fn rest_for_one_restarts_failing_child_and_successors() {
        let mut s = sup(RestartStrategy::RestForOne, 10, 1_000_000_000);
        match s.on_failure("b", 0) {
            Decision::Restart(v) => {
                assert_eq!(v, vec!["b", "c", "d"].iter().map(|x| x.to_string()).collect::<Vec<_>>());
            }
            other => panic!("expected restart, got {other:?}"),
        }
    }

    #[test]
    fn unknown_child_is_reported() {
        let mut s = sup(RestartStrategy::OneForOne, 10, 1_000_000_000);
        assert!(matches!(
            s.on_failure("nope", 0),
            Decision::Unknown(_)
        ));
    }

    #[test]
    fn exceeding_max_restarts_within_window_escalates() {
        let mut s = sup(RestartStrategy::OneForOne, 2, 1_000_000_000);
        assert!(matches!(s.on_failure("a", 0), Decision::Restart(_)));
        assert!(matches!(s.on_failure("a", 100_000_000), Decision::Restart(_)));
        // Third failure within the 1s window → escalate.
        assert!(matches!(
            s.on_failure("a", 200_000_000),
            Decision::Escalate { .. }
        ));
        assert!(s.is_escalated());
    }

    #[test]
    fn failures_outside_window_are_forgotten() {
        let mut s = sup(RestartStrategy::OneForOne, 2, 1_000_000_000);
        let _ = s.on_failure("a", 0);
        let _ = s.on_failure("a", 500_000_000);
        // 2 seconds later: prior failures are outside the window.
        let _ = s.on_failure("a", 3_000_000_000);
        // We should still be allowed to restart; no escalation.
        assert!(!s.is_escalated());
        assert_eq!(s.failures.len(), 1);
    }

    #[test]
    fn after_escalation_subsequent_failures_stay_escalated() {
        let mut s = sup(RestartStrategy::OneForOne, 1, 1_000_000_000);
        let _ = s.on_failure("a", 0);
        assert!(matches!(s.on_failure("a", 1), Decision::Escalate { .. }));
        assert!(matches!(s.on_failure("a", 2), Decision::Escalate { .. }));
    }
}
