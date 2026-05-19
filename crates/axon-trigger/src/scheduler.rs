//! In-process scheduler.
//!
//! Owns a `Vec<Trigger>` and exposes `tick(now_ns)` — returns the IDs of
//! triggers whose deadline has passed *and* updates each trigger's
//! `last_fired_ns`. The runtime is responsible for invoking the matching
//! handler; the scheduler is pure bookkeeping so it stays deterministic
//! and replayable.
//!
//! Persistence: `Scheduler::load_from_memory(store)` rehydrates a saved
//! scheduler from any [`axon_memory::Store`]; `save_to_memory(store)`
//! writes it back. Each call serializes the full trigger list under the
//! key `"_triggers"`. A restart picks up exactly where the last save
//! left off — `last_fired_ns` and `fail_count` survive.

use std::sync::Arc;

use axon_memory::{Entry, Store};
use serde::{Deserialize, Serialize};

use crate::Trigger;

const STORE_KEY: &str = "_triggers";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scheduler {
    pub triggers: Vec<Trigger>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FiredTrigger {
    pub id: String,
    pub handler: String,
    /// The fire time the scheduler used (may be in the past if the runtime
    /// missed multiple firings).
    pub fired_at_ns: i64,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            triggers: Vec::new(),
        }
    }

    pub fn add(&mut self, trigger: Trigger) -> Result<(), String> {
        if self.triggers.iter().any(|t| t.id == trigger.id) {
            return Err(format!("trigger id `{}` already exists", trigger.id));
        }
        self.triggers.push(trigger);
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.triggers.len();
        self.triggers.retain(|t| t.id != id);
        before != self.triggers.len()
    }

    pub fn get(&self, id: &str) -> Option<&Trigger> {
        self.triggers.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Trigger> {
        self.triggers.iter_mut().find(|t| t.id == id)
    }

    pub fn len(&self) -> usize {
        self.triggers.len()
    }
    pub fn is_empty(&self) -> bool {
        self.triggers.is_empty()
    }

    /// Advance the scheduler to `now_ns`. Returns the triggers that fired
    /// (each at most once per tick, in `id` order to be deterministic).
    ///
    /// `tick` does NOT invoke handlers — it only updates `last_fired_ns`
    /// and returns descriptors the caller uses to dispatch.
    pub fn tick(&mut self, now_ns: i64) -> Vec<FiredTrigger> {
        let mut to_fire: Vec<FiredTrigger> = Vec::new();
        for trigger in self.triggers.iter() {
            if let Some(fire_at) = trigger.due_at(now_ns) {
                to_fire.push(FiredTrigger {
                    id: trigger.id.clone(),
                    handler: trigger.handler.clone(),
                    fired_at_ns: fire_at,
                });
            }
        }
        to_fire.sort_by(|a, b| a.id.cmp(&b.id));
        for f in &to_fire {
            if let Some(t) = self.get_mut(&f.id) {
                t.mark_fired(f.fired_at_ns);
            }
        }
        to_fire
    }

    pub fn save_to_memory(&self, store: &Arc<dyn Store>) -> Result<(), String> {
        let value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        store
            .set(STORE_KEY, Entry::new(value))
            .map_err(|e| e.to_string())
    }

    pub fn load_from_memory(store: &Arc<dyn Store>) -> Result<Self, String> {
        match store.get(STORE_KEY).map_err(|e| e.to_string())? {
            Some(entry) => serde_json::from_value(entry.value).map_err(|e| e.to_string()),
            None => Ok(Self::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schedule;
    use axon_memory::EphemeralStore;

    #[test]
    fn tick_fires_due_triggers_in_id_order() {
        let mut s = Scheduler::new();
        s.add(Trigger::new("b", "h_b", Schedule::every_seconds(10)))
            .unwrap();
        s.add(Trigger::new("a", "h_a", Schedule::every_seconds(10)))
            .unwrap();
        // 30s into the future — both due, fired in id order.
        let fired = s.tick(30_000_000_000);
        assert_eq!(fired.len(), 2);
        assert_eq!(fired[0].id, "a");
        assert_eq!(fired[1].id, "b");
    }

    #[test]
    fn tick_does_not_refire_within_the_same_period() {
        let mut s = Scheduler::new();
        s.add(Trigger::new(
            "t1",
            "h",
            Schedule::every_seconds(60),
        ))
        .unwrap();
        let f1 = s.tick(0); // fire at t=0
        assert_eq!(f1.len(), 1);
        let f2 = s.tick(30_000_000_000); // 30s later — too early
        assert_eq!(f2.len(), 0);
        let f3 = s.tick(70_000_000_000); // 70s later — due
        assert_eq!(f3.len(), 1);
    }

    #[test]
    fn at_trigger_fires_once_then_never_again() {
        let mut s = Scheduler::new();
        s.add(Trigger::new(
            "once",
            "h",
            Schedule::At {
                when_ns: 50_000_000_000,
            },
        ))
        .unwrap();
        let f1 = s.tick(60_000_000_000);
        assert_eq!(f1.len(), 1);
        let f2 = s.tick(120_000_000_000);
        assert_eq!(f2.len(), 0);
    }

    #[test]
    fn save_and_load_through_memory_store_survives() {
        let store: Arc<dyn Store> = Arc::new(EphemeralStore::new());
        let mut s = Scheduler::new();
        s.add(Trigger::new("t", "h", Schedule::every_seconds(60)))
            .unwrap();
        s.tick(0);
        s.save_to_memory(&store).unwrap();

        let restored = Scheduler::load_from_memory(&store).unwrap();
        assert_eq!(restored.triggers.len(), 1);
        assert_eq!(restored.triggers[0].last_fired_ns, Some(0));
    }

    #[test]
    fn add_rejects_duplicate_ids() {
        let mut s = Scheduler::new();
        s.add(Trigger::new("t", "h", Schedule::every_seconds(60)))
            .unwrap();
        assert!(s
            .add(Trigger::new("t", "h", Schedule::every_seconds(60)))
            .is_err());
    }

    #[test]
    fn remove_returns_whether_removed() {
        let mut s = Scheduler::new();
        s.add(Trigger::new("t", "h", Schedule::every_seconds(60)))
            .unwrap();
        assert!(s.remove("t"));
        assert!(!s.remove("t"));
    }
}
