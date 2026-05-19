# Axon — Implemented Features

A snapshot of everything Axon ships today, grouped by the stages that introduced each capability. All features below are covered by the workspace test suite (**253 tests passing** across 27 crates).

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
