//! Deterministic simulation harness (§55.3).
//!
//! `sim "..."` blocks need a mock world: virtual clock, scripted agents,
//! and a step-deterministic scheduler so multi-agent runs are
//! reproducible byte-for-byte across CI machines.
//!
//! `World` owns:
//!   * a monotonic `clock_ns` that advances only on `World::advance`.
//!   * a seeded PRNG (`splitmix64`) so `World::rand_*` is reproducible.
//!   * a list of *agents* — each agent is a typed
//!     `Box<dyn ScriptedAgent>` that exposes `step(world) -> AgentAction`
//!     when its mailbox is non-empty or its periodic-tick fires.
//!   * an event log so tests can assert exact step sequences.
//!
//! Network/disk/random in user-space go through World so a `sim "..."`
//! run is fully reproducible.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimEvent {
    pub step: usize,
    /// Nanoseconds-since-start of this world's clock.
    pub at_ns: u64,
    pub agent: String,
    pub action: String,
    pub payload: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentBox {
    pub name: String,
    /// Each entry is a scripted message handler outcome — produced in
    /// FIFO order each time the agent is stepped. When empty the agent
    /// idles (does nothing on its turn).
    pub script: Vec<ScriptedAction>,
    pub mailbox: VecDeque<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptedAction {
    /// Send a message to `to` with payload `payload`.
    Send { to: String, payload: String },
    /// Record an arbitrary event without sending a message.
    Note { kind: String, payload: String },
    /// Mark this agent as `settled` (used by termination predicates).
    Settle,
}

pub struct World {
    pub seed: u64,
    pub start_ns: u64,
    pub clock_ns: u64,
    pub steps: usize,
    pub events: Vec<SimEvent>,
    pub agents: Vec<AgentBox>,
    pub settled: Vec<String>,
    rng_state: u64,
}

impl World {
    pub fn new(seed: u64, start_ns: u64) -> Self {
        Self {
            seed,
            start_ns,
            clock_ns: start_ns,
            steps: 0,
            events: Vec::new(),
            agents: Vec::new(),
            settled: Vec::new(),
            rng_state: seed,
        }
    }

    pub fn spawn(&mut self, name: impl Into<String>, script: Vec<ScriptedAction>) {
        self.agents.push(AgentBox {
            name: name.into(),
            script,
            mailbox: VecDeque::new(),
        });
    }

    /// Drop a message into the named agent's mailbox.
    pub fn send_to(
        &mut self,
        agent_name: &str,
        payload: impl Into<String>,
    ) -> Result<(), String> {
        let target = self
            .agents
            .iter_mut()
            .find(|a| a.name == agent_name)
            .ok_or_else(|| format!("sim: unknown agent `{agent_name}`"))?;
        target.mailbox.push_back(payload.into());
        Ok(())
    }

    /// Splitmix64 — small, fast, well-distributed; perfect for sim.
    pub fn rand_u64(&mut self) -> u64 {
        self.rng_state = self.rng_state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.rng_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Advance the clock by `dt_ns` and step every agent once in name
    /// order. Each step pops the agent's mailbox (if any) and consumes
    /// the next entry from its `script`. Returns the number of events
    /// recorded.
    pub fn advance(&mut self, dt_ns: u64) -> usize {
        self.clock_ns = self.clock_ns.saturating_add(dt_ns);
        self.steps += 1;
        let step = self.steps;
        let at_ns = self.clock_ns - self.start_ns;
        let mut new_events = Vec::new();
        let mut deliveries: Vec<(String, String, String)> = Vec::new(); // (to, payload, by)
        let mut newly_settled: Vec<String> = Vec::new();
        // Snapshot the agent order before mutating any of them.
        let order: Vec<String> = self.agents.iter().map(|a| a.name.clone()).collect();
        for name in order {
            // Look up by name each iteration so a previous-iter mutation
            // to scripts/mailbox is observable.
            let Some(idx) = self.agents.iter().position(|a| a.name == name) else {
                continue;
            };
            let agent = &mut self.agents[idx];
            // Drop any pending mailbox entry on this tick (real systems
            // would invoke a handler; the scripted shape advances anyway).
            let mailbox_drain = agent.mailbox.pop_front().unwrap_or_default();
            if agent.script.is_empty() {
                continue;
            }
            let action = agent.script.remove(0);
            match action {
                ScriptedAction::Send { to, payload } => {
                    new_events.push(SimEvent {
                        step,
                        at_ns,
                        agent: name.clone(),
                        action: "send".into(),
                        payload: format!("to={to};{payload}"),
                    });
                    deliveries.push((to, payload, name.clone()));
                }
                ScriptedAction::Note { kind, payload } => {
                    let pl = if mailbox_drain.is_empty() {
                        payload
                    } else {
                        format!("inbox={mailbox_drain};{payload}")
                    };
                    new_events.push(SimEvent {
                        step,
                        at_ns,
                        agent: name.clone(),
                        action: kind,
                        payload: pl,
                    });
                }
                ScriptedAction::Settle => {
                    new_events.push(SimEvent {
                        step,
                        at_ns,
                        agent: name.clone(),
                        action: "settle".into(),
                        payload: String::new(),
                    });
                    if !self.settled.contains(&name) && !newly_settled.contains(&name) {
                        newly_settled.push(name.clone());
                    }
                }
            }
        }
        for (to, payload, _by) in &deliveries {
            let _ = self.send_to(to, payload.clone());
        }
        self.settled.extend(newly_settled);
        let n = new_events.len();
        self.events.extend(new_events);
        n
    }

    /// Run until `predicate(&self)` is true or `max_ticks` ticks have
    /// elapsed; each tick is `dt_ns` long.
    pub fn run_until<P: Fn(&World) -> bool>(
        &mut self,
        dt_ns: u64,
        max_ticks: usize,
        predicate: P,
    ) -> bool {
        for _ in 0..max_ticks {
            if predicate(self) {
                return true;
            }
            self.advance(dt_ns);
        }
        predicate(self)
    }

    pub fn agent_settled(&self, name: &str) -> bool {
        self.settled.iter().any(|n| n == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buyer_seller_world() -> World {
        let mut w = World::new(42, 0);
        w.spawn(
            "buyer",
            vec![
                ScriptedAction::Send {
                    to: "seller".into(),
                    payload: "offer:80".into(),
                },
                ScriptedAction::Note {
                    kind: "negotiate".into(),
                    payload: "considering counter".into(),
                },
                ScriptedAction::Settle,
            ],
        );
        w.spawn(
            "seller",
            vec![
                ScriptedAction::Note {
                    kind: "wait".into(),
                    payload: "idle".into(),
                },
                ScriptedAction::Send {
                    to: "buyer".into(),
                    payload: "counter:110".into(),
                },
                ScriptedAction::Settle,
            ],
        );
        w
    }

    #[test]
    fn deterministic_seed_yields_repeatable_rng() {
        let mut a = World::new(123, 0);
        let mut b = World::new(123, 0);
        for _ in 0..5 {
            assert_eq!(a.rand_u64(), b.rand_u64());
        }
    }

    #[test]
    fn different_seed_diverges() {
        let mut a = World::new(1, 0);
        let mut b = World::new(2, 0);
        // First values should differ.
        assert_ne!(a.rand_u64(), b.rand_u64());
    }

    #[test]
    fn advance_records_events_in_agent_order() {
        let mut w = buyer_seller_world();
        let n = w.advance(1_000_000_000); // 1s tick
        assert_eq!(n, 2);
        assert_eq!(w.events[0].agent, "buyer");
        assert_eq!(w.events[1].agent, "seller");
        assert_eq!(w.clock_ns, 1_000_000_000);
    }

    #[test]
    fn deliveries_land_in_target_mailbox() {
        let mut w = buyer_seller_world();
        w.advance(1_000_000_000);
        // buyer sent offer:80; seller's mailbox should have it.
        let seller_idx = w.agents.iter().position(|a| a.name == "seller").unwrap();
        assert_eq!(
            w.agents[seller_idx].mailbox.front().map(|s| s.as_str()),
            Some("offer:80")
        );
    }

    #[test]
    fn run_until_stops_on_predicate() {
        let mut w = buyer_seller_world();
        let hit = w.run_until(1_000_000_000, 10, |w| w.agent_settled("buyer"));
        assert!(hit);
        // 3 ticks: send, note, settle.
        assert!(w.steps >= 3);
    }

    #[test]
    fn run_until_caps_at_max_ticks() {
        let mut w = World::new(0, 0);
        w.spawn(
            "x",
            vec![ScriptedAction::Note {
                kind: "loop".into(),
                payload: "p".into(),
            }],
        );
        let hit = w.run_until(1_000, 2, |_| false);
        assert!(!hit);
        assert_eq!(w.steps, 2);
    }

    #[test]
    fn idle_agent_records_no_event() {
        let mut w = World::new(0, 0);
        w.spawn("idle", Vec::new());
        let n = w.advance(1);
        assert_eq!(n, 0);
        assert!(w.events.is_empty());
    }
}
