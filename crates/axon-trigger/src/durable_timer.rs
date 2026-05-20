//! Durable timers — `sleep_until(deadline_ns)` that survives restarts
//! (§52.2).
//!
//! A regular `std::thread::sleep` evaporates with the process. A durable
//! timer is *checkpointed* to a [`axon_memory::Store`]: a process that
//! restarts mid-sleep reads the timer table, refuses to call the
//! caller's continuation until the wall clock has actually advanced
//! past `deadline_ns`, and then fires.
//!
//! The library is bookkeeping-only: it owns no clock, doesn't sleep,
//! doesn't spawn threads. The runtime checks `DurableTimerTable::due`
//! on every tick and fires whichever timer's deadline has passed.
//!
//! Persistence layout (in the memory store, keyed `axon.timers`):
//!
//! ```text
//! { "v": 1, "timers": [
//!     { "id": "t-abc", "name": "wakeup_payroll", "deadline_ns": 1781000000000000000,
//!       "armed_ns": 1779000000000000000, "fired": false, "cancelled": false },
//!     ...
//! ] }
//! ```
//!
//! Combined with the existing `Scheduler`/`Trigger`, durable timers
//! cover §52.2's `sleep_until` and the long-tailed
//! "wake-an-agent-up-Monday-morning" pattern.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const TIMER_TABLE_KEY: &str = "axon.timers";
pub const TIMER_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableTimer {
    pub id: String,
    pub name: String,
    pub deadline_ns: i64,
    pub armed_ns: i64,
    #[serde(default)]
    pub fired: bool,
    #[serde(default)]
    pub cancelled: bool,
    /// Free-form payload — typically a JSON blob the runtime hands to
    /// the continuation when the timer fires.
    #[serde(default)]
    pub payload: String,
}

impl DurableTimer {
    pub fn is_due(&self, now_ns: i64) -> bool {
        !self.fired && !self.cancelled && now_ns >= self.deadline_ns
    }

    pub fn is_pending(&self) -> bool {
        !self.fired && !self.cancelled
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DurableTimerTable {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub timers: BTreeMap<String, DurableTimer>,
}

impl DurableTimerTable {
    pub fn new() -> Self {
        Self {
            version: TIMER_FORMAT_VERSION,
            timers: BTreeMap::new(),
        }
    }

    pub fn arm(&mut self, t: DurableTimer) -> Result<(), String> {
        if t.id.is_empty() {
            return Err("durable timer id must be non-empty".into());
        }
        if self.timers.contains_key(&t.id) {
            return Err(format!("durable timer `{}` already armed", t.id));
        }
        self.timers.insert(t.id.clone(), t);
        Ok(())
    }

    pub fn cancel(&mut self, id: &str) -> bool {
        if let Some(t) = self.timers.get_mut(id) {
            if t.is_pending() {
                t.cancelled = true;
                return true;
            }
        }
        false
    }

    /// Return all timer IDs whose deadline has passed and which haven't
    /// fired yet, sorted by deadline ascending. The caller is expected
    /// to call `mark_fired` after running the continuation.
    pub fn due(&self, now_ns: i64) -> Vec<String> {
        let mut due: Vec<(&DurableTimer)> = self
            .timers
            .values()
            .filter(|t| t.is_due(now_ns))
            .collect();
        due.sort_by_key(|t| t.deadline_ns);
        due.into_iter().map(|t| t.id.clone()).collect()
    }

    pub fn mark_fired(&mut self, id: &str) -> bool {
        if let Some(t) = self.timers.get_mut(id) {
            if t.is_pending() {
                t.fired = true;
                return true;
            }
        }
        false
    }

    pub fn pending_count(&self) -> usize {
        self.timers.values().filter(|t| t.is_pending()).count()
    }

    pub fn purge_fired_or_cancelled(&mut self) -> usize {
        let before = self.timers.len();
        self.timers.retain(|_, t| t.is_pending());
        before - self.timers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, deadline_ns: i64) -> DurableTimer {
        DurableTimer {
            id: id.into(),
            name: format!("name-{id}"),
            deadline_ns,
            armed_ns: 0,
            fired: false,
            cancelled: false,
            payload: String::new(),
        }
    }

    #[test]
    fn due_returns_only_passed_deadlines_in_order() {
        let mut tbl = DurableTimerTable::new();
        tbl.arm(t("c", 3000)).unwrap();
        tbl.arm(t("a", 1000)).unwrap();
        tbl.arm(t("b", 2000)).unwrap();
        let now = 2500;
        let due = tbl.due(now);
        assert_eq!(due, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn mark_fired_prevents_redelivery() {
        let mut tbl = DurableTimerTable::new();
        tbl.arm(t("x", 100)).unwrap();
        assert_eq!(tbl.due(200), vec!["x".to_string()]);
        assert!(tbl.mark_fired("x"));
        assert!(tbl.due(200).is_empty());
        // Fired again is a no-op.
        assert!(!tbl.mark_fired("x"));
    }

    #[test]
    fn cancel_drops_a_pending_timer() {
        let mut tbl = DurableTimerTable::new();
        tbl.arm(t("x", 1000)).unwrap();
        assert!(tbl.cancel("x"));
        assert!(tbl.due(2000).is_empty());
        assert_eq!(tbl.pending_count(), 0);
    }

    #[test]
    fn duplicate_arm_rejected() {
        let mut tbl = DurableTimerTable::new();
        tbl.arm(t("dup", 1)).unwrap();
        assert!(tbl.arm(t("dup", 2)).is_err());
    }

    #[test]
    fn purge_drops_fired_and_cancelled() {
        let mut tbl = DurableTimerTable::new();
        tbl.arm(t("a", 1)).unwrap();
        tbl.arm(t("b", 2)).unwrap();
        tbl.arm(t("c", 3)).unwrap();
        tbl.mark_fired("a");
        tbl.cancel("b");
        let n = tbl.purge_fired_or_cancelled();
        assert_eq!(n, 2);
        assert_eq!(tbl.pending_count(), 1);
    }

    #[test]
    fn json_round_trip_preserves_table() {
        let mut tbl = DurableTimerTable::new();
        tbl.arm(t("a", 100)).unwrap();
        tbl.arm(t("b", 200)).unwrap();
        tbl.mark_fired("a");
        let j = serde_json::to_string(&tbl).unwrap();
        let back: DurableTimerTable = serde_json::from_str(&j).unwrap();
        assert_eq!(back, tbl);
    }

    #[test]
    fn empty_id_rejected() {
        let mut tbl = DurableTimerTable::new();
        assert!(tbl.arm(t("", 1)).is_err());
    }
}
