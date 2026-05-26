//! Schema migration chains.
//!
//! Long-lived agents persist values whose `schema` shape evolves. A
//! `Migrator` owns an ordered list of per-version transforms that upgrade
//! a value from its stored version up to the current one.
//!
//! Each step is keyed by the *source* version: a step keyed `from = 2`
//! takes a v2-shaped value and produces a v3-shaped one. Loading a v1
//! value into a v3 schema walks `step[from=1] → step[from=2]` in order.
//!
//! The transforms themselves are language values: the host crate stores a
//! `Value::Fn`/`Value::NativeExt` per step and invokes them through
//! `Interpreter::call_value`, so users write their migration logic in
//! plain Axon. This module is only concerned with the **plumbing**:
//! gap detection, ordering, and clean error messages.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationError {
    /// No step keyed at `from = V` was registered, and the stored value
    /// has version `V < current`. The user forgot a step.
    Missing { from_version: u32 },
    /// Stored value's version is higher than the schema's current — the
    /// program is older than its data and refuses to "downgrade".
    Downgrade {
        stored: u32,
        current: u32,
    },
    /// A step itself errored; the inner string is the underlying message.
    Step { from_version: u32, message: String },
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrationError::Missing { from_version } => write!(
                f,
                "no migration step registered for `from = {from_version}`"
            ),
            MigrationError::Downgrade { stored, current } => write!(
                f,
                "stored value is v{stored} but schema is v{current} \
                — refusing to downgrade (rebuild your binary to the newer schema)"
            ),
            MigrationError::Step {
                from_version,
                message,
            } => write!(f, "migration step from v{from_version} failed: {message}"),
        }
    }
}

impl std::error::Error for MigrationError {}

/// A registered migration step. The handler is opaque here; the host
/// crate fills in the actual invocation (we store the version index, the
/// host stores the callable keyed by `(schema, from)`).
#[derive(Clone, Debug)]
pub struct StepRecord {
    pub from_version: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Migrator {
    pub schema: String,
    /// Schema's *current* version (i.e. what stored values get upgraded TO).
    pub current_version: u32,
    /// Steps keyed by source version, sorted ascending.
    pub steps: BTreeMap<u32, StepRecord>,
}

impl Migrator {
    pub fn new(schema: impl Into<String>, current_version: u32) -> Self {
        Self {
            schema: schema.into(),
            current_version,
            steps: BTreeMap::new(),
        }
    }

    pub fn add_step(&mut self, from_version: u32) -> Result<(), String> {
        if from_version >= self.current_version {
            return Err(format!(
                "cannot add a step keyed `from = {from_version}` for a v{current} schema",
                current = self.current_version
            ));
        }
        if self.steps.contains_key(&from_version) {
            return Err(format!("step from v{from_version} already registered"));
        }
        self.steps.insert(from_version, StepRecord { from_version });
        Ok(())
    }

    /// Plan the chain of step keys to walk for a stored value at
    /// `from_version`. Returns:
    ///
    ///   * `Ok(vec![])` if `from_version == current_version` (no-op).
    ///   * `Err(Downgrade)` if `from_version > current_version`.
    ///   * `Err(Missing)` if any step in the path is unregistered.
    pub fn plan(&self, from_version: u32) -> Result<Vec<u32>, MigrationError> {
        if from_version > self.current_version {
            return Err(MigrationError::Downgrade {
                stored: from_version,
                current: self.current_version,
            });
        }
        let mut out = Vec::new();
        let mut v = from_version;
        while v < self.current_version {
            if !self.steps.contains_key(&v) {
                return Err(MigrationError::Missing { from_version: v });
            }
            out.push(v);
            v += 1;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_op_when_already_current() {
        let m = Migrator::new("Profile", 3);
        assert_eq!(m.plan(3).unwrap(), Vec::<u32>::new());
    }

    #[test]
    fn plans_consecutive_steps() {
        let mut m = Migrator::new("Profile", 3);
        m.add_step(1).unwrap();
        m.add_step(2).unwrap();
        assert_eq!(m.plan(1).unwrap(), vec![1, 2]);
        assert_eq!(m.plan(2).unwrap(), vec![2]);
    }

    #[test]
    fn missing_step_is_reported() {
        let mut m = Migrator::new("Profile", 3);
        m.add_step(2).unwrap();
        // No v1 → v2 step → planning v1 fails.
        assert!(matches!(
            m.plan(1),
            Err(MigrationError::Missing { from_version: 1 })
        ));
    }

    #[test]
    fn downgrade_is_refused() {
        let m = Migrator::new("Profile", 2);
        assert!(matches!(
            m.plan(5),
            Err(MigrationError::Downgrade {
                stored: 5,
                current: 2
            })
        ));
    }

    #[test]
    fn add_step_rejects_invalid_versions() {
        let mut m = Migrator::new("Profile", 3);
        assert!(m.add_step(3).is_err(), "from == current is illegal");
        assert!(m.add_step(4).is_err(), "from > current is illegal");
        m.add_step(1).unwrap();
        assert!(m.add_step(1).is_err(), "duplicate keys rejected");
    }
}
