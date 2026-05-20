//! Stream runtime (§28).
//!
//! `Stream<T>` is the asynchronous-iteration counterpart of `List<T>`.
//! Producers push values with `send`; consumers pull values with `take`.
//! `close()` signals end-of-stream; subsequent `take()`s return `None`.
//!
//! The library is a bounded MPMC ring with optional backpressure: a
//! producer that hits the buffer cap blocks (returns `Backpressure`)
//! until a consumer drains. The synchronous runtime treats blocking
//! as a graceful "try again" — async schedulers can suspend the
//! producer instead.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamHandle {
    pub name: String,
    pub buffer: VecDeque<serde_json::Value>,
    pub capacity: usize,
    pub closed: bool,
    /// Telemetry — total messages ever sent, taken, dropped (when
    /// `policy = DropOldest` is enabled).
    pub sent: u64,
    pub taken: u64,
    pub dropped: u64,
    /// Backpressure policy when `buffer.len() >= capacity`.
    pub policy: BackpressurePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackpressurePolicy {
    /// Producer's `send` returns `SendOutcome::Backpressure` — caller
    /// can retry.
    Block,
    /// Drop the *oldest* buffered value to make room for the new one.
    /// `dropped` increments.
    DropOldest,
    /// Drop the *new* value silently. `dropped` increments.
    DropNew,
}

impl Default for BackpressurePolicy {
    fn default() -> Self {
        BackpressurePolicy::Block
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    /// Value buffered.
    Buffered,
    /// Stream is closed; value rejected.
    Closed,
    /// Buffer is full and policy is `Block`.
    Backpressure,
    /// Value buffered after dropping one to make room.
    DroppedOldest,
    /// Value rejected, buffer was full and policy is `DropNew`.
    DroppedNew,
}

impl StreamHandle {
    pub fn new(name: impl Into<String>, capacity: usize, policy: BackpressurePolicy) -> Self {
        Self {
            name: name.into(),
            buffer: VecDeque::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
            closed: false,
            sent: 0,
            taken: 0,
            dropped: 0,
            policy,
        }
    }

    pub fn send(&mut self, value: serde_json::Value) -> SendOutcome {
        if self.closed {
            return SendOutcome::Closed;
        }
        if self.buffer.len() >= self.capacity {
            return match self.policy {
                BackpressurePolicy::Block => SendOutcome::Backpressure,
                BackpressurePolicy::DropOldest => {
                    self.buffer.pop_front();
                    self.buffer.push_back(value);
                    self.sent += 1;
                    self.dropped += 1;
                    SendOutcome::DroppedOldest
                }
                BackpressurePolicy::DropNew => {
                    self.dropped += 1;
                    SendOutcome::DroppedNew
                }
            };
        }
        self.buffer.push_back(value);
        self.sent += 1;
        SendOutcome::Buffered
    }

    /// Pop the front value. Returns `None` when the buffer is empty;
    /// the consumer should distinguish "empty but open" from "closed
    /// and drained" via [`Self::is_done`].
    pub fn take(&mut self) -> Option<serde_json::Value> {
        let v = self.buffer.pop_front();
        if v.is_some() {
            self.taken += 1;
        }
        v
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    /// `true` when no more values will ever arrive — the stream is
    /// closed *and* the buffer is empty. `for await` loops use this
    /// as the termination check.
    pub fn is_done(&self) -> bool {
        self.closed && self.buffer.is_empty()
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> StreamHandle {
        StreamHandle::new("test", 3, BackpressurePolicy::Block)
    }

    #[test]
    fn send_then_take_round_trips() {
        let mut h = s();
        assert_eq!(h.send(serde_json::json!(1)), SendOutcome::Buffered);
        assert_eq!(h.send(serde_json::json!(2)), SendOutcome::Buffered);
        assert_eq!(h.take(), Some(serde_json::json!(1)));
        assert_eq!(h.take(), Some(serde_json::json!(2)));
        assert_eq!(h.take(), None);
    }

    #[test]
    fn closed_stream_rejects_send() {
        let mut h = s();
        h.close();
        assert_eq!(h.send(serde_json::json!(1)), SendOutcome::Closed);
    }

    #[test]
    fn block_policy_signals_backpressure() {
        let mut h = StreamHandle::new("t", 2, BackpressurePolicy::Block);
        h.send(serde_json::json!(1));
        h.send(serde_json::json!(2));
        assert_eq!(h.send(serde_json::json!(3)), SendOutcome::Backpressure);
    }

    #[test]
    fn drop_oldest_keeps_newest_value() {
        let mut h = StreamHandle::new("t", 2, BackpressurePolicy::DropOldest);
        h.send(serde_json::json!(1));
        h.send(serde_json::json!(2));
        assert_eq!(h.send(serde_json::json!(3)), SendOutcome::DroppedOldest);
        assert_eq!(h.take(), Some(serde_json::json!(2)));
        assert_eq!(h.take(), Some(serde_json::json!(3)));
        assert_eq!(h.dropped, 1);
    }

    #[test]
    fn drop_new_policy_silently_drops() {
        let mut h = StreamHandle::new("t", 2, BackpressurePolicy::DropNew);
        h.send(serde_json::json!(1));
        h.send(serde_json::json!(2));
        assert_eq!(h.send(serde_json::json!(3)), SendOutcome::DroppedNew);
        // Buffer is unchanged.
        assert_eq!(h.take(), Some(serde_json::json!(1)));
        assert_eq!(h.take(), Some(serde_json::json!(2)));
        assert_eq!(h.take(), None);
    }

    #[test]
    fn is_done_distinguishes_empty_vs_drained() {
        let mut h = s();
        // Empty + open.
        assert!(!h.is_done());
        h.send(serde_json::json!(1));
        h.close();
        // Closed but value still buffered.
        assert!(!h.is_done());
        h.take();
        // Closed + drained.
        assert!(h.is_done());
    }

    #[test]
    fn telemetry_counters_track_send_take_drop() {
        let mut h = StreamHandle::new("t", 1, BackpressurePolicy::DropOldest);
        h.send(serde_json::json!(1));
        h.send(serde_json::json!(2));
        h.take();
        assert_eq!(h.sent, 2);
        assert_eq!(h.taken, 1);
        assert_eq!(h.dropped, 1);
    }

    #[test]
    fn round_trip_through_json() {
        let mut h = s();
        h.send(serde_json::json!("hi"));
        let s = serde_json::to_string(&h).unwrap();
        let back: StreamHandle = serde_json::from_str(&s).unwrap();
        assert_eq!(back, h);
    }
}
