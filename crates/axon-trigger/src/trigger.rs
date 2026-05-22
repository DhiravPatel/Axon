//! Trigger record + retry policy.

use serde::{Deserialize, Serialize};

use crate::Schedule;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff_ns: i64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_ns: 5 * 1_000_000_000, // 5 seconds
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trigger {
    pub id: String,
    pub handler: String,
    pub schedule: Schedule,
    #[serde(default)]
    pub retry: RetryPolicy,
    /// Last successful fire time (ns since epoch). `None` if it has never
    /// fired successfully.
    #[serde(default)]
    pub last_fired_ns: Option<i64>,
    /// Consecutive failures since the last success. Used by the retry
    /// policy to decide when to give up.
    #[serde(default)]
    pub fail_count: u32,
    /// If `true`, the scheduler skips this trigger but keeps the record.
    #[serde(default)]
    pub disabled: bool,
}

impl Trigger {
    pub fn new(id: impl Into<String>, handler: impl Into<String>, schedule: Schedule) -> Self {
        Self {
            id: id.into(),
            handler: handler.into(),
            schedule,
            retry: RetryPolicy::default(),
            last_fired_ns: None,
            fail_count: 0,
            disabled: false,
        }
    }

    pub fn due_at(&self, now_ns: i64) -> Option<i64> {
        if self.disabled {
            return None;
        }
        self.schedule.due_at(self.last_fired_ns, now_ns)
    }

    pub fn mark_fired(&mut self, when_ns: i64) {
        self.last_fired_ns = Some(when_ns);
        self.fail_count = 0;
    }

    pub fn mark_failed(&mut self) -> TriggerError {
        self.fail_count += 1;
        if self.fail_count >= self.retry.max_attempts {
            self.disabled = true;
            TriggerError::Exhausted {
                attempts: self.fail_count,
            }
        } else {
            TriggerError::Backoff {
                attempt: self.fail_count,
                next_in_ns: self.retry.backoff_ns,
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerError {
    Backoff { attempt: u32, next_in_ns: i64 },
    Exhausted { attempts: u32 },
}
