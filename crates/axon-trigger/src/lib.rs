//! `axon-trigger` — schedule expressions, triggers, and a durable
//! in-process scheduler.
//!
//! Stage 14 surface for §52:
//!
//!   * [`Schedule`] — `Every(period)`, `At(instant)`, `Cron(expr)` —
//!     the *what fires when* descriptor.
//!   * [`Trigger`] — handler name + schedule + retry/backoff policy +
//!     bookkeeping (`last_fired_ns`, `fail_count`).
//!   * [`Scheduler`] — owns a `Vec<Trigger>`, persists state to an
//!     [`axon_memory::Store`], and exposes `tick(now_ns)` which returns
//!     the IDs of triggers whose deadline has passed. The runtime
//!     decides how to *run* the handler — the scheduler is pure
//!     bookkeeping so it stays deterministic and replayable.
//!
//! The cron parser implements the 5-field POSIX subset
//! (`min hour dom mon dow`) with `*`, comma lists, `a-b` ranges, and
//! `*/N` step values. That covers >95% of real cron expressions; weird
//! niceties like `@reboot` or seconds-precision are out of scope for v0.

pub mod cron;
pub mod durable_timer;
mod scheduler;
mod schedule;
mod trigger;

pub use cron::CronExpr;
pub use durable_timer::{DurableTimer, DurableTimerTable, TIMER_FORMAT_VERSION, TIMER_TABLE_KEY};
pub use schedule::Schedule;
pub use scheduler::{Scheduler, FiredTrigger};
pub use trigger::{RetryPolicy, Trigger, TriggerError};
