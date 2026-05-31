//! Process-wide tokio runtime singleton (Stage 36).
//!
//! Stage 36 introduces an *async boundary* at the eval.rs top-level entry:
//! [`crate::Interpreter::run_async`] enters this runtime via `block_on` so
//! subsequent `eval_parallel` (and host bindings like `flow_parallel_asks`)
//! can spawn_blocking without each call starting its own runtime — which
//! would either panic ("Cannot start a runtime from within a runtime") or
//! shred the per-process FD/thread budget.
//!
//! Why a single runtime, not one per consumer:
//!
//! - **Nested-`block_on` safety.** With one runtime, `flow_parallel_asks`
//!   called from inside `run_async` detects the already-current handle and
//!   uses `block_in_place` instead of spawning a second runtime.
//! - **Process-lifetime cost.** Tokio runtime startup is ~5-20ms — paid
//!   once at first touch, amortized across the rest of the process.
//! - **Pool budget.** Tokio's default 512-thread blocking pool is shared
//!   across every `spawn_blocking` site instead of being multiplied by N
//!   independent runtimes.
//!
//! The interior of `eval.rs` stays synchronous. The async substrate is the
//! seam, not a rewrite. Stage 37 lifts `select`/`for await` onto it; Stage
//! 38 lifts channels.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Returns the process-wide tokio runtime, initializing it on first call.
///
/// The runtime is multi-thread with a small worker pool (the IO is blocking
/// and runs on the separate 512-thread blocking pool). Thread name
/// `axon-async` shows up in stack traces and `ps`.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("axon-async")
            .worker_threads(num_workers())
            .build()
            .expect("tokio runtime: failed to build axon-async runtime")
    })
}

fn num_workers() -> usize {
    // The model I/O is blocking — `spawn_blocking` runs on a separate
    // blocking pool whose default is 512 threads. Four worker threads on
    // the core executor are plenty to drive coordination tasks.
    4
}

/// True when called from inside a tokio runtime context (e.g. a future
/// running on the singleton). Used by `flow_parallel_asks` to decide
/// between `block_on` (no current handle) and `block_in_place` (current
/// handle, must not start a nested runtime).
pub fn in_runtime_context() -> bool {
    tokio::runtime::Handle::try_current().is_ok()
}

/// Telemetry counter — incremented every time [`spawn_blocking_counted`] is
/// called. The replay test uses this to assert that a replay run never
/// touches the blocking pool. Public for tests, intentionally unsynchronized
/// load: a single-threaded test reads it after the run finishes.
pub static SPAWN_BLOCKING_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Thin wrapper around `Runtime::spawn_blocking` that increments
/// [`SPAWN_BLOCKING_COUNT`] before dispatching. Call this from any code
/// path that genuinely needs the blocking pool (live model I/O); skip it
/// for paths that should NOT spawn (replay).
pub fn spawn_blocking_counted<F, R>(f: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    SPAWN_BLOCKING_COUNT.fetch_add(1, Ordering::SeqCst);
    runtime().spawn_blocking(f)
}

/// Reset the counter (used by tests that want to measure a single run in
/// isolation).
pub fn reset_spawn_blocking_count() {
    SPAWN_BLOCKING_COUNT.store(0, Ordering::SeqCst);
}

/// Read the counter (used by tests).
pub fn spawn_blocking_count() -> usize {
    SPAWN_BLOCKING_COUNT.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_is_singleton_across_calls() {
        let a = runtime();
        let b = runtime();
        // Pointer-equal: OnceLock returns the same allocation every time.
        assert!(std::ptr::eq(a, b));
    }

    #[test]
    fn runtime_singleton_across_threads() {
        let a = runtime() as *const tokio::runtime::Runtime as usize;
        let h = std::thread::spawn(|| runtime() as *const tokio::runtime::Runtime as usize);
        let b = h.join().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn spawn_blocking_counter_increments() {
        reset_spawn_blocking_count();
        let rt = runtime();
        let n_before = spawn_blocking_count();
        let h = spawn_blocking_counted(|| 1 + 1);
        let v = rt.block_on(async { h.await.unwrap() });
        assert_eq!(v, 2);
        assert_eq!(spawn_blocking_count(), n_before + 1);
    }

    #[test]
    fn in_runtime_context_outside_is_false() {
        assert!(!in_runtime_context());
    }

    #[test]
    fn in_runtime_context_inside_is_true() {
        let rt = runtime();
        let inside = rt.block_on(async { in_runtime_context() });
        assert!(inside);
    }
}
