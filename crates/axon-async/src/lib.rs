//! `axon-async` — foundation for the future async AxVM.
//!
//! The current AxVM is a synchronous tree-walking interpreter. Agent
//! workloads are I/O-bound (model calls take 1–3 seconds), so the
//! whole leverage of `spawn`, `parallel`, `race`, and `with scope` is
//! overlapping those waits. The synchronous mailbox model means a
//! 5-agent pipeline serializes what should run concurrently.
//!
//! This crate is the **scaffolding** for the rewrite — not the
//! rewrite itself. It ships:
//!
//!   * [`AsyncRuntime`] — a thin tokio multi-thread runtime wrapper
//!     with deterministic shutdown.
//!   * [`AsyncMailbox<T>`] — bounded MPMC channel suited to actor
//!     message delivery, with backpressure semantics that match the
//!     spec's `Stream<T>` (`Block` / `DropOldest` / `DropNew`).
//!   * [`Task`] — a typed handle to a spawned green task, with
//!     cancellation + structured-concurrency `join_all`.
//!   * [`AsyncBudget`] — wall-clock cancellation so a stalled handler
//!     doesn't hold up `with budget(wall = 30s)`.
//!
//! What this crate is **NOT**: it doesn't migrate `eval.rs` to async.
//! That's a multi-week rewrite — every host binding, every `call_value`,
//! every channel, every model call. The scaffolding is here so when
//! the migration happens, the contract is settled.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackpressurePolicy {
    /// Producer awaits until there's space in the buffer.
    Block,
    /// Drop the oldest buffered value to make room for the new one.
    DropOldest,
    /// Drop the new value when the buffer is full.
    DropNew,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncError {
    /// The runtime was shut down while a task was still running.
    ShutDown,
    /// A `with budget(wall = …)` exceeded its wall-clock limit.
    WallClockExceeded { limit_ms: u64 },
    /// A task panicked.
    Panicked(String),
    /// A send was attempted on a closed mailbox.
    MailboxClosed,
}

impl std::fmt::Display for AsyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsyncError::ShutDown => write!(f, "runtime is shut down"),
            AsyncError::WallClockExceeded { limit_ms } => {
                write!(f, "wall-clock budget exceeded ({limit_ms} ms)")
            }
            AsyncError::Panicked(s) => write!(f, "task panicked: {s}"),
            AsyncError::MailboxClosed => write!(f, "mailbox is closed"),
        }
    }
}

impl std::error::Error for AsyncError {}

/// Tokio-backed runtime. Hold one per process; the runtime owns its
/// own worker threads (default = num_cpus). Dropping the runtime
/// triggers a graceful shutdown.
pub struct AsyncRuntime {
    inner: tokio::runtime::Runtime,
}

impl AsyncRuntime {
    pub fn new() -> std::io::Result<Self> {
        let inner = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        Ok(Self { inner })
    }

    /// Block the calling (sync) thread on an async future. The future
    /// runs on the runtime's worker pool.
    pub fn block_on<F: std::future::Future>(&self, fut: F) -> F::Output {
        self.inner.block_on(fut)
    }

    /// Spawn a top-level task. Returns a [`Task`] handle for joining
    /// or cancelling.
    pub fn spawn<F, T>(&self, fut: F) -> Task<T>
    where
        F: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let handle = self.inner.spawn(fut);
        Task {
            inner: Arc::new(TaskInnerT {
                handle: tokio::sync::Mutex::new(Some(handle)),
            }),
        }
    }

    /// Apply a wall-clock budget. Returns `Err(WallClockExceeded)` if
    /// the future doesn't complete in time.
    pub async fn with_wall_budget<F, T>(
        &self,
        limit_ms: u64,
        fut: F,
    ) -> Result<T, AsyncError>
    where
        F: std::future::Future<Output = T>,
    {
        match tokio::time::timeout(Duration::from_millis(limit_ms), fut).await {
            Ok(v) => Ok(v),
            Err(_) => Err(AsyncError::WallClockExceeded { limit_ms }),
        }
    }
}

struct TaskInnerT<T> {
    handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<T>>>,
}

/// A handle to a spawned task. Drop to detach; call `join` to await
/// completion; call `cancel` to abort.
pub struct Task<T> {
    inner: Arc<TaskInnerT<T>>,
}

impl<T: Send + 'static> Task<T> {
    /// Wait for the task to complete.
    pub async fn join(self) -> Result<T, AsyncError> {
        let handle = {
            let mut guard = self.inner.handle.lock().await;
            guard.take()
        };
        match handle {
            Some(h) => match h.await {
                Ok(v) => Ok(v),
                Err(e) if e.is_cancelled() => Err(AsyncError::ShutDown),
                Err(e) => Err(AsyncError::Panicked(e.to_string())),
            },
            None => Err(AsyncError::ShutDown),
        }
    }

    /// Cancel the task. The task may not stop immediately; this just
    /// signals cancellation.
    pub fn cancel(&self) {
        if let Ok(mut guard) = self.inner.handle.try_lock() {
            if let Some(h) = guard.as_ref() {
                h.abort();
            }
            *guard = None;
        }
    }
}


/// Bounded async mailbox with selectable backpressure. Producers
/// hand off `T`s; consumers `recv` them; on overflow the configured
/// policy decides.
pub struct AsyncMailbox<T> {
    tx: mpsc::Sender<T>,
    rx: tokio::sync::Mutex<mpsc::Receiver<T>>,
    policy: BackpressurePolicy,
    capacity: usize,
}

impl<T: Send + 'static> AsyncMailbox<T> {
    pub fn new(capacity: usize, policy: BackpressurePolicy) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        Arc::new(Self {
            tx,
            rx: tokio::sync::Mutex::new(rx),
            policy,
            capacity,
        })
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub async fn send(&self, value: T) -> Result<SendOutcome, AsyncError> {
        match self.policy {
            BackpressurePolicy::Block => {
                self.tx
                    .send(value)
                    .await
                    .map(|_| SendOutcome::Buffered)
                    .map_err(|_| AsyncError::MailboxClosed)
            }
            BackpressurePolicy::DropOldest => match self.tx.try_send(value) {
                Ok(()) => Ok(SendOutcome::Buffered),
                Err(mpsc::error::TrySendError::Full(value)) => {
                    // Pop the oldest, push the new. `try_recv` is
                    // non-blocking — we hold the receiver lock briefly.
                    let mut rx = self.rx.lock().await;
                    let _ = rx.try_recv();
                    drop(rx);
                    self.tx
                        .send(value)
                        .await
                        .map(|_| SendOutcome::DroppedOldest)
                        .map_err(|_| AsyncError::MailboxClosed)
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Err(AsyncError::MailboxClosed),
            },
            BackpressurePolicy::DropNew => match self.tx.try_send(value) {
                Ok(()) => Ok(SendOutcome::Buffered),
                Err(mpsc::error::TrySendError::Full(_)) => Ok(SendOutcome::DroppedNew),
                Err(mpsc::error::TrySendError::Closed(_)) => Err(AsyncError::MailboxClosed),
            },
        }
    }

    pub async fn recv(&self) -> Option<T> {
        let mut rx = self.rx.lock().await;
        rx.recv().await
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Buffered,
    DroppedOldest,
    DroppedNew,
}

/// Run a set of futures concurrently to completion (structured
/// concurrency `join_all`). Returns each result in the input order.
/// If any future errors the others continue — collect errors per slot.
pub async fn join_all<F, T>(futs: Vec<F>) -> Vec<Result<T, AsyncError>>
where
    F: std::future::Future<Output = Result<T, AsyncError>> + Send + 'static,
    T: Send + 'static,
{
    let mut handles: Vec<oneshot::Receiver<Result<T, AsyncError>>> =
        Vec::with_capacity(futs.len());
    for fut in futs {
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let r = fut.await;
            let _ = tx.send(r);
        });
        handles.push(rx);
    }
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(
            h.await
                .unwrap_or(Err(AsyncError::Panicked("task vanished".into()))),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn runtime_spawns_and_joins_task() {
        let rt = AsyncRuntime::new().unwrap();
        let task = rt.spawn(async { 42i32 });
        let v = rt.block_on(async move { task.join().await }).unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn join_all_collects_in_order() {
        let rt = AsyncRuntime::new().unwrap();
        let results = rt.block_on(join_all(vec![
            Box::pin(async { Ok::<_, AsyncError>(1) }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<i32, AsyncError>> + Send>>,
            Box::pin(async { Ok::<_, AsyncError>(2) }),
            Box::pin(async { Ok::<_, AsyncError>(3) }),
        ]));
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].as_ref().ok(), Some(&1));
        assert_eq!(results[2].as_ref().ok(), Some(&3));
    }

    #[test]
    fn wall_budget_times_out() {
        let rt = AsyncRuntime::new().unwrap();
        let r = rt.block_on(async {
            rt.with_wall_budget(20, async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                42i32
            })
            .await
        });
        assert!(matches!(r, Err(AsyncError::WallClockExceeded { .. })));
    }

    #[test]
    fn mailbox_blocks_then_delivers() {
        let rt = AsyncRuntime::new().unwrap();
        let mb = AsyncMailbox::<i32>::new(2, BackpressurePolicy::Block);
        let mb_clone = mb.clone();
        rt.block_on(async move {
            mb_clone.send(1).await.unwrap();
            mb_clone.send(2).await.unwrap();
            assert_eq!(mb_clone.recv().await, Some(1));
            assert_eq!(mb_clone.recv().await, Some(2));
        });
    }

    #[test]
    fn drop_new_policy_drops_when_full() {
        let rt = AsyncRuntime::new().unwrap();
        let mb = AsyncMailbox::<i32>::new(1, BackpressurePolicy::DropNew);
        let mb_clone = mb.clone();
        rt.block_on(async move {
            let r1 = mb_clone.send(1).await.unwrap();
            let r2 = mb_clone.send(2).await.unwrap();
            assert_eq!(r1, SendOutcome::Buffered);
            assert_eq!(r2, SendOutcome::DroppedNew);
            assert_eq!(mb_clone.recv().await, Some(1));
        });
    }

    #[test]
    fn drop_oldest_policy_replaces_buffered() {
        let rt = AsyncRuntime::new().unwrap();
        let mb = AsyncMailbox::<i32>::new(1, BackpressurePolicy::DropOldest);
        let mb_clone = mb.clone();
        rt.block_on(async move {
            mb_clone.send(1).await.unwrap();
            let r = mb_clone.send(2).await.unwrap();
            assert_eq!(r, SendOutcome::DroppedOldest);
            // We popped one in the drop-oldest path, then pushed the
            // new; the buffer holds {2}.
            assert_eq!(mb_clone.recv().await, Some(2));
        });
    }

    #[test]
    fn task_cancel_is_idempotent() {
        let rt = AsyncRuntime::new().unwrap();
        let task: Task<()> = rt.spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        task.cancel();
        task.cancel(); // safe to call again
    }

    #[test]
    fn concurrent_tasks_actually_overlap() {
        // The whole point of the async runtime: I/O-bound work that
        // would serialize in the tree-walker runs concurrently.
        let rt = AsyncRuntime::new().unwrap();
        let counter = Arc::new(AtomicU32::new(0));
        let start = std::time::Instant::now();
        let futs: Vec<_> = (0..4)
            .map(|_| {
                let c = counter.clone();
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, AsyncError>(())
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<Output = Result<(), AsyncError>> + Send,
                        >,
                    >
            })
            .collect();
        rt.block_on(join_all(futs));
        let elapsed = start.elapsed();
        assert_eq!(counter.load(Ordering::SeqCst), 4);
        // Serial would be 200ms+; concurrent should be ~50-100ms.
        assert!(
            elapsed < Duration::from_millis(180),
            "expected concurrent execution, took {elapsed:?}"
        );
    }
}
