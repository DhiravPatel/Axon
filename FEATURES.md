# Axon — Implemented Features

A snapshot of everything Axon ships today, grouped by the stages that introduced each capability. All features below are covered by the workspace test suite (**579 tests passing** across 30+ crates).

---

## Stage 23 — Dynamic-library FFI (§35) + Delegated Identity (§54.2) + `axon pkg` (§36)

Three production-quality additions land together in Stage 23: in-process FFI for pre-built native libraries, the `on_behalf_of` delegation primitive that lets one principal authorize another, and a CLI subcommand for managing project dependencies.

### §35 Dynamic-library FFI via `libloading`
- New `axon-ffi::dlib` module: `DynamicLibrary` (RAII wrapper around `libloading::Library`), `DlibValue { I64 | F64 | Str }`, typed `DlibError`.
- `DynamicLibrary::open(path)` — load any `.so` / `.dylib` / `.dll` from disk; symbols looked up lazily via `Library::get`.
- `DynamicLibrary::call(symbol, args, ret_is_str)` — dispatches based on the arg shape against a small, deliberately narrow set of supported C signatures: i64 arity 0..=4 → i64, f64 arity 0..=2 → f64, single `*const c_char` → `*const c_char`, void → `*const c_char`. Anything outside that closed set is rejected statically with `DlibError::UnsupportedSignature` rather than risking undefined behavior.
- Host binding `ffi_dlib_call(lib_path, symbol, args_list, ret_is_str)` returns `{ ok, value, error }`. Args are tagged records `{ ty: "i64"|"f64"|"str", v: <val> }` so the host can pick the right C signature without relying on Axon's dynamic-typing inference.
- Real test: opens `libSystem.dylib` (macOS) / `libm.so.6` (Linux), calls `cos(0.0)`, asserts `1.0` round-trips through the boundary.

### §54.2 Delegated identity (`on_behalf_of`)
- New `axon-a2a::identity::Delegation` — `{ principal, audience, scopes, expires_at_secs, nonce }`. Serializes to canonical JSON; the JSON is what's actually signed.
- `SignedDelegation { delegation_json, signature_hex, signer_pubkey_hex }` — same shape as `SignedAgentCard` from Stage 22, reusing the Ed25519 primitives.
- `KeyPair::sign_delegation(&Delegation)` produces a `SignedDelegation`; `SignedDelegation::verify(&TrustStore, expected_audience, now_secs)` is **fail-closed** in this order:
  1. hex parse (signature & pubkey both syntactically valid);
  2. trust-store membership check (untrusted-but-mathematically-valid signatures rejected);
  3. signature verification (ed25519-dalek);
  4. audience match (`expected_audience` must equal `delegation.audience`);
  5. expiry check (`now_secs <= delegation.expires_at_secs`).
- The trust check runs *before* the signature math so an attacker probing with a known-untrusted key learns nothing about signature internals — defense-in-depth against timing oracles.
- Host bindings: `a2a_sign_delegation(seed_hex, principal, audience, scopes_list, expires_at_secs, nonce, dest_json_path)` returns the signer's pubkey hex; `a2a_verify_delegation(signed_path, trust_store_name, expected_audience, now_secs)` returns the parsed delegation record on success and errors on any failed gate.
- Reuses the Stage 22 `TRUST_STORES` thread-local registry — store-name semantics are identical between signed cards and signed delegations.

### §36 `axon pkg` subcommand
- New `axon pkg <list|add|remove|audit>` for read/edit of the `[deps.<name>]` tables in `axon.toml`.
- `pkg list` prints each dep with its path; `pkg add NAME --path P` writes (or overwrites) the entry, validating the dep name is alphanumeric/underscore/dash only; `pkg remove NAME` deletes the entry (errors if missing); `pkg audit` walks every declared dep and reports `ok`, `WARN` (dir present but no `axon.toml` or `src/`), or `FAIL` (path doesn't exist).
- Round-trips through `toml::Value` so unknown manifest sections survive edits unchanged. `--manifest PATH` overrides the default of `./axon.toml` so the command works from any directory.
- Network/git deps land in a later stage; today the surface is local-path only — same constraint as `axon-project` itself.

### Test coverage
- `crates/axon-ffi/src/dlib.rs` — 5 unit tests including a real `cos(0.0)` invocation through `libloading`.
- `crates/axon-a2a/src/identity.rs` — 5 new delegation tests on top of Stage 22's signed-card tests (round trip, audience mismatch, expiry, untrusted signer, tampered JSON).
- `crates/axon-cli/tests/host_dlib_and_delegation.rs` — 6 end-to-end tests through the `axon` binary.
- `crates/axon-cli/tests/axon_pkg.rs` — 8 tests covering list / add / remove / audit on real on-disk manifests.

### CLI demo (real run)
```
$ axon run examples/stage23_dlib_and_delegation.ax
ffi_dlib_call(cos, 0.0) ok =
true
cos(0.0) =
1
signed delegation written to
/tmp/axon-stage23-deleg.json
verified delegation:
  principal =
user:alice
  audience  =
research-agent-1
  nonce     =
demo-nonce-001
```

---

## Stage 1 — Lexer, Parser, AST

**Crates:** [axon-lexer](crates/axon-lexer/), [axon-parser](crates/axon-parser/), [axon-ast](crates/axon-ast/), [axon-diag](crates/axon-diag/)

- Hand-written lexer with Unicode NFC normalization on identifiers.
- Nested block comments (`/* ... /* ... */ ... */`).
- Doc comments (`///` item-attached, `//!` module-level) preserved as distinct token kinds.
- String literals: plain, raw (`r"..."`), multi-line (`"""..."""`), and prompt strings (`p"..."`).
- Domain literals: money (`$1.99`, `€10`), duration (`30s`, `5m`, `2h`), dates (`2026-05-19`), times (`14:30`).
- Numeric literals: integer, float (with exponents), hex/binary/octal, underscores for grouping.
- Recursive-descent parser with Pratt expression parsing for full operator precedence.
- Items: `fn`, `type`, `schema`, `agent`, `actor`, `tool`, `model`, `memory`, `prompt`, `trait`, `const`, plus imports and visibility (`pub`).
- Every AST node carries a `Span { file: u16, start: u32, end: u32 }` — file-stamped for cross-file diagnostics.
- Rich diagnostics with primary + secondary labels, source-file registry, ANSI-colored terminal rendering.

## Stage 2 — Type Checker

**Crate:** [axon-tyck](crates/axon-tyck/), [axon-types](crates/axon-types/)

- Bidirectional type checking: synthesis + checking modes.
- Full effect rows on every function arrow (`uses { Network, FileSystem }`).
- `Tainted<T>` as a distinct type (not a subtype of `T`) — propagation is explicit.
- Schema types with structural records, optional fields, default values.
- Generics on functions, types, traits.
- Trait resolution with coherence and overlap checking.
- Union types, refinement predicates, narrowing.
- Type-error suggestions ("did you mean...") for unknown identifiers and fields.

## Stage 3 — Tree-Walking Interpreter

**Crate:** [axon-runtime](crates/axon-runtime/)

- Direct AST evaluation for the pure-Rust subset.
- Closures with proper lexical scoping.
- Pattern matching (literals, tuples, records, variants, wildcards, guards).
- Mutable local bindings (`var`) and `while`/`for` loops.
- User-defined functions, recursion, higher-order functions.
- Built-in operations on `Int`, `Float`, `String`, `Bool`, lists, records, options.

## Stage 4 — Capability System

**Crate:** [axon-runtime](crates/axon-runtime/) (caps module)

- Static effect rows checked at compile time.
- Per-frame capability attenuation: a function can only pass *strictly fewer* capabilities to its callees.
- No ambient authority — `with caps { ... }` is the only way to grant a capability.
- Built-in capabilities: `Console`, `Network`, `FileSystem`, `Random`, `Clock`, `Env`.
- Capability-aware standard library (e.g., `print` requires `Console`, `http.get` requires `Network`).

## Stage 5 — AxVM Bytecode VM

**Crate:** [axon-vm](crates/axon-vm/)

- Stack-based bytecode with ~40 opcodes.
- Lua-style upvalues for closure capture (open/close on stack frame exit).
- Compile target shared with the interpreter — programs run identically on both.
- Inline caches for record-field access.
- Tail-call optimization for `return f(...)` in tail position.

## Stage 5.5 — Actors

**Crate:** [axon-runtime](crates/axon-runtime/) (actors module)

- `actor` declarations with typed state, message handlers, and lifecycle hooks (`on_start`, `on_stop`).
- Synchronous-dispatch mailbox model (deterministic, replayable).
- Typed messages with pattern-matched handlers.
- Cross-actor send returns a future / awaited reply.
- Supervision with restart strategies.

## Stage 6 — LLM Integration

**Crate:** [axon-models](crates/axon-models/)

- Anthropic Messages API client via `ureq`.
- `model "claude-..." { ... }` declarations with system prompt, temperature, top_p, max_tokens.
- `prompt "..."` blocks with template slots and structured output (`-> Schema`).
- Multi-turn tool-use loop: the model can call back into Axon tools and the runtime feeds results back in.
- Streaming responses (server-sent events) for token-by-token output.

## Stage 6.5 — Tools

**Crates:** [axon-runtime](crates/axon-runtime/), [axon-models](crates/axon-models/)

- First-class `tool` declarations with typed parameters, typed result, capability requirements.
- Tools are exposed to LLM models as JSON-schema tool definitions automatically.
- Tool-call results round-trip through the model's tool-use protocol.
- Capability checking on tool calls — a model can't invoke a tool requiring caps the caller doesn't hold.

## Stage 7 — Tracing, Budgets, Replay

**Crate:** [axon-runtime](crates/axon-runtime/) (tracing/budget/replay modules)

- Structured JSON event log: every model call, tool call, capability grant, actor message.
- `with budget(tokens=1000, cost=$0.50, time=30s) on_exceeded handler` — composable budget stacks.
- Record mode: capture every non-deterministic outcome (LLM responses, randomness, clock reads, network).
- Replay mode: re-execute against the captured tape — byte-identical results.
- Trace viewer JSONL format compatible with downstream tooling.

## Stage 8 — Modules, Tests, Project Manifest

**Crate:** [axon-project](crates/axon-project/)

- File-as-module project layout (`src/foo.ax` → module `foo`).
- `axon.toml` manifest: `[package]`, `[run]`, `[caps]`, `[dependencies]`.
- Public/private item visibility with cross-module checking.
- `#[test]` attribute + `axon test` runner with per-test isolation.
- Module collision diagnostic (P0001), unknown-module diagnostic (P0011).
- Re-export support and import paths (`use foo.bar.baz`).

## Stage 8.5 — Privacy, Per-File Spans, Dependencies

**Crates:** [axon-diag](crates/axon-diag/), [axon-project](crates/axon-project/)

- Source registry with `file_id` stamped onto every span — diagnostics point to the right file across the whole project.
- Privacy diagnostic (P0010): accessing a private item from another module is a hard error with a fix-it hint.
- Dependency resolution from `axon.toml` — local path deps work today; registry deps stubbed for v1.

## Stage 9 — WebAssembly Backend

**Crate:** [axon-wasm](crates/axon-wasm/)

- WASM codegen via `wasm-encoder` for the pure-Int subset (`fn`, arithmetic, `if`, `while`, `let`/`var`, recursion).
- Two-pass body compiler to handle locked locals.
- `wasmparser` validation of every emitted module.
- `wasmi` execution path for running compiled WASM in-process.
- `axon build <file> -o out.wasm` CLI command.
- Host-import surface for `print_int` and similar.

## Stage 9.5 — Language Server (LSP)

**Crate:** [axon-lsp](crates/axon-lsp/)

- LSP server via `lsp-server` + `lsp-types`.
- Push diagnostics on file change (parse + type errors).
- Hover: type signature + doc comment for the item under the cursor.
- Go-to-definition for local and cross-module symbols.
- Completion: keywords, in-scope identifiers, member access.
- Editor integration ready (VS Code, Neovim, any LSP-aware client).

## Stage 22 — Platform Sandboxes (Linux seccomp + macOS sandbox-exec) + Ed25519 A2A Identity

Real OS-level isolation for tool subprocesses, plus verifiable cross-org agent identity via Ed25519.

### §42 Platform sandboxes
- New `axon-sandbox::platform` module with `PlatformProfile` (declarative intent: `read_only_fs`, `allow_network`, `allow_subprocess`, `extra_syscalls`) and `PlatformSandbox` (mutates a `std::process::Command` before spawn).
- Three presets: `strict()` (default — read-only FS, no net, no fork), `networked()`, `build_tool()`.
- **Linux**: seccomp-bpf filter via `seccompiler` (pure Rust, same library Firecracker uses). `PR_SET_NO_NEW_PRIVS + seccomp(2)` installed inside `pre_exec`, so the filter is in force from instruction 1 of the user's process. Whitelist covers POSIX core syscalls; `KillProcess` (SIGKILL) on anything outside the allowlist.
- **macOS**: `sandbox-exec(1)` wrapping. The command is rewritten as `sandbox-exec -p <inline-sbpl> <original-program> <args...>`. sbpl profile is `(deny default)` + opt-ins for the operations the profile enables.
- **Windows**: documented v0 limit — `Limits` + wall-timeout still apply, Job Object integration deferred.
- Host binding `sandbox_run_with_profile(program, args, cpu, mem_mb, wall_s, profile_name)` returns the same result record as `sandbox_run` from Stage 15; the kernel sandbox layer is additive.

### §54 Ed25519 signed agent identity
- New `axon-a2a::identity` module: `KeyPair` (ed25519-dalek), `SignedAgentCard { card_json, signature_hex, signer_pubkey_hex }`, `TrustStore`, `IdentityError`.
- `KeyPair::generate()` uses OS RNG; `KeyPair::from_seed_bytes` is deterministic for reproducible tests and seed-vault recovery.
- `KeyPair::sign_card(&AgentCard)` produces a `SignedAgentCard` with the canonical card JSON, 64-byte hex signature, and 32-byte hex verifying key.
- `SignedAgentCard::verify(&TrustStore)` is **fail-closed**: signature math must verify *and* the signer's pubkey must be in the trust store. Untrusted-but-mathematically-valid signatures are rejected with `IdentityError::Untrusted(hex)`.
- `KeyPair::Debug` redacts the private seed; the only way to extract it is through `seed_hex()` / `seed_bytes()` (named so audit trails flag the call).
- Host bindings: `a2a_keypair_generate`, `a2a_keypair_from_seed`, `a2a_sign_card(card_json_path, seed_hex, signed_dest_path)`, `a2a_verify_signed_card(signed_path, trust_store_name)`, `a2a_trust_store_new(name, list_of_pubkey_hex)`.

### CLI demo (real run)
```
$ axon run demo.ax
--- platform sandbox: strict profile ---
hello-from-sandbox                          # echo ran under macOS sandbox-exec
0
--- Ed25519 keypair ---
a79b45a4162df95e                            # first 16 hex chars of pubkey
64                                          # 64-char seed
--- sign + verify ---
research-acme                               # verified card.agent_id
Acme Research Agent
--- signed card on disk ---
{
  "card_json": "{...canonical AgentCard JSON...}",
  "signature_hex": "68edfe57...a04",        # 64-byte Ed25519 sig
  "signer_pubkey_hex": "a79b45a4...500"     # 32-byte verifying key
}
```

---

## Stage 21 — OAuth Vault, TLS Serve, Graceful Shutdown, `axon login`

Production security layer: refreshable OAuth tokens, rustls-terminated HTTPS, SIGINT-driven drain shutdown, and a CLI credential capture flow.

### §40.2 OAuth-aware vault
- New `axon-secret::oauth::OauthToken` — `access_token` + optional `refresh_token` + `expires_at_secs` + `token_url` + `client_id`. All fields serialize for vault storage; the value layer keeps `Secret<T>` redaction.
- `is_expired_at(now)`, `needs_refresh(slack_secs)` — pure predicates for tests.
- `OauthToken::refresh()` — POSTs `grant_type=refresh_token` to the stored `token_url` via `ureq`, parses the JSON response, rotates `access_token` (and `refresh_token` if the server rotated it), and recomputes `expires_at_secs` from `expires_in` (defaults to 1 hour per OAuth2 spec).
- `Vault::set_oauth(name, &token)` / `Vault::get_oauth(name)` — OAuth tokens live under `oauth:{name}` in the same vault JSON, so plain API keys and OAuth tokens coexist without a schema change.
- `Vault::load_oauth_with_refresh(name, slack_secs, path)` — loads, refreshes if needed, persists rotated token back to disk before returning. The standard "always-fresh, no re-login" pattern.
- Typed `TokenRefreshError`: `NoRefreshToken` / `NoTokenUrl` / `Http(...)` / `HttpStatus { status, body }` / `Io` / `Parse`.

### §41 TLS via rustls
- `Server::with_tls_pem(cert_pem_path, key_pem_path)` — loads a PEM-encoded cert chain + private key (rustls-pemfile parses RSA, ECDSA, EdDSA, and PKCS8 keys). Pure Rust, no OpenSSL.
- Single crypto provider registered via `rustls::crypto::ring::default_provider().install_default()` (called lazily on first use).
- `handle_connection_plain` / `handle_connection_tls` share the same `read_request_from<S: Read>` / `write_response_to<S: Write>` helpers, so adding TLS didn't fork the protocol logic.
- v0 limit: TLS reads PEM from disk at bind time; cert rotation requires a redeploy. ACME / file-watching rotation is a Stage 22+ enhancement.

### §41 Graceful shutdown
- `Server.in_flight: AtomicUsize` — incremented when a connection thread starts, decremented when it returns. After `stop` flips, `run()` waits up to `shutdown_grace` (default 10s) for `in_flight == 0` before returning.
- `Server::install_signal_handler()` — Unix-only: installs a `SIGINT`/`SIGTERM` handler that flips `stop`. Process-wide; latest install wins (last `axon serve` invocation in the same process). Non-Unix is a no-op — callers can still flip `server.stop` programmatically.
- `serve_run` / `serve_run_tls` both call `install_signal_handler()` before entering the run loop, so `Ctrl-C` and `kill -TERM` drain in-flight requests automatically.

### §36 `axon login`
- `axon login <provider> [--vault PATH] [--key VALUE]` — stores an API key in the vault under `<PROVIDER>_API_KEY`.
- Key source precedence: `--key` arg → `<PROVIDER>_API_KEY` env var → interactive stdin prompt.
- Vault path resolution: `--vault` flag → `AXON_VAULT` env → `~/.axon/vault.json`.
- Vault file is mode `0600` on Unix (verified by `axon-secret::Vault::save`).
- Multiple `axon login` calls on the same vault append cleanly — keys for different providers coexist.

### `axon serve` extended
- New flags: `--tls-cert PATH` / `--tls-key PATH` (must be paired). When present, routes through `serve_run_tls` instead of `serve_run`.
- Banner now mentions Ctrl-C: `axon serve [tls]: listening on https://… (Ctrl-C to shutdown)`.

### CLI demo (real run)
```
$ axon login anthropic --vault /tmp/v.json --key sk-ant-demo
saved `ANTHROPIC_API_KEY` to /tmp/v.json (mode 0600 on Unix)
$ ls -l /tmp/v.json
-rw-------  1 user user  72 May 19 21:35 /tmp/v.json

$ axon serve svc.ax --listen 127.0.0.1:18432 \
                    --tls-cert /tmp/cert.pem --tls-key /tmp/key.pem &
axon serve [tls]: listening on https://127.0.0.1:18432 (Ctrl-C to shutdown)

$ curl -sk -X POST https://127.0.0.1:18432/invoke -d "hello-axon"
got: hello-axon

$ kill -TERM %1
(server drains in-flight requests, exits cleanly)
```

---

## Stage 20 — OTLP Exporter + `axon replay/--patch` + `axon trace` + `axon repl`

Closes three observability/tooling gaps: OpenTelemetry-compatible trace export, deterministic replay-with-edits, and an interactive REPL.

### §31 OTLP/HTTP-JSON exporter
- New `axon-runtime::otlp` module: converts internal `TraceSpan` records into OpenTelemetry Protocol JSON (`ExportTraceServiceRequest` shape) — byte-compatible with what real OTLP exporters POST to `/v1/traces`.
- Span-kind mapped to `SPAN_KIND_INTERNAL` (1); error spans get `STATUS_CODE_ERROR` (2) with the message; OK spans get `STATUS_CODE_OK` (1).
- Resource bag includes `service.name`, `telemetry.sdk.name`, `telemetry.sdk.language`, and `telemetry.sdk.version`.
- Stable 32-hex-char `traceId` keyed off the recording's smallest span id so record/replay pairs produce identical IDs.
- Nanosecond timestamps (OTLP requires nanos; we multiply our `start_ms`/`end_ms` by 1_000_000).
- Host binding `trace_export_otlp(path, service_name)` — NativeExt that pulls live spans via `Interpreter::with_trace_spans`.
- Refuses cleanly when tracing wasn't enabled: `trace_export_otlp: tracing is not enabled — re-run with axon run --trace …`.

### §32 `axon replay <rec> <src>` with `--patch`
- New CLI subcommand. Strict mode (default) replays an Axon program against a recording byte-identically and reports `consumed N of M recorded event(s)` on stderr.
- `--patch` mode flips `Replay::lenient`: a program edited *after* the recording was made (extra model calls, etc.) gets a cleaner `replay exhausted (patch mode)` error message and the cursor report shows `[patch]` vs `[strict]`.
- New `Interpreter::enable_replay_lenient` + `replay_progress() -> Option<(cursor, total, lenient)>` for end-of-run summaries.

### §36 `axon trace <file.jsonl>`
- Reads a JSONL trace file (the format `--trace PATH` already writes) and pretty-prints a colored span tree with durations, kinds, and any attached error.
- Empty file → `(no spans)`; malformed JSON → typed error with the offending line number.
- No external deps — uses `serde_json::Value`.

### §36 `axon repl`
- Interactive read-eval-print loop with banner, prompt-numbered input, and three dot-commands: `.help`, `.quit`/`.exit`, `.effects`.
- Each line is wrapped in a synthesized `fn __repl_N() uses { ...standard row... } { ... }` so built-ins like `print_int`, `time_now`, `http_fetch` work without `uses` clauses in REPL input.
- Persistent interpreter — bindings from `let x = ...` survive across prompts (within the synthesized fn's scope).
- Tracing is auto-enabled so the REPL can show effect summaries on demand.

### CLI demo (real run)
```
$ axon run --trace internal.jsonl --record rec.json svc.ax
the answer is 42

$ axon trace internal.jsonl
trace: 1 span(s), max span duration 0ms
ask (ask)  0ms

$ head -5 traces.otlp.json
{
  "resourceSpans": [
    {
      "resource": {
        "attributes": [ ... "service.name": "demo-svc" ... ]

$ axon replay rec.json svc.ax
axon replay [strict]: consumed 1 of 1 recorded event(s)

$ axon replay rec.json svc-edited.ax --patch
... replay exhausted (patch mode): no recorded event remaining for this call
axon replay [patch]: consumed 1 of 1 recorded event(s)

$ printf 'print_int(40 + 2)\n.effects\n.quit\n' | axon repl
Axon 0.1.0 REPL — type `.help` for commands, `.quit` to exit.
axon[1]> 42
axon[2]> active capabilities: {Audit, Channel, Console, ...}
```

---

## Stage 19 — `for await`, `select`, `plan with` Enhancements

Three core language constructs from §14 and §26 wired through parser, type checker, and runtime.

### §14 `for await` — async-stream iteration
- New `is_await: bool` field on `ExprKind::For`; parser recognizes `for await pat in expr { body }`.
- Eval routes async-flagged iteration to a Chan-draining loop when the iterator is a `Chan` value; lists, sets, maps, tuples, and strings still iterate as usual.
- `break` and `continue` work correctly mid-stream.
- The `await` keyword is semantic markup today (the synchronous interpreter drains eagerly) — the surface is identical to what the async scheduler will use when it lands.

### §14 `select` — multi-channel arms
- Replaced the raw-text `SelectArm` placeholder with a typed AST: `SelectArmKind::{Recv, Timeout, Else}`.
- New parser for `select { name = recv(chan) => body, _ = timeout(dur) => body, else => body }`. No new tokens — uses existing call syntax instead of `<-` so it composes with the rest of the grammar.
- Runtime semantics: walk recv arms in declaration order, pick the first whose channel is non-empty. If none is ready, take the first `timeout` (fires immediately in the sync runtime), then the first `else`. With no fallback arm, a runtime error surfaces.
- Declaration order is the tiebreak when multiple channels are ready — deterministic and easy to reason about.

### §26 `plan with` — `max_steps` + `output: Schema`
- `max_steps:` slot now actually caps the tool-use loop (overrides the default `MAX_TOOL_USE_ITERATIONS = 8`). Validates that the value is `> 0` *before* the first model call.
- `output:` slot steers the model toward emitting JSON (appended as a system-prompt nudge) and parses the final response as a `Record` so call sites can pattern-match `r.field`.
- Type checker now returns `Dyn` for `plan ... { output: ... }` (Stage 12 gradual-escape-hatch propagation) so field access on the returned record type-checks without manual ascription.
- Bad JSON in the model's final response surfaces a clean `plan` with `output:` expects valid JSON error.

### Type-checker generalizations driven by these features
- `for` over `Dyn` no longer errors (E0230) — propagates as `Dyn` element type, same as field access on `Dyn`.
- Method calls on `Dyn` no longer error — return `Dyn` with `Dyn` argument types.
- These two relaxations are what let stdlib calls (`list_new(...)`, `chan()`, `mem_*`, `rag_*`) feed into the new constructs without per-program annotations.

### CLI demo (real run)
```
--- for await over a list (sync stream) ---
1
2
3
--- for await draining a chan ---
alpha
beta
gamma
--- select picks the ready channel ---
hello-from-b
--- select falls through to else ---
(no message)
--- plan with output: schema returns a Record ---
42                                  # parsed from {"answer":"42","confidence":0.95}
```

---

## Stage 17 — Deploy (HTTP server, health checks, env binding, manifest)

**Crate:** [axon-deploy](crates/axon-deploy/). Adds `axon serve` and `axon deploy` CLI subcommands.

### `axon-deploy` (§41) — production deploy primitives
- **Minimal HTTP/1.1 server** in pure Rust (`std::net::TcpListener`, thread-per-connection, no `tokio`/`hyper`).
  - Routes `POST /invoke` to a user handler.
  - Routes `GET /healthz` (liveness: 200 if anything is up) and `GET /readyz` (readiness: 503 if any check fails).
  - 15s read/write timeouts, 4 MiB body cap, 32 KiB header cap — malformed/oversize requests are rejected before they reach a handler.
  - `Connection: close` per response; one request per socket. Keeps the server loop ~200 LoC.
- `HealthCheck` trait + built-in `Liveness` + `AlwaysHealthy(name)`. Custom checks plug in via `Server::with_check(Box::new(...))`.
- **Dotenv loader** that preserves existing process env by default (deployment-baked secrets win over repo defaults). `overwrite: true` for the rare cases that need it.
- `DeployManifest` (`deploy.json`): `name`, `entrypoint_handler`, `port`, `env: BTreeMap<String, String>`, `health_checks: Vec<String>`, optional `dotenv` + `vault` refs, version-checked on load.

### CLI bindings
- `env_get(name)`, `env_get_or(name, default)`, `env_load_dotenv(path, overwrite)`.
- `serve_run(listen_addr, handler)` — **NativeExt**; binds the server, then routes each request through the Axon handler in the interpreter thread. Cross-thread handoff via `mpsc` so the single-threaded interpreter stays sound even though the HTTP loop is multi-threaded.
- `deploy_write_manifest(dir, name, entrypoint, port)`.

### New `axon` subcommands
- **`axon serve <file> [--listen ADDR] [--handler NAME]`** — start the HTTP server.
- **`axon deploy <project_dir> -o <out_dir> [--name N] [--port P] [--handler H]`** — package: writes `<name>.axskill` (Stage 14 format) + `deploy.json`.

### CLI demo (real run, with `nc` hitting the live server)
```
$ axon deploy src_project -o dist --port 9191 --handler greet
wrote dist/greet-svc.axskill
wrote dist/deploy.json

$ axon serve server.ax --listen 127.0.0.1:9192 --handler greet
axon serve: listening on 127.0.0.1:9192

$ printf 'POST /invoke HTTP/1.1\r\n...\r\n\r\naxon!' | nc 127.0.0.1 9192
HTTP/1.1 200 OK
Content-Length: 13
Hello, axon!!

$ printf 'GET /healthz HTTP/1.1\r\n...\r\n\r\n' | nc 127.0.0.1 9192
HTTP/1.1 200 OK
{"checks":[{"detail":"","name":"liveness","ok":true}],"ok":true}
```

---

## Stage 16 — Trajectory Eval, Cost Optimization, FFI

**Crates:** [axon-eval](crates/axon-eval/), [axon-cost](crates/axon-cost/), [axon-ffi](crates/axon-ffi/)

### `axon-eval` (§55) — scenarios, metrics, suite runner
- `Scenario { name, input, expected, tags }`; `RunResult { output, latency_ms, data, error }`.
- Five built-in metrics: `ExactMatch`, `Contains`, `RegexLike` (anchored wildcard), `JsonPath` (`/foo/bar=value`), `LatencyP95` (per-suite budget over actual latencies).
- `Metric` trait is object-safe so suites hold heterogeneous `Box<dyn Metric>`.
- `Suite::run` takes `FnMut` so the host can capture `&mut Interpreter` and dispatch through user-supplied handlers.
- `SuiteReport::to_junit_xml()` emits CI-friendly XML with per-testcase failure messages.

### `axon-cost` (§56) — ledger, profiles, reports
- `CostEntry`: per-call `(provider, model, input_tokens, output_tokens, cached_input_tokens, latency_ms, timestamp_ns, tag)`.
- `ProviderProfile`: prices in cents-per-million-tokens (so integer math doesn't round cheap models to zero), plus optional per-call fixed cost and cached-input discount.
- `Ledger::save`/`load` for cross-process persistence with versioned JSON.
- `Report::build` aggregates: total calls / cents, per-provider summary, **p50 + p95 latency**, top-N most-expensive calls.

### `axon-ffi` (§35) — subprocess FFI with JSON line protocol
- `call_once(spec, request)` — spawn, write one JSON line on stdin, read one JSON line from stdout, return response. Bounded by `timeout_ms` with a sentinel thread that `SIGKILL`s overstays via libc.
- `Connection` for persistent line-protocol children (amortizes spawn cost).
- All FFI is subprocess-based — no `libloading`, no `unsafe`, no native deps beyond `libc` on Unix.

### CLI bindings
- `eval_suite_new`, `eval_add_scenario`, `eval_add_metric` (`exact_match`/`contains`/`regex_like`/`json_path`), `eval_set_latency_budget`, `eval_run` (NativeExt — invokes the user handler), `eval_report_junit`.
- `cost_record`, `cost_profile_add`, `cost_report`, `cost_save`, `cost_load`, `cost_reset`.
- `ffi_call(program, args_list, request_json, timeout_ms)` → `{ ok, response_json, error }`.

### CLI demo (real run)
```
--- eval ---
3                                  # total scenarios
2                                  # passed (1 expected-mismatch failed)
--- cost ---
3                                  # total calls
2700                               # total cents ($27.00 across 3 calls)
600                                # p50 latency_ms
900                                # p95 latency_ms
anthropic                          # top-spend provider
anthropic                          # most expensive single call's provider
--- ffi ---
true
{"hello":"axon"}                   # JSON round-tripped through /bin/cat
```
The JUnit XML report from `eval_report_junit` includes:
```xml
<testsuite name="polite-suite" tests="3" failures="1">
  <testcase name="wrong" time="0">
    <failure message="exact_match: expected `Goodbye, axon!`, got `Hello, axon!`"/>
  </testcase>
</testsuite>
```

---

## Stage 15 — Guardrails, Secrets, Sandbox

**Crates:** [axon-guard](crates/axon-guard/), [axon-secret](crates/axon-secret/), [axon-sandbox](crates/axon-sandbox/)

### `axon-guard` (§30) — guardrails for inputs and outputs
- `ContentFilter` with detectors for **Email**, **US phone**, **US SSN**, **credit cards** (with Luhn check + word boundaries), **API keys** (`sk-ant-`/`sk-`/`ghp_`/`github_pat_`), **AWS access keys** (`AKIA…`/`ASIA…`), **private-key headers** (RSA / OpenSSH / EC / DSA / PKCS8).
- `Finding` carries a `redacted` preview so logs never show the raw match.
- `injection_score(text)` — heuristic 0..=1 with weighted flags for `IgnorePrevious` / `RoleOverride` / `EmbeddedSystemTag` / `PromptLeak` / `JailbreakLingo` / `SuspiciousBase64Blob`.
- `Policy` — `allow`/`deny` rule list with `Contains` and anchored `Wildcard` matchers, default action, first-match-wins evaluation. JSON-serializable for `axon.toml` integration.

### `axon-secret` (§40) — redaction-aware secrets
- `Secret<T>` — wraps a value so `Debug`, `Display`, and `Serialize` all emit `<redacted>`. The only way to read the inner value is `expose_for_use()`, whose name flags audit-trail usage.
- `Vault` — JSON file with `{ version, secrets: BTreeMap }`. Save uses atomic `tmp → rename`; on Unix the file is created with **mode `0600`** via `OpenOptions::mode`.
- `Vault::load` **rejects insecure permissions** (`mode & 0o077 != 0` → `InsecurePermissions { path, mode }`) with an actionable error message ("Run `chmod 600 …`").

### `axon-sandbox` (§42) — resource-limited subprocesses
- `Limits { cpu_seconds, memory_mb, max_open_files, wall_seconds }` with conservative defaults (10s CPU / 256 MB / 64 FDs / 15s wall).
- `run_sandboxed` applies `setrlimit` via Unix `pre_exec` so the limits take effect **before the child's `execve`**.
- Wall-clock timeout is enforced by the parent via polling + `kill`.
- `SandboxResult` distinguishes `wall_timeout` (parent killed it) from `limit_breached` (kernel signaled, e.g. `SIGXCPU`).
- Windows path is documented as v0 limit — `Limits` are accepted but only wall timeout fires.

### CLI bindings
- `guard_scan_pii(text)`, `guard_scan_secrets(text)`, `guard_injection_score(text)`, `guard_policy_evaluate(json_path, text)`.
- `secret_open(path)`, `secret_get(name)` (returns `<redacted>` by default), `secret_set(name, value)`, `secret_remove(name)`, `secret_names()`, `secret_redact(s)`.
- `sandbox_run(program, args_list, cpu_seconds, memory_mb, wall_seconds)` → `{ exit_code, stdout, stderr, wall_ms, wall_timeout, limit_breached }`.

### CLI demo (real run)
```
--- guardrails ---
3
Email
PhoneUs
SsnUs
2                                  # 2 injection flags
allow                              # policy: "approved: ship it"
deny                               # policy: "key=AKIA..." (deny-aws-key)
--- secrets ---
2                                  # 2 secrets stored
<redacted>                         # secret_get never shows clear value
--- sandbox ---
sandboxed                          # stdout from /bin/sh
0                                  # exit_code
true                               # wall_timeout fired on `sleep 5` with 1s limit
```
The vault file is `-rw-------` on disk — `ls -l` confirms mode `0600`.

---

## Stage 14 — Durable Triggers, Skill Packaging, A2A Discovery

**Crates:** [axon-trigger](crates/axon-trigger/), [axon-skill](crates/axon-skill/), [axon-a2a](crates/axon-a2a/)

### `axon-trigger` (§52) — schedules + in-process scheduler
- `Schedule::Every { period_ns }` / `Schedule::At { when_ns }` / `Schedule::Cron(CronExpr)`.
- `CronExpr::parse` — 5-field POSIX subset (`min hour dom mon dow`) with `*`, lists, ranges, `*/N` steps, and the OR-semantics for restricted dom+dow.
- `Schedule::due_at(last, now)` — coalescing catch-up: a process offline for hours fires once on resume, not once per missed window. Returns the period-grid deadline so persisted state stays aligned.
- `Scheduler::tick(now_ns)` is pure bookkeeping — returns IDs in deterministic id-sorted order so the host can dispatch.
- `save_to_memory(store)` / `load_from_memory(store)` — durable state on top of any `axon_memory::Store`. Restart resumes exactly where the last save left off.
- `RetryPolicy` with bounded `max_attempts` + `backoff_ns`; trigger auto-disables after exhaustion.

### `axon-skill` (§53) — `.axskill` package format
- One file, one JSON document: `{ manifest, files }`. Pure-Rust, deterministic on disk, `cat | jq`-friendly.
- `Manifest`: name, version, entrypoint, capabilities, dependencies, authors, description.
- `Skill::pack(dir)` walks a directory tree (UTF-8 only) and slurps every file relative to the root.
- Every archive carries a **content hash** (FNV-1a over canonical key/body concat). `Skill::verify()` rejects tampered files, unknown format versions, missing entrypoints.
- `Skill::unpack_to(dest)` only writes after verification passes.

### `axon-a2a` (§54) — agent cards & discovery
- `AgentCard` schema matching `/.well-known/agent-card.json`: `agent_id`, `name`, `version`, `endpoint`, `capabilities` (with input/output schema URLs), `auth` (`None`/`ApiKey`/`Bearer`/`OAuth2`), `pricing`, `rate_limits`, free-form metadata.
- `AgentCard::verify()` — required fields non-empty, endpoint is `http(s)://`, capability names unique, schema URLs well-formed.
- `load_card_from_path(path)` for local/test discovery; `fetch_card(base_url)` for HTTPS via `ureq` with a 10-second total timeout.

### CLI bindings
- `trigger_every`, `trigger_at`, `trigger_cron`, `trigger_remove`, `trigger_len`, `trigger_save`, `trigger_load`, and `trigger_tick(now_ns)` (NativeExt — looks up the handler by name in the global env and invokes it).
- `skill_pack(src_dir, dest)`, `skill_install(pkg, dest_dir)`, `skill_inspect(pkg)`.
- `a2a_card_load(path)`, `a2a_card_fetch(url)`, `a2a_card_has_capability(path, name)`.

### CLI demo (real run)
```
--- triggers ---
2                                      # 2 triggers registered
[trigger] heartbeat                    # first tick: heartbeat fires
1
[trigger] running nightly report       # second tick: nightly_at fires
1
--- skill packaging ---
hello-skill
1.0.0
h_ce8bc9a61355bb08                     # content hash
--- a2a discovery ---
Acme Research Agent
https://research.acme.com/agent
2                                      # 2 capabilities
Research
true                                   # has_capability("Summarize")
```

---

## Stage 13 — Orchestration & Reasoning

**Crate:** [axon-flow](crates/axon-flow/), plus a runtime extension (`NativeExtFn` / `Value::NativeExt`).

### `axon-flow` — three production combinators
- `sequential(steps, input)` — pipeline: thread `input` through each `Step` in order; short-circuit on first failure with `sequential[i]` path crumb.
- `parallel(steps, input)` — fan-out: run every `Step` on the *same* input, collect outputs in order. `Vec<Result<...>>` so caller decides on partial failure. `parallel_strict` short-circuits.
- `refine(generate, critique, revise, accept, max_rounds)` — planner-critic loop. Keeps best-so-far; returns `(draft, score, RefineOutcome::Accepted | MaxRounds)`. Matches §49.3 `flow.reflect` shape.
- Generic over `Step<I, O>`; `FnStep` wraps closures; `ScriptedStep` returns pre-recorded outputs (for tests).
- `FlowError` carries a path breadcrumb (`sequential[2]`, `parallel[branch=4]`, `refine[critique:2]`) so failures localize to a step.

### Runtime extension: `NativeExtFn` / `Value::NativeExt`
- New native-fn variant that **takes `&mut Interpreter` plus the call-site span**. Enables host bindings to invoke user closures supplied as arguments.
- `Interpreter::register_native_ext(name, fn)` is the registration point.
- Arity, capability, and error-trace handling all mirror the existing `NativeFn` path.

### CLI bindings (all `NativeExt` so user fns can be supplied as steps)
- `flow_seq(list_of_callables, input)` → final value
- `flow_parallel(list_of_callables, input)` → `List<output>`
- `flow_refine(generate, critique, revise, max_rounds, accept_score)` → `{ draft, score, rounds, outcome }`

### CLI demo (real run)
```
--- pipeline ---
issue #42                              # classify → summarize → polish
--- fan-out ---
research n                             # style_concise
RESEARCH NOTES                         # style_loud
[summary] research notes               # style_label
--- refine ---
accepted                               # outcome
draft+++                               # draft after 3 revisions
3                                      # rounds used
```

---

## Stage 12 — RAG & Multimodal

**Crates:** [axon-rag](crates/axon-rag/), [axon-media](crates/axon-media/)

### `axon-rag` — production retrieval primitives
- `RecursiveChunker` — paragraph → sentence → word → char fallback with configurable overlap.
- `HashEmbedder` — deterministic feature-hashing (FNV-1a) + L2 normalization. Zero network, byte-identical across runs, ideal for tests/replay; same trait surface as future remote embedders.
- `Index` — in-memory vector + lexical store with stable JSON serialization; rehydrates BM25 sidecar on load; idempotent inserts keyed by content-hashed `passage_id`.
- `Bm25` — Okapi BM25+ (k1=1.5, b=0.75) over the same tokenization the embedder uses.
- `Retriever` — hybrid scorer (`α·cosine + (1-α)·BM25_normalized`); top-k with deterministic tie-break.
- CLI bindings: `rag_index_new`, `rag_ingest`, `rag_chunk`, `rag_retrieve`, `rag_save`, `rag_load`, `rag_index_len`.

### `axon-media` — typed multimodal primitives
- `Image::from_bytes` — real header parsers for **PNG IHDR**, **JPEG SOFn**, **GIF LSD**; returns width, height, MIME, byte length without decoding pixels.
- `Audio::from_bytes` — RIFF/WAVE parser; walks `fmt ` and `data` sub-chunks to recover sample rate, channels, bit depth, and duration_ms; rejects non-PCM formats with a typed error.
- `Document::from_bytes` — UTF-8 text with optional form-feed (`\x0C`) page boundaries (the `pdftotext` convention).
- `sniff()` — content-first MIME detection (PNG/JPEG/GIF/WAV/PDF/text/unknown).
- Every parser rejects malformed input with a typed `MediaError` — no panics, no decoder exploits.
- CLI bindings: `media_image_load`, `media_audio_load`, `media_document_load`, `media_sniff`.

### Host + tyck wiring
- Type checker now treats `Dyn.field` as `Dyn` so structured Records returned by native bindings can be drilled into without type ascriptions — propagation through `Dyn` and `Error`, not a blanket relaxation.
- All `rag_*` and `media_*` names added to the type checker's PURE built-in list.

### CLI demo (real run)
```axon
fn main() uses { Console } {
    let img = media_image_load("chart.png")
    print(img.mime)            // "image/png"
    print_int(img.width)        // 800
    print_int(img.height)       // 600

    rag_index_new(512)
    rag_ingest("doc1", "Ferret are small carnivorous mammals ...")
    rag_ingest("doc2", "The stock market closed lower today ...")
    let hits = rag_retrieve("how to train a ferret pet", 2)
    // top hit: "Domestic ferret can be trained to use a litter box..."
    rag_save("kb.json")
    // ... next process can rag_load("kb.json") and pick up where we left off
}
```

---

## Stage 11 — Standard Library & Memory

**Crates:** [axon-std](crates/axon-std/), [axon-memory](crates/axon-memory/)

### `axon-std` — 87 functions across 8 modules
- `std.string` (16): `str_upper`, `str_lower`, `str_trim*`, `str_split`, `str_join`, `str_contains`, `str_starts_with`/`ends_with`, `str_replace`, `str_repeat`, `str_len`, `str_chars`, `str_index_of`, `str_substring`.
- `std.list` (16): `list_new`/`len`/`push`/`pop`/`get`/`set`, `first`/`last`/`contains`, `reverse`/`sort`, `take`/`drop`/`concat`, `index_of`, `remove_at`.
- `std.map` (10): insertion-ordered KV — `map_new`/`len`/`get`/`get_or`/`set`/`remove`/`contains`/`keys`/`values`/`merge`.
- `std.set` (9): dedup-preserving — `set_new`/`add`/`remove`/`contains`/`union`/`intersection`/`difference`/`to_list`/`len`.
- `std.option` (6) + `std.result` (7): first-order helpers; `Result` uses tagged `Instance` so pattern matching works uniformly.
- `std.math` (14): `pow`, `sqrt`, `floor`/`ceil`/`round`, `sin`/`cos`/`tan`, `log`/`log2`, `exp`, `pi`/`e`, `gcd`.
- `std.time` (9): `dur_seconds`/`millis` and reverse, `date_year/month/day`, `date_make` (validates day-of-month), `date_is_leap`.

### `axon-memory` — pluggable persistent stores
- `EphemeralStore` — in-process `BTreeMap`.
- `FileStore` — JSON-backed with **atomic writes** (write `.tmp`, fsync, rename) so a partial process kill never leaves the file half-written.
- Single `Store` trait — downstream code holds `Arc<dyn Store>` and never cares which backend.
- Sorted-key snapshots → deterministic on-disk output.
- Schema versioning with explicit rejection of unknown versions.
- `forget_tagged()` and `forget_older_than()` for retention/GDPR-style passes.

### Host wiring
- `Interpreter::register_native()` is a new public method so downstream crates plug into the runtime without modification.
- `axon-cli` exposes 8 `mem_*` built-ins (open file, open ephemeral, set, get, remove, keys, len, contains) backed by `axon-memory`, with automatic `Value`↔`serde_json::Value` conversion.
- Type checker's built-in list extended so all stdlib + `mem_*` names type-check.

### CLI demo (real, end-to-end)
```axon
fn main() uses { Console } {
    mem_open_file("/tmp/wordcount.json")
    let words = str_split(str_lower(sentence), " ")
    let counts = map_new()
    // ... (frequency map via list_get/map_set) ...
    mem_set("top_word", best_word)   // persists across processes
    mem_set("top_count", best_count)
    print_int(best_count)
}
```
The JSON file survives the process exit and is read back in the next run.

---

## Stage 10 — Doc Site Generator & Formatter

**Crates:** [axon-doc](crates/axon-doc/), [axon-fmt](crates/axon-fmt/)

### `axon doc`
- Walks a `LoadedProject`, pairs `///` doc comments with the items that follow.
- Emits one HTML page per module + an index, with an embedded stylesheet.
- Markdown rendering via `pulldown-cmark`; signatures and doc bodies HTML-escaped.
- Public vs. private items get distinct CSS classes.
- Item-prefix tokens (`pub`, `async`, attributes) correctly bridged so doc comments attach to the right item.
- CLI: `axon doc <path> [-o dir]`.

### `axon fmt`
- Token-stream-based formatter — re-emits each lexer token with canonical spacing rules.
- 4-space indent tracked by `(`, `[`, `{` nesting.
- Canonical spacing for binary ops, `,`, `:`, `->`, `=>`, `|>`, unary prefixes, call-attach.
- Blank-line collapsing (runs collapse to one); always-trailing newline.
- Idempotent: `format(format(x)) == format(x)`, pinned by tests.
- CLI: `axon fmt <path> [--write] [--check]` — `--check` exits non-zero on diff (CI-friendly).

---

## CLI surface today

```
axon run    <file>           # interpret (Stage 3+)
axon test   [path]           # run #[test] items (Stage 8)
axon build  <file> -o out    # compile to WASM (Stage 9)
axon doc    <path> [-o dir]  # generate HTML doc site (Stage 10)
axon fmt    <path> [--write] [--check]   # canonical formatter (Stage 10)
axon lsp                     # language server (Stage 9.5)
```

## Test count by area

| Crate | Tests |
|---|---|
| axon-ast | 3 |
| axon-diag | 5 |
| axon-doc | 5 |
| axon-fmt | 12 |
| axon-lexer | 24 |
| axon-lsp | 7 |
| axon-models | 12 |
| axon-parser | 13 |
| axon-project | 9 |
| axon-runtime | 9 + 16 + 11 + 15 + 11 + 28 + 9 + 27 (per-module suites) |
| axon-tyck | 30 |
| axon-types | — |
| axon-vm | 12 |
| axon-wasm | — (runs via integration tests) |
| **Total** | **253 passing** |

## Workspace shape

```
crates/
├── axon-ast/        # syntax tree types + spans
├── axon-cli/        # the `axon` binary
├── axon-diag/       # diagnostics + source registry
├── axon-doc/        # static HTML doc site generator   ← Stage 10
├── axon-fmt/        # canonical formatter              ← Stage 10
├── axon-lexer/      # tokenizer
├── axon-lsp/        # language server
├── axon-models/     # LLM client + prompts + tool loop
├── axon-parser/     # recursive-descent + Pratt parser
├── axon-project/    # axon.toml + module loading
├── axon-runtime/    # interpreter + caps + actors + tracing + budgets + replay
├── axon-tyck/       # bidirectional type checker
├── axon-types/      # core type representation
├── axon-vm/         # AxVM bytecode VM
└── axon-wasm/       # WebAssembly codegen
```
