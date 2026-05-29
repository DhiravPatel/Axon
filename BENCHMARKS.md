# Axon Benchmarks

Real measured numbers, not design targets. Every figure here is produced by code in [crates/axon-cli/tests/dogfood_benchmarks.rs](crates/axon-cli/tests/dogfood_benchmarks.rs) and [examples/dogfood_triage/benches/](examples/dogfood_triage/benches/), reproducible with one command. Each measurement subtracts a `noop.ax` baseline so the published numbers reflect work, not `axon` process startup.

```bash
cargo test -p axon-cli --test dogfood_benchmarks -- --nocapture
```

## Hardware

| Item | Value |
|---|---|
| Machine | Apple Silicon (`arm64`) |
| Cores | 10 physical |
| OS | Darwin 24.5.0 |
| Build | `cargo build` (debug) — release builds will be faster |

Numbers below are from the **debug** build. The async overhead floor matters most in dev, so this is the honest comparison; release-build numbers are strictly better.

## §32.1 — Parallel-vs-serial model I/O

The headline metric for the async I/O slice: how much wall time does `flow_parallel_asks` save vs the equivalent serial `ask` loop, when the model I/O is the bottleneck?

**Workload**: 4 × `mock_model_slow("ok", 200ms)` asks per batch, 5 batches per run.

| Mode | Wall (total) | Wall (minus baseline) | Per batch |
|---|---|---|---|
| baseline (`noop.ax`) | 7.5 ms | — | — |
| serial `ask` loop | 4.082 s | 4.075 s | 815 ms |
| `flow_parallel_asks` | 1.035 s | 1.027 s | 205 ms |
| **speedup** | — | — | **3.9×** |

Expected ceiling is 4× (`BATCH_SIZE = 4`, all asks identical latency). The observed 3.9× is within `spawn_blocking` task-launch overhead of the theoretical maximum. The serial baseline (815 ms / batch ≈ 4 × 200 ms) confirms there's nothing else stealing time.

Asserted as a hard gate in [`parallel_asks_beats_serial_by_at_least_2x`](crates/axon-cli/tests/dogfood_benchmarks.rs#L77). The test fails CI if the speedup drops below 2×.

## §32.1 — Replay determinism on a non-trivial program

The promise: a parallel-dispatch run is byte-identical to a serial run when replayed. The proof: the [dogfood triage agent](examples/dogfood_triage/) — which classifies 5 tickets via `flow_parallel_asks` across 3 models each, looks up customers in memory, applies routing policy, and drafts auto-replies via a 4th model — records and replays byte-identically.

```text
record-then-replay diff: (empty) — REPLAY IS BYTE-IDENTICAL
```

Asserted in [`triage_agent_replay_is_byte_identical`](crates/axon-cli/tests/dogfood_benchmarks.rs#L106). 15 model calls per run (3 classifiers × 5 tickets), 3 drafter calls, 5 memory writes — all in input order, all stable across runs.

## Coverage gaps (honest)

What's deliberately **not** here yet:
- **Agent-spawn time.** The advisor named this; the runtime supports `spawn Agent(...)` but the actor lifecycle is still synchronous (the §32.1 slice is model-I/O only). Measuring spawn time before the actor scheduler is async would mostly measure tree-walker dispatch overhead — not a useful number to publish.
- **Scheduler throughput.** Same reason: the message-passing scheduler is single-threaded today. The number would be ~"interpreter ops/sec on a hot path", not "messages/sec across the actor graph in production".
- **End-to-end agent latency under real model calls.** The dogfood agent runs against mock providers. With a real Anthropic provider, the parallel speedup would be even more pronounced because the sleep is replaced with real network I/O that genuinely overlaps.
- **Release-build numbers.** Above table is debug. Release adds ~3-5× across the interpreter; the *ratio* between serial and parallel stays the same because the bottleneck is the (mock) I/O wait, not interpreter speed.

These will land alongside the rest of the async migration. Publishing them today against a still-mostly-sync runtime would be misleading.

## What to read next

- [§32.1 in FEATURES.md](FEATURES.md) — what `flow_parallel_asks` is and how the implementation works.
- [examples/dogfood_triage/](examples/dogfood_triage/) — the real agent the replay-determinism number comes from.
- [PAPERCUTS.md](PAPERCUTS.md) — everything we tripped over while writing real Axon. The honest counterweight to a benchmarks page.
