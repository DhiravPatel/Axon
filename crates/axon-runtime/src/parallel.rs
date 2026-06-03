//! `parallel { ask m1 with { ... }, ask m2 with { ... } }` evaluation (Stage 36).
//!
//! This module owns the actual fan-out/join for the new `parallel { }`
//! surface syntax. `eval.rs::eval_parallel` is the surface-facing arm; it
//! pre-evaluates each arm's operands on the calling thread (so thread-local
//! state like RNG/frozen-clock only reads from the main thread), builds
//! owned `(Arc<dyn ModelProvider>, ChatRequest)` pairs, and hands them to
//! [`run_parallel_asks`].
//!
//! The interpreter itself never crosses a thread boundary — only the
//! per-arm `(Arc, ChatRequest)` does. This means the Stage 32 substrate
//! (Arc<dyn ModelProvider>, Send+Sync on `ModelProvider`) is enough; Stage
//! 37 will lift the "single ask per arm" restriction once `Interpreter` is
//! made Send-safe.

use std::sync::Arc;

use crate::async_rt;
use crate::eval::Interpreter;
use crate::EvalSignal;
use axon_diag::Span;
use axon_models::{ChatRequest, ChatResponse, ModelProvider, ProviderError};

/// One arm of a `parallel { ... }` block, fully materialized on the main
/// thread before any dispatch.
pub(crate) struct ParallelArm {
    pub provider: Arc<dyn ModelProvider>,
    pub request: ChatRequest,
}

/// Run N model calls in parallel, joining in *input order*. Mirrors the
/// Stage 32 `flow_parallel_asks_impl` pattern but takes pre-built arms so
/// it composes cleanly with the new `eval_parallel` surface syntax.
///
/// - Caller-thread responsibilities (must happen BEFORE this call):
///   - `interp.precheck_budget(span)` if not in replay.
///   - Pre-evaluation of each arm's operands.
/// - This function:
///   - Replay path: pops N events from the recording in input order; never
///     touches tokio. Returns the `ChatResponse`s in input order.
///   - Live path: spawn_blocking each `provider.complete(&req)` via
///     [`crate::async_rt::spawn_blocking_counted`]; joins in input order;
///     calls `record_model_call` + `debit_budget_for` per response on the
///     calling thread after the join.
pub(crate) fn run_parallel_asks(
    interp: &mut Interpreter,
    arms: Vec<ParallelArm>,
    span: Span,
) -> Result<Vec<ChatResponse>, EvalSignal> {
    if arms.is_empty() {
        return Ok(Vec::new());
    }

    // Replay short-circuit — must not touch tokio.
    if interp.replay_active() {
        let mut out = Vec::with_capacity(arms.len());
        for _ in 0..arms.len() {
            out.push(interp.pop_replay_model_call(span)?);
        }
        return Ok(out);
    }

    let rt = async_rt::runtime();

    // Spawn every arm. The interpreter is NOT shared with these tasks —
    // only the Arc<dyn ModelProvider> and an owned ChatRequest move
    // across the thread boundary. Sound because `ModelProvider: Send+Sync`.
    let mut handles: Vec<tokio::task::JoinHandle<Result<ChatResponse, ProviderError>>> =
        Vec::with_capacity(arms.len());
    for arm in &arms {
        let p = arm.provider.clone();
        let r = arm.request.clone();
        handles.push(async_rt::spawn_blocking_counted(move || p.complete(&r)));
    }

    // Join in input order. If we're already in a tokio context (the new
    // Stage 36 `Interpreter::run_async` wraps cmd_run in a `block_on`), we
    // can't nest `block_on` — use `block_in_place` to release the worker
    // thread to siblings while we wait synchronously here.
    let join_async = async move {
        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            let r = match h.await {
                Ok(Ok(resp)) => Ok(resp),
                Ok(Err(e)) => Err(e.to_string()),
                Err(join_err) => Err(format!("task panicked: {join_err}")),
            };
            out.push(r);
        }
        out
    };
    let results: Vec<Result<ChatResponse, String>> = if async_rt::in_runtime_context() {
        tokio::task::block_in_place(|| rt.block_on(join_async))
    } else {
        rt.block_on(join_async)
    };

    // §36.6 verification fix C1 — atomic record-or-don't.
    //
    // The naive "record successes as we walk, return on first error"
    // shape leaves the recording desynchronized: if arm 2 of 3 fails
    // but arm 3 also succeeded, only arm 1 is recorded. On replay, a
    // pop attributes arm 3's response to arm 2 (because the recording
    // is event-indexed, not arm-indexed), then the third pop runs out
    // — silently corrupting any program that examines parallel arms.
    //
    // Fix: first pass — collect responses, capture any error WITHOUT
    // touching the recording or budget. Second pass — only when every
    // arm succeeded, record + debit in input order. A failed batch
    // adds zero events to the recording, so replay's pop count never
    // outruns the saved events for that batch.
    let mut responses: Vec<ChatResponse> = Vec::with_capacity(results.len());
    for (idx, (arm, res)) in arms.iter().zip(results.into_iter()).enumerate() {
        match res {
            Ok(resp) => responses.push(resp),
            Err(msg) => {
                return Err(EvalSignal::error(
                    format!(
                        "parallel: arm {idx} (model `{}`) failed: {msg}",
                        arm.provider.name()
                    ),
                    span,
                ));
            }
        }
    }
    debug_assert_eq!(responses.len(), arms.len());
    for (arm, resp) in arms.iter().zip(responses.iter()) {
        interp.record_model_call(arm.provider.name(), resp.clone());
        interp.debit_budget_for(resp);
    }
    Ok(responses)
}
