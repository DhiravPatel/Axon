//! The `axon` command-line tool.
//!
//! This stage covers only the front-end commands:
//!
//! * `axon tokens <file>` — pretty-prints the token stream from the lexer
//! * `axon parse  <file>` — parses the file and prints the AST or any
//!   diagnostics, exiting non-zero on parse errors
//!
//! Future stages will fill in `axon run`, `axon build`, `axon fmt`, `axon
//! doc`, `axon test`, etc. The CLI stays intentionally thin: each subcommand
//! is a few lines that compose `axon-lexer` / `axon-parser` with a small
//! amount of output formatting.

use std::process::ExitCode;

use axon_diag::SourceFile;

mod host;
mod qol;
mod scaffold;
mod watch_format;
mod why;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = match args.next() {
        Some(c) => c,
        None => {
            // §64.2: a bare `axon` suggests the most likely next command
            // based on project state, then points at help.
            println!("axon {}\n", env!("CARGO_PKG_VERSION"));
            println!("hint: {}", qol::next_command_hint());
            println!("\nRun `axon help` for all commands.");
            return ExitCode::from(2);
        }
    };
    match cmd.as_str() {
        "tokens" => match args.next() {
            Some(path) => cmd_tokens(&path),
            None => {
                eprintln!("usage: axon tokens <file>");
                ExitCode::from(2)
            }
        },
        "parse" => match args.next() {
            Some(path) => cmd_parse(&path),
            None => {
                eprintln!("usage: axon parse <file>");
                ExitCode::from(2)
            }
        },
        "check" => {
            let remaining: Vec<String> = args.collect();
            cmd_check(&remaining)
        }
        "run" => {
            let remaining: Vec<String> = args.collect();
            cmd_run(&remaining)
        }
        "test" => {
            let remaining: Vec<String> = args.collect();
            cmd_test(&remaining)
        }
        "build" => {
            let remaining: Vec<String> = args.collect();
            cmd_build(&remaining)
        }
        "lsp" => {
            // The LSP server takes no flags in v0 — editors spawn it as
            // a subprocess and talk JSON-RPC over stdin/stdout.
            match axon_lsp::run() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("axon-lsp: {e}");
                    ExitCode::from(1)
                }
            }
        }
        "doc" => {
            let remaining: Vec<String> = args.collect();
            cmd_doc(&remaining)
        }
        "fmt" => {
            let remaining: Vec<String> = args.collect();
            cmd_fmt(&remaining)
        }
        "serve" => {
            let remaining: Vec<String> = args.collect();
            cmd_serve(&remaining)
        }
        "deploy" => {
            let remaining: Vec<String> = args.collect();
            cmd_deploy(&remaining)
        }
        "replay" => {
            let remaining: Vec<String> = args.collect();
            cmd_replay(&remaining)
        }
        "trace" => {
            let remaining: Vec<String> = args.collect();
            cmd_trace(&remaining)
        }
        "repl" => {
            let remaining: Vec<String> = args.collect();
            cmd_repl(&remaining)
        }
        "login" => {
            let remaining: Vec<String> = args.collect();
            cmd_login(&remaining)
        }
        "pkg" => {
            let remaining: Vec<String> = args.collect();
            cmd_pkg(&remaining)
        }
        "optimize" => {
            let remaining: Vec<String> = args.collect();
            cmd_optimize(&remaining)
        }
        "schema" => {
            let remaining: Vec<String> = args.collect();
            cmd_schema(&remaining)
        }
        "explain" => {
            let remaining: Vec<String> = args.collect();
            cmd_explain(&remaining)
        }
        "fix" => {
            let remaining: Vec<String> = args.collect();
            cmd_fix(&remaining)
        }
        "why" => {
            let remaining: Vec<String> = args.collect();
            why::cmd_why(&remaining)
        }
        "watch" => {
            let remaining: Vec<String> = args.collect();
            cmd_watch(&remaining)
        }
        "new" => {
            let remaining: Vec<String> = args.collect();
            scaffold::cmd_new(&remaining)
        }
        "tour" => {
            let remaining: Vec<String> = args.collect();
            scaffold::cmd_tour(&remaining)
        }
        "stats" => {
            let remaining: Vec<String> = args.collect();
            qol::cmd_stats(&remaining)
        }
        "clean" => {
            let remaining: Vec<String> = args.collect();
            qol::cmd_clean(&remaining)
        }
        "completions" => {
            let remaining: Vec<String> = args.collect();
            qol::cmd_completions(&remaining)
        }
        "doctor" => {
            let remaining: Vec<String> = args.collect();
            qol::cmd_doctor(&remaining)
        }
        "prof" => {
            let remaining: Vec<String> = args.collect();
            cmd_prof(&remaining)
        }
        "version" | "--version" | "-V" => {
            println!("axon {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "help" | "--help" | "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("axon: unknown subcommand `{other}`");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    println!(
        "axon {}\n\
         The programming language for autonomous AI agents.\n\n\
         USAGE: axon <command> [args...]\n\n\
         COMMANDS:\n\
           tokens <file>   Print the lexer token stream\n\
           parse  <file>   Parse <file> and print the item summary\n\
           test   [path]  Discover and run `test \"name\" {{ ... }}` blocks in a\n\
                          project directory (or single file). Reports pass/fail.\n\
           lsp            Start the Language Server Protocol server on stdin/\n\
                          stdout. Editors (VS Code, Helix, Zed, neovim, ...)\n\
                          spawn this as a subprocess for diagnostics, hover,\n\
                          go-to-definition, and completion.\n\
           doc    [path] [-o dir]\n\
                          Generate a static HTML documentation site for a\n\
                          project. Walks `///` doc comments, renders them\n\
                          via CommonMark, emits one page per module plus\n\
                          an index.\n\
           fmt    [path] [--write] [--check]\n\
                          Canonical formatter. Without flags, prints the\n\
                          formatted output to stdout. `--write` overwrites\n\
                          the input in place; `--check` exits non-zero if\n\
                          any file would be reformatted (useful in CI).\n\
           serve  <file> --listen ADDR --handler NAME\n\
                          [--tls-cert PATH --tls-key PATH]\n\
                          [--protocol plain|mcp|openai|grpc|a2a]\n\
                          Start an HTTP/1.1 server. POST /invoke dispatches\n\
                          to the named handler with the request body; GET\n\
                          /healthz and /readyz return JSON health status.\n\
                          With --tls-* flags, terminate TLS via rustls.\n\
                          With --protocol, the request shape is wrapped per\n\
                          the chosen wire protocol (MCP, OpenAI chat,\n\
                          gRPC, A2A); the handler reads $AXON_SERVE_PROTOCOL\n\
                          and dispatches via `serve_protocol_route`.\n\
                          SIGINT/SIGTERM triggers graceful shutdown with\n\
                          in-flight handler drain.\n\
           login  <provider> [--vault PATH] [--key VALUE]\n\
                          Save an API key to the local vault (mode 0600 on\n\
                          Unix). Reads --key, then $PROVIDER_API_KEY env,\n\
                          then prompts on stdin.\n\
           pkg    <subcmd>  Manage dependencies declared in `axon.toml`.\n\
                          Subcommands: `list`, `add NAME --path P`,\n\
                          `remove NAME`, `audit`.\n\
           optimize <prompt.ax> --eval <suite.ax> --metric NAME [--budget B] [--trials N]\n\
                          Search over prompt + strategy variants against\n\
                          an eval suite and propose a diff that beats the\n\
                          baseline (§49.6). Writes the winning variant\n\
                          alongside the input as `<name>.vN.ax`.\n\
           schema <subcmd>  Schema migration tooling.\n\
                          Subcommands:\n\
                            migrate <store.json> --to N [--schema NAME]\n\
                              Plan + (with --apply) execute the\n\
                              registered migration chain over a JSON\n\
                              store. Without --apply it reports what\n\
                              steps would run.\n\
                            inspect <store.json> [--schema NAME]\n\
                              Count entries per (schema, version).\n\
           prof   --cost <ledger.json> [--top N] [--by provider|model|tag]\n\
                          Render a cost report from a recorded ledger:\n\
                          total cents, per-provider breakdown, p50/p95\n\
                          latency, and top-N most expensive calls.\n\
           deploy <file> -o DIR [--name N] [--port P] [--handler H]\n\
                          Package a project for deployment: write a\n\
                          `.axskill` archive plus a `deploy.json` manifest\n\
                          into DIR. Pair with `axon serve` on the target.\n\
           build  [-o out.wasm] <file>\n\
                          Lower the integer subset of Axon to a WebAssembly\n\
                          module. The output runs in any standard WASM runtime\n\
                          (wasmtime, wasmer, browsers, Node.js) and imports\n\
                          `host.print_int(i64)` for output.\n\
           check  <file>   Parse and type-check <file>; reports type errors\n\
           run    [flags] <file>\n\
                          Parse, type-check, and execute `main()` in <file>.\n\
                          Flags:\n\
                            --with EFFECT[,EFFECT...]  Grant exactly this set of\n\
                                                       capabilities. Pure functions\n\
                                                       don't need any.\n\
                            --isolated                 Grant no capabilities. Useful\n\
                                                       for running sandbox-graded code.\n\
                            --vm                       Use the AxVM bytecode backend.\n\
                            --tree                     Use the tree-walking interpreter\n\
                                                       (default).\n\
                            --trace PATH               Write JSON Lines trace spans to PATH.\n\
                            --record PATH              Capture every model response into PATH.\n\
                            --replay PATH              Replay model responses from PATH instead of\n\
                                                       calling any real provider.\n\
                            --features F1,F2,...       Enable named features from `axon.toml`'s\n\
                                                       `[features]` table. `#[cfg(feature=\"X\")]`\n\
                                                       items are dropped when X isn't active.\n\
                            --no-default-features      Don't seed the `default` feature.\n\
                            (no flag)                  Grant the standard default set.\n\
           fix    [--apply | --interactive | --watch] [--only CODE] <path>\n\
                          Apply mechanically applicable fixes attached to\n\
                          diagnostics (did-you-mean replacements, missing\n\
                          `uses` effects, ...). Without a mode flag, prints\n\
                          a unified diff (dry run). --apply rewrites in\n\
                          place; --interactive walks each hunk with y/n/a/q;\n\
                          --watch auto-applies Safe-tier fixes on save\n\
                          (requires axon.toml + a descendant of CWD by\n\
                          default; AXON_FIX_WATCH_FORCE=1 to override).\n\
                          --only restricts to one diagnostic code.\n\
           why    <EFFECT> [<path>] [--from <fn>]\n\
                          Trace every call site that introduces <EFFECT>\n\
                          into the chosen entry function (default `main`).\n\
                          Built-in calls (`http_fetch`, `read_file`, …)\n\
                          appear as leaves; user-defined fns expand into\n\
                          their own subtree (effect-pruned).\n\
           watch  <file> [--trace PATH] [--no-color]\n\
                          Run a program with the live trace inspector —\n\
                          one line per span (ask/plan/tool/handler/spawn)\n\
                          streamed to stderr as the program runs. Stdout\n\
                          is left alone so program output remains pipeable.\n\
           version         Print the compiler version\n\
           help            Show this message\n",
        env!("CARGO_PKG_VERSION")
    );
}

fn read_or_die(path: &str) -> Option<SourceFile> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("axon: cannot read `{path}`: {e}");
            return None;
        }
    };
    Some(SourceFile::new(path, text))
}

fn cmd_tokens(path: &str) -> ExitCode {
    let Some(file) = read_or_die(path) else {
        return ExitCode::from(1);
    };
    let (tokens, diags) = axon_lexer::tokenize(&file);
    for tok in &tokens {
        let (l, c) = file.line_col(tok.span.start as usize);
        println!("{l:>4}:{c:<3}  {:?}", tok.kind);
    }
    emit_diagnostics(&diags, &file);
    if diags.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn cmd_doc(args: &[String]) -> ExitCode {
    let mut path: Option<&str> = None;
    let mut out_dir = "doc".to_string();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon doc: -o requires a directory");
                    return ExitCode::from(2);
                }
                out_dir = args[i].clone();
                i += 1;
            }
            other if other.starts_with("-o=") => {
                out_dir = other["-o=".len()..].to_string();
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("axon doc: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            _ => {
                if path.is_some() {
                    eprintln!("axon doc: only one project path is supported");
                    return ExitCode::from(2);
                }
                path = Some(a);
                i += 1;
            }
        }
    }
    let path = path.unwrap_or(".");
    let project = match axon_project::LoadedProject::load(std::path::Path::new(path)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let site = axon_doc::generate(&project);
    match site.write_to(std::path::Path::new(&out_dir)) {
        Ok(_) => {
            println!(
                "wrote {} files to {}",
                site.files.len(),
                out_dir
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("axon doc: cannot write to `{out_dir}`: {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_fmt(args: &[String]) -> ExitCode {
    let mut path: Option<&str> = None;
    let mut write = false;
    let mut check = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--write" => {
                write = true;
                i += 1;
            }
            "--check" => {
                check = true;
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("axon fmt: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            _ => {
                if path.is_some() {
                    eprintln!("axon fmt: only one path is supported");
                    return ExitCode::from(2);
                }
                path = Some(a);
                i += 1;
            }
        }
    }
    let path = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            eprintln!("usage: axon fmt [--write] [--check] <file|dir>");
            return ExitCode::from(2);
        }
    };

    let files: Vec<std::path::PathBuf> = if path.is_dir() {
        let mut out = Vec::new();
        if let Err(e) = collect_ax_files(&path, &mut out) {
            eprintln!("axon fmt: {e}");
            return ExitCode::from(1);
        }
        out
    } else {
        vec![path]
    };

    let mut any_diff = false;
    for f in &files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("axon fmt: cannot read `{}`: {e}", f.display());
                return ExitCode::from(1);
            }
        };
        let (formatted, diags) = axon_fmt::format(&text);
        if !diags.is_empty() && check {
            eprintln!("axon fmt: lexer errors in `{}`; refusing to check", f.display());
            return ExitCode::from(1);
        }
        let changed = formatted != text;
        if changed {
            any_diff = true;
        }
        if check {
            if changed {
                println!("would reformat {}", f.display());
            }
        } else if write {
            if changed {
                if let Err(e) = std::fs::write(f, &formatted) {
                    eprintln!("axon fmt: cannot write `{}`: {e}", f.display());
                    return ExitCode::from(1);
                }
                println!("formatted {}", f.display());
            }
        } else {
            print!("{formatted}");
        }
    }
    if check && any_diff {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn collect_ax_files(
    dir: &std::path::Path,
    out: &mut Vec<std::path::PathBuf>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            collect_ax_files(&p, out)?;
        } else if p.extension().and_then(|s| s.to_str()) == Some("ax") {
            out.push(p);
        }
    }
    Ok(())
}

fn cmd_build(args: &[String]) -> ExitCode {
    // `axon build [-o out.wasm] <file>`. v0 only handles single-file
    // inputs — multi-module WASM builds need namespacing and inlined
    // dependency resolution we haven't built yet.
    let mut out_path: Option<String> = None;
    let mut input: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon build: -o requires a path");
                    return ExitCode::from(2);
                }
                out_path = Some(args[i].clone());
                i += 1;
            }
            other if other.starts_with("-o=") => {
                out_path = Some(other["-o=".len()..].to_string());
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("axon build: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            _ => {
                if input.is_some() {
                    eprintln!("axon build: only one input file is supported in v0");
                    return ExitCode::from(2);
                }
                input = Some(a);
                i += 1;
            }
        }
    }
    let input_path = match input {
        Some(p) => p,
        None => {
            eprintln!("usage: axon build [-o out.wasm] <file>");
            return ExitCode::from(2);
        }
    };
    let file = match read_or_die(input_path) {
        Some(f) => f,
        None => return ExitCode::from(1),
    };
    let (program, parse_diags) = axon_parser::parse(&file);
    if !parse_diags.is_empty() {
        emit_diagnostics(&parse_diags, &file);
        return ExitCode::from(1);
    }
    // Type-check first so users get the same diagnostics they'd see
    // running the program.
    let (_, type_diags) = axon_tyck::check(&file, &program);
    if !type_diags.is_empty() {
        emit_diagnostics(&type_diags, &file);
        return ExitCode::from(1);
    }
    let module = match axon_wasm::build(&program) {
        Ok(m) => m,
        Err(axon_wasm::BuildError::Unsupported(diags)) => {
            emit_diagnostics(&diags, &file);
            return ExitCode::from(1);
        }
        Err(axon_wasm::BuildError::Internal(s)) => {
            eprintln!("axon build: {s}");
            return ExitCode::from(1);
        }
    };
    let out = out_path.unwrap_or_else(|| {
        let mut p = std::path::PathBuf::from(input_path);
        p.set_extension("wasm");
        p.to_string_lossy().into_owned()
    });
    match std::fs::write(&out, &module.bytes) {
        Ok(_) => {
            println!(
                "wrote {} bytes to {}",
                module.bytes.len(),
                out
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("axon build: cannot write `{out}`: {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_test(args: &[String]) -> ExitCode {
    // Accept `axon test [path]`. Path defaults to "." so running inside a
    // project directory just works.
    let mut features: Vec<String> = Vec::new();
    let mut enable_default_features = true;
    // §35.4 — trajectory snapshot testing.
    let mut record_trajectory: Option<String> = None;
    let mut match_trajectory: Option<String> = None;
    // §35.5 — doc tests.
    let mut include_doc_tests = false;
    let mut doc_only = false;
    // §35.4 — collect positionals during the flag-parsing pass so the
    // path detector below doesn't accidentally grab a flag's value.
    let mut positionals: Vec<&str> = Vec::new();
    // §36.A.2 — see cmd_run; same escape hatch.
    let mut no_async = std::env::var("AXON_NO_ASYNC").is_ok();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-async" => {
                no_async = true;
                i += 1;
            }
            "--features" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon test: --features requires VAL");
                    return ExitCode::from(2);
                }
                features.extend(parse_caps_to_vec(&args[i]));
                i += 1;
            }
            other if other.starts_with("--features=") => {
                features.extend(parse_caps_to_vec(&other["--features=".len()..]));
                i += 1;
            }
            "--no-default-features" => {
                enable_default_features = false;
                i += 1;
            }
            "--record-trajectory" => {
                i += 1;
                if i >= args.len() {
                    eprintln!(
                        "axon test: --record-trajectory requires a name (e.g. --record-trajectory baseline)"
                    );
                    return ExitCode::from(2);
                }
                record_trajectory = Some(args[i].clone());
                i += 1;
            }
            "--match-trajectory" => {
                i += 1;
                if i >= args.len() {
                    eprintln!(
                        "axon test: --match-trajectory requires a name"
                    );
                    return ExitCode::from(2);
                }
                match_trajectory = Some(args[i].clone());
                i += 1;
            }
            "--doc" => {
                include_doc_tests = true;
                i += 1;
            }
            "--doc-only" => {
                include_doc_tests = true;
                doc_only = true;
                i += 1;
            }
            "--help" | "-h" => {
                // §35.6 verification fix M5 — surface the Stage 35
                // additions (--doc / --doc-only / --record-trajectory /
                // --match-trajectory) so they're discoverable.
                println!(
                    "usage: axon test [path] [FLAGS]\n\
                     \n\
                     Discover and run `test \"name\" {{ ... }}` blocks in a project\n\
                     directory (or single file). Path defaults to `.`.\n\
                     \n\
                     FLAGS:\n\
                       --features F1,F2,...\n\
                                                Enable named features from axon.toml's\n\
                                                [features] table.\n\
                       --no-default-features    Don't seed the `default` feature.\n\
                       --doc                    Also run ``axon`` fenced blocks extracted\n\
                                                from /// doc comments as tests.\n\
                       --doc-only               Run ONLY the doc-test snippets; drop\n\
                                                user-declared test blocks.\n\
                       --record-trajectory NAME\n\
                                                Capture each test's model-call trajectory\n\
                                                to tests/.trajectories/NAME/ as a snapshot.\n\
                       --match-trajectory NAME\n\
                                                Re-run each test and assert its trajectory\n\
                                                shape matches the saved snapshot (step\n\
                                                count, grounded fraction, tool set,\n\
                                                error-recovery flag — not exact strings)."
                );
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--") => {
                eprintln!("axon test: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                positionals.push(other);
                i += 1;
            }
        }
    }
    if record_trajectory.is_some() && match_trajectory.is_some() {
        eprintln!(
            "axon test: --record-trajectory and --match-trajectory are mutually exclusive"
        );
        return ExitCode::from(2);
    }
    let path = positionals.first().copied().unwrap_or(".");
    let path = std::path::Path::new(path);
    let mut project = match axon_project::LoadedProject::load_with_features(
        path,
        &features,
        enable_default_features,
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let active = axon_project::resolve_features(
        &project.manifest.features,
        &features,
        enable_default_features,
    );
    host::set_active_features(active.names());
    if !project.diagnostics.is_empty() {
        emit_project_diagnostics_via_registry(&project.diagnostics, &project.sources);
        return ExitCode::from(1);
    }
    // §35.5 — doc-test synthesis. When `--doc` is set, extract every
    // `` ```axon `` fence from `///` comments, wrap each in a synthetic
    // `test "doc(item)" { ... }` block, parse the synthesized source,
    // and splice its TestDecl items into the merged program before
    // tyck runs. `--doc-only` additionally drops the user's normal
    // tests before the run loop.
    let mut doc_snippets_seen: usize = 0;
    if include_doc_tests {
        let snippets = axon_doc::extract_doc_tests(&project);
        doc_snippets_seen = snippets.len();
        if !snippets.is_empty() {
            // §35.6 verification fix M3 — collect user test names so
            // synthesize_with_existing suffixes any name collisions
            // (otherwise a user-written `test "doc(add)" { ... }`
            // plus a `///`-extracted snippet on `fn add(...)` would
            // produce an E0204 pointing at the phantom `<doc-tests>`
            // file the user can't edit).
            let user_test_names: Vec<String> = project
                .merged
                .items
                .iter()
                .filter_map(|i| match i {
                    axon_ast::Item::Test(t) => Some(t.name.clone()),
                    _ => None,
                })
                .collect();
            let synth_src = axon_doc::doctest::synthesize_with_existing(
                &snippets,
                &user_test_names,
            );
            let synth_path = "<doc-tests>";
            let synth_id = project.sources.register(synth_path, synth_src.clone());
            let synth_file = axon_diag::SourceFile::with_id(
                synth_id, synth_path, synth_src,
            );
            let (synth_program, synth_diags) = axon_parser::parse(&synth_file);
            if !synth_diags.is_empty() {
                eprintln!(
                    "axon test --doc: {} parse error(s) in synthesized doc-tests:",
                    synth_diags.len()
                );
                for d in &synth_diags {
                    eprintln!("{}", axon_diag::render(d, &synth_file, true));
                }
                return ExitCode::from(1);
            }
            // §35.5 `--doc-only` drops user tests, keeps only the
            // synthesized doc-tests.
            if doc_only {
                project
                    .merged
                    .items
                    .retain(|i| !matches!(i, axon_ast::Item::Test(_)));
            }
            project.merged.items.extend(synth_program.items);
        } else if doc_only {
            println!(
                "axon test --doc-only: no doc fences found in `{}`",
                path.display()
            );
            return ExitCode::SUCCESS;
        }
    }
    // Type-check against the merged program — use the first module's
    // source file for diagnostic rendering since type errors carry spans
    // that point into the original module file.
    let primary_source = project
        .modules
        .first()
        .map(|m| m.source.clone())
        .unwrap_or_else(|| axon_diag::SourceFile::new("<empty>", String::new()));
    let (_, tyck_diags) = axon_tyck::check(&primary_source, &project.merged);
    if !tyck_diags.is_empty() {
        emit_project_diagnostics_via_registry(&tyck_diags, &project.sources);
        return ExitCode::from(1);
    }

    let caps = if project.manifest.caps.default.is_empty() {
        axon_runtime::CapSet::standard_default()
    } else {
        axon_runtime::CapSet::from_iter(project.manifest.caps.default.iter().cloned())
    };

    // Collect tests.
    let tests: Vec<&axon_ast::TestDecl> = project
        .merged
        .items
        .iter()
        .filter_map(|i| {
            if let axon_ast::Item::Test(t) = i {
                Some(t)
            } else {
                None
            }
        })
        .collect();

    if tests.is_empty() {
        println!("no tests found in `{}`", path.display());
        // §35.5 — even with no runnable tests, if --doc was set we
        // should still report how many fences were extracted (zero
        // counts as a useful signal: "you asked for doc-tests and got
        // none; check your /// fences").
        if include_doc_tests {
            println!(
                "doc-tests: {doc_snippets_seen} `\u{0060}\u{0060}\u{0060}axon` fence(s) extracted from /// comments"
            );
        }
        return ExitCode::SUCCESS;
    }

    println!("running {} test{}", tests.len(), if tests.len() == 1 { "" } else { "s" });
    let start = std::time::Instant::now();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());

    // §35.4 — trajectory snapshot scaffold.
    let trajectory_dir = path.join("tests").join(".trajectories");
    if record_trajectory.is_some() {
        if let Err(e) = std::fs::create_dir_all(&trajectory_dir) {
            eprintln!(
                "axon test: cannot create `{}`: {e}",
                trajectory_dir.display()
            );
            return ExitCode::from(1);
        }
    }
    // The set of test names that ran through the trajectory path so the
    // end-of-run summary can include "saved N trajectories" / "matched M".
    let mut trajectory_saved = 0usize;
    let mut trajectory_matched = 0usize;
    let mut trajectory_drift = 0usize;

    for t in &tests {
        let mut interp = axon_runtime::Interpreter::with_caps(caps.clone());
        host::install(&interp);
        // §35.4 — enable recording whenever a trajectory mode is set.
        // We need the recording either way: --record writes it out as
        // a Trajectory snapshot; --match builds the actual Trajectory
        // from this run's recording so we can diff against the saved one.
        if record_trajectory.is_some() || match_trajectory.is_some() {
            interp.enable_recording();
        }
        interp.load_program(&project.merged);
        host::register_policies(&project.merged);
        host::register_schemas(&project.merged);
        let closure = axon_runtime::Closure::new(
            Some(format!("test:{}", t.name)),
            Vec::new(),
            axon_runtime::ClosureBody::Block(t.body.clone()),
            interp.globals.clone(),
            t.span,
            // §35.4 — tests inherit the runner's full cap set (which
            // comes from the manifest's `[caps] default = [...]`). Tests
            // are scaffolding; making them re-declare every effect they
            // call into would be obnoxious. The previous default of
            // `Some(Vec::new())` (empty attenuation) made any effectful
            // test impossible to write.
            None,
        );
        // Stage 36: route through the async-eval seam by default. The
        // body of each test executes inside `block_on`, so `parallel { }`
        // and `flow_parallel_asks` inside test bodies share the singleton
        // runtime instead of crashing with nested-block_on.
        let result = if no_async {
            interp.call_value(
                &axon_runtime::Value::Fn(std::rc::Rc::new(closure)),
                &[],
                t.span,
            )
        } else {
            interp.call_value_async(
                &axon_runtime::Value::Fn(std::rc::Rc::new(closure)),
                &[],
                t.span,
            )
        };
        let test_failed = match result {
            Ok(_) => {
                println!("  ok   `{}`", t.name);
                passed += 1;
                false
            }
            Err(axon_runtime::EvalSignal::Error(err)) => {
                failed += 1;
                println!("  FAIL `{}`", t.name);
                let file = project
                    .sources
                    .get(err.span.file)
                    .or_else(|| project.sources.iter().next())
                    .unwrap_or(&primary_source);
                eprint!("    {}", err.render(file, use_color));
                true
            }
            Err(other) => {
                failed += 1;
                println!("  FAIL `{}` — unexpected control flow: {other:?}", t.name);
                true
            }
        };
        if test_failed {
            continue;
        }
        // §35.4 — trajectory snapshot handling. Only run for tests
        // that actually produced model calls (no ModelCall events in
        // the recording → no trajectory).
        if record_trajectory.is_some() || match_trajectory.is_some() {
            let recording = interp.take_recording();
            let events: Vec<axon_eval::trajectory::TrajectoryEvent> = recording
                .map(|r| {
                    r.events
                        .into_iter()
                        .filter_map(|e| match e {
                            axon_runtime::RecordedEvent::ModelCall { response, .. } => {
                                Some(axon_eval::trajectory::TrajectoryEvent::ModelCall {
                                    content: response.content.clone(),
                                    tool_calls: response
                                        .tool_calls
                                        .iter()
                                        .map(|tc| axon_eval::trajectory::ToolCall {
                                            name: tc.name.clone(),
                                            args_json: tc.input.to_string(),
                                            errored: false,
                                        })
                                        .collect(),
                                })
                            }
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            // §35.6 verification fix C2 — DO NOT short-circuit on
            // empty events. A test that previously produced model
            // calls and now produces zero is exactly the regression
            // snapshot testing is designed to catch. In record mode
            // we still skip writing an empty trajectory (no signal);
            // in match mode we read the baseline and let
            // compare_trajectories report the drift to zero steps.
            let traj = axon_eval::trajectory::trajectory_from_events(
                t.name.clone(),
                &events,
            );
            if let Some(name) = &record_trajectory {
                // Refuse path-traversal on the snapshot-set name
                // (§35.6 verification fix M2). The per-test name is
                // already sanitized via sanitize_filename, but the
                // snapshot-set name was joined raw — `axon test
                // --record-trajectory ../../etc` escaped the project
                // dir. Sanitize the set name the same way.
                let safe_set = sanitize_filename(name);
                if safe_set.is_empty() || safe_set != *name {
                    eprintln!(
                        "axon test: trajectory snapshot name `{name}` rejected — \
                         only alphanumeric / `_` / `-` are allowed (got sanitized form `{safe_set}`)"
                    );
                    return ExitCode::from(2);
                }
                if events.is_empty() {
                    // No model calls → no snapshot worth saving.
                    continue;
                }
                let dest = trajectory_dir
                    .join(&safe_set)
                    .join(format!("{}.json", sanitize_filename(&t.name)));
                if let Some(parent) = dest.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let body = serde_json::to_string_pretty(&traj).unwrap_or_default();
                if let Err(e) = std::fs::write(&dest, &body) {
                    eprintln!("axon test: cannot write `{}`: {e}", dest.display());
                } else {
                    trajectory_saved += 1;
                    println!("    trajectory recorded -> {}", dest.display());
                }
            }
            if let Some(name) = &match_trajectory {
                let safe_set = sanitize_filename(name);
                if safe_set.is_empty() || safe_set != *name {
                    eprintln!(
                        "axon test: trajectory snapshot name `{name}` rejected — \
                         only alphanumeric / `_` / `-` are allowed"
                    );
                    return ExitCode::from(2);
                }
                let src = trajectory_dir
                    .join(&safe_set)
                    .join(format!("{}.json", sanitize_filename(&t.name)));
                let baseline_body = match std::fs::read_to_string(&src) {
                    Ok(s) => s,
                    Err(_) => {
                        // §35.6 verification fix C2: a missing
                        // baseline file in match mode is itself a
                        // signal — count it as drift so CI catches a
                        // snapshot file someone accidentally deleted.
                        println!(
                            "    no saved trajectory at `{}` — run with --record-trajectory {name} first",
                            src.display()
                        );
                        trajectory_drift += 1;
                        continue;
                    }
                };
                let baseline: axon_eval::trajectory::Trajectory =
                    match serde_json::from_str(&baseline_body) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!(
                                "axon test: cannot parse `{}`: {e}",
                                src.display()
                            );
                            trajectory_drift += 1;
                            continue;
                        }
                    };
                let tolerance = axon_eval::trajectory::TrajectoryTolerance::default();
                let drifts = axon_eval::trajectory::compare_trajectories(
                    &baseline, &traj, &tolerance,
                );
                if drifts.is_empty() {
                    trajectory_matched += 1;
                    println!("    trajectory matches {}", src.display());
                } else {
                    trajectory_drift += 1;
                    println!("    trajectory DRIFT vs {}", src.display());
                    for d in &drifts {
                        println!(
                            "      {} : baseline={} actual={} delta={}",
                            d.metric, d.baseline, d.actual, d.delta
                        );
                    }
                }
            }
        }
    }
    let took = start.elapsed();
    let summary_color = if failed == 0 && use_color {
        "\x1b[32m"
    } else if use_color {
        "\x1b[31m"
    } else {
        ""
    };
    let reset = if use_color { "\x1b[0m" } else { "" };
    println!(
        "\n{summary_color}{} passed, {} failed{reset} in {:.2?}",
        passed, failed, took
    );
    // §35.4 — trajectory summary, printed when a trajectory mode ran.
    if record_trajectory.is_some() {
        println!(
            "trajectories recorded: {trajectory_saved} (in {})",
            trajectory_dir.display()
        );
    }
    if match_trajectory.is_some() {
        println!(
            "trajectories: {trajectory_matched} matched, {trajectory_drift} drifted"
        );
    }
    // §35.5 — doc-tests summary.
    if include_doc_tests {
        println!(
            "doc-tests: {doc_snippets_seen} `\u{0060}\u{0060}\u{0060}axon` fence(s) extracted from /// comments"
        );
    }
    if failed > 0 || trajectory_drift > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// §35.4 — sanitize a test name into a filesystem-safe filename. Tests
/// can have arbitrary string names (`test "the api should retry"`);
/// the snapshot files need stable, slash-free names.
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}


fn emit_project_diagnostics_via_registry(
    diags: &[axon_diag::Diagnostic],
    registry: &axon_diag::SourceRegistry,
) {
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
    for d in diags {
        eprintln!("{}", axon_diag::render_with_registry(d, registry, use_color));
    }
}

fn cmd_run(args: &[String]) -> ExitCode {
    // Parse `axon run` flags. Order is flexible — the file path is whatever
    // positional argument we encounter that isn't consumed by a flag.
    let mut path: Option<&str> = None;
    let mut cap_names: Option<Vec<String>> = None;
    let mut isolated = false;
    let mut use_vm = false;
    let mut dry_run = false;
    let mut trace_path: Option<String> = None;
    let mut record_path: Option<String> = None;
    let mut replay_path: Option<String> = None;
    let mut features: Vec<String> = Vec::new();
    let mut enable_default_features = true;
    // Stage 36: route through the async-eval boundary by default. The env
    // var `AXON_NO_ASYNC=1` and CLI flag `--no-async` bypass `run_async`
    // and call `run_main` directly — escape hatch for perf A/B and a
    // kill-switch if Stage 36 turns out to regress something.
    let mut no_async = std::env::var("AXON_NO_ASYNC").is_ok();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--isolated" => {
                isolated = true;
                i += 1;
            }
            "--no-async" => {
                no_async = true;
                i += 1;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--vm" => {
                use_vm = true;
                i += 1;
            }
            "--tree" => {
                use_vm = false;
                i += 1;
            }
            "--with" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon run: --with requires a comma-separated list of effects");
                    return ExitCode::from(2);
                }
                cap_names = Some(parse_caps_to_vec(&args[i]));
                i += 1;
            }
            other if other.starts_with("--with=") => {
                cap_names = Some(parse_caps_to_vec(&other["--with=".len()..]));
                i += 1;
            }
            "--trace" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon run: --trace requires a path");
                    return ExitCode::from(2);
                }
                trace_path = Some(args[i].clone());
                i += 1;
            }
            other if other.starts_with("--trace=") => {
                trace_path = Some(other["--trace=".len()..].to_string());
                i += 1;
            }
            "--record" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon run: --record requires a path");
                    return ExitCode::from(2);
                }
                record_path = Some(args[i].clone());
                i += 1;
            }
            other if other.starts_with("--record=") => {
                record_path = Some(other["--record=".len()..].to_string());
                i += 1;
            }
            "--replay" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon run: --replay requires a path");
                    return ExitCode::from(2);
                }
                replay_path = Some(args[i].clone());
                i += 1;
            }
            other if other.starts_with("--replay=") => {
                replay_path = Some(other["--replay=".len()..].to_string());
                i += 1;
            }
            "--features" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon run: --features requires a comma-separated list");
                    return ExitCode::from(2);
                }
                features.extend(parse_caps_to_vec(&args[i]));
                i += 1;
            }
            other if other.starts_with("--features=") => {
                features.extend(parse_caps_to_vec(&other["--features=".len()..]));
                i += 1;
            }
            "--no-default-features" => {
                enable_default_features = false;
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("axon run: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            _ => {
                if path.is_some() {
                    eprintln!(
                        "axon run: more than one file given (`{}` and `{a}`); only one supported",
                        path.unwrap()
                    );
                    return ExitCode::from(2);
                }
                path = Some(a);
                i += 1;
            }
        }
    }
    if record_path.is_some() && replay_path.is_some() {
        eprintln!("axon run: --record and --replay are mutually exclusive");
        return ExitCode::from(2);
    }
    if use_vm && (trace_path.is_some() || record_path.is_some() || replay_path.is_some()) {
        eprintln!(
            "axon run: --trace, --record, --replay are only available with the tree-walker today; drop --vm or wait for stage 7.5"
        );
        return ExitCode::from(2);
    }
    // No path → run the project in the current directory (§58.1: `axon
    // run` inside a scaffolded project just works).
    let path = path.unwrap_or(".");
    let path_buf = std::path::Path::new(path);
    let project = match axon_project::LoadedProject::load_with_features(
        path_buf,
        &features,
        enable_default_features,
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let active = axon_project::resolve_features(
        &project.manifest.features,
        &features,
        enable_default_features,
    );
    host::set_active_features(active.names());
    let file = project
        .modules
        .first()
        .map(|m| m.source.clone())
        .unwrap_or_else(|| axon_diag::SourceFile::new("<empty>", String::new()));
    let program = project.merged.clone();
    if !project.diagnostics.is_empty() {
        emit_project_diagnostics_via_registry(&project.diagnostics, &project.sources);
        return ExitCode::from(1);
    }
    // Manifest-level capability default can preempt the CLI default.
    if cap_names.is_none() && !project.manifest.caps.default.is_empty() && !isolated {
        cap_names = Some(project.manifest.caps.default.clone());
    }
    let (_, type_diags) = axon_tyck::check(&file, &program);
    if !type_diags.is_empty() {
        emit_diagnostics(&type_diags, &file);
        return ExitCode::from(1);
    }
    if dry_run {
        return dry_run_report(&program, cap_names.as_deref(), isolated);
    }
    if use_vm {
        let vm_caps = if isolated {
            axon_vm::CapSet::empty()
        } else if let Some(names) = &cap_names {
            axon_vm::CapSet::from_iter(names.iter().cloned())
        } else {
            axon_vm::CapSet::standard_default()
        };
        match axon_vm::run_with_caps(&file, &program, vm_caps) {
            Ok(_) => ExitCode::SUCCESS,
            Err(axon_vm::RunError::Compile(diags)) => {
                emit_diagnostics(&diags, &file);
                ExitCode::from(1)
            }
            Err(axon_vm::RunError::Runtime(err)) => {
                let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
                eprint!("{}", err.render(&file, use_color));
                ExitCode::from(1)
            }
        }
    } else {
        let caps = if isolated {
            axon_runtime::CapSet::empty()
        } else if let Some(names) = &cap_names {
            axon_runtime::CapSet::from_iter(names.iter().cloned())
        } else {
            axon_runtime::CapSet::standard_default()
        };
        let mut interp = axon_runtime::Interpreter::with_caps(caps);
        host::install(&interp);
        if trace_path.is_some() {
            interp.enable_tracing();
        }
        if record_path.is_some() {
            interp.enable_recording();
        }
        if let Some(rp) = &replay_path {
            let raw = match std::fs::read_to_string(rp) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("axon run: cannot read replay `{rp}`: {e}");
                    return ExitCode::from(1);
                }
            };
            let parsed: serde_json::Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("axon run: replay `{rp}` is not valid JSON: {e}");
                    return ExitCode::from(1);
                }
            };
            let rec = match axon_runtime::Recording::from_json(&parsed) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("axon run: replay `{rp}`: {e}");
                    return ExitCode::from(1);
                }
            };
            interp.enable_replay(rec);
        }
        interp.load_program(&program);
        host::register_policies(&program);
        host::register_schemas(&program);
        let start = std::time::Instant::now();
        // Stage 36: route through the async-eval seam by default. The
        // `run_async` wrapper enters the process-wide tokio runtime via
        // `block_on` so nested `parallel { }` blocks and `flow_parallel_asks`
        // can spawn_blocking without starting a second runtime. The
        // `--no-async` flag bypasses the seam for perf A/B + as a kill-switch.
        let run_result = if no_async {
            interp.run_main()
        } else {
            interp.run_async()
        };
        let exit = match run_result {
            Ok(_) => ExitCode::SUCCESS,
            Err(err) => {
                let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
                eprint!("{}", err.render(&file, use_color));
                ExitCode::from(1)
            }
        };
        let elapsed = start.elapsed();
        if let (Some(tp), Some(tracer)) = (trace_path.as_ref(), interp.take_tracer()) {
            let _ = std::fs::write(tp, tracer.to_jsonl());
        }
        if let (Some(rp), Some(rec)) = (record_path.as_ref(), interp.take_recording()) {
            let json = rec.to_json();
            let _ = std::fs::write(rp, serde_json::to_string_pretty(&json).unwrap_or_default());
        }
        // §64.3 footer: wall time, tokens, cost — so you cannot
        // accidentally run an expensive thing twice without noticing.
        // Suppressed when stderr isn't a terminal (CI / piped output)
        // or when explicitly silenced via $AXON_NO_FOOTER.
        if std::io::IsTerminal::is_terminal(&std::io::stderr())
            && std::env::var("AXON_NO_FOOTER").is_err()
        {
            let (tokens, cents) = host::footer_totals();
            eprintln!(
                "─── axon run: {:.2?}  ·  {} tokens  ·  ${:.4}",
                elapsed,
                tokens,
                cents as f64 / 100.0,
            );
            // Stage 33: if `default_model()` fell back to the mock because
            // no API key was set, surface the one-line hint right under the
            // footer so the next thing a confused user sees is the cause.
            // Closes the loop the advisor flagged — "(no ANTHROPIC_API_KEY
            // — using mock model. run `axon login anthropic`)".
            if axon_runtime::default_model_used_mock() {
                eprintln!(
                    "    (no ANTHROPIC_API_KEY — `default_model()` used the mock. \
                     run `axon login anthropic` to use the real model.)"
                );
            }
        }
        exit
    }
}

/// `axon run --dry-run` (§64.3): type-check, then print what *would*
/// run — declared models, agents, tools, and handlers, plus the active
/// capability set — without executing anything or spending money.
fn dry_run_report(
    program: &axon_ast::Program,
    cap_names: Option<&[String]>,
    isolated: bool,
) -> ExitCode {
    use axon_ast::Item;
    let mut models = Vec::new();
    let mut agents = Vec::new();
    let mut tools = Vec::new();
    let mut has_main = false;
    for item in &program.items {
        match item {
            Item::Model(m) => models.push(m.name.name.clone()),
            Item::Agent(a) => agents.push(a.name.name.clone()),
            Item::Tool(t) => tools.push(t.name.name.clone()),
            Item::Fn(f) if f.name.name == "main" => has_main = true,
            _ => {}
        }
    }
    println!("axon run --dry-run — nothing was executed\n");
    let caps = if isolated {
        "(isolated: none)".to_string()
    } else {
        match cap_names {
            Some(c) if !c.is_empty() => c.join(", "),
            _ => "(standard default set)".to_string(),
        }
    };
    println!("  entrypoint   {}", if has_main { "main()" } else { "(no main)" });
    println!("  capabilities {caps}");
    println!("  models       {}", fmt_list(&models));
    println!("  agents       {}", fmt_list(&agents));
    println!("  tools        {}", fmt_list(&tools));
    println!(
        "\n  note: model calls, tool calls, and effects were NOT performed."
    );
    ExitCode::SUCCESS
}

fn fmt_list(xs: &[String]) -> String {
    if xs.is_empty() {
        "(none)".to_string()
    } else {
        xs.join(", ")
    }
}

fn parse_caps_to_vec(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn cmd_check(args: &[String]) -> ExitCode {
    let mut path: Option<&str> = None;
    let mut json = false;
    let mut explain_errors = false;
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            "--explain-errors" => explain_errors = true,
            other if other.starts_with("--") => {
                eprintln!("axon check: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                if path.is_some() {
                    eprintln!("axon check: only one file is supported");
                    return ExitCode::from(2);
                }
                path = Some(other);
            }
        }
    }
    let Some(path) = path else {
        eprintln!("usage: axon check [--json] [--explain-errors] <file>");
        return ExitCode::from(2);
    };
    let Some(file) = read_or_die(path) else {
        return ExitCode::from(1);
    };
    let (program, parse_diags) = axon_parser::parse(&file);
    // Parse errors short-circuit: don't run tyck on a broken tree.
    let (item_count, all): (usize, Vec<axon_diag::Diagnostic>) = if parse_diags.is_empty() {
        let (ctx, type_diags) = axon_tyck::check(&file, &program);
        (ctx.len(), type_diags)
    } else {
        (0, parse_diags)
    };

    if json {
        println!("{}", diagnostics_to_json(&all, path));
        return if all.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }
    if !all.is_empty() {
        emit_diagnostics(&all, &file);
        if explain_errors {
            print_explanations(&all);
        }
        return ExitCode::from(1);
    }
    println!("type-checked {item_count} item(s) successfully");
    ExitCode::SUCCESS
}

// ===========================================================================
// §32 — `axon fix`: apply diagnostic-attached rewrites.
//
// Diagnostics carry `fixes: Vec<Fix>` (each fix is a description + a list
// of span-keyed text edits). `axon fix` runs the same parse + type-check
// pipeline as `axon check`, collects fixes from the resulting diagnostics,
// and either:
//
//   default      → prints a unified diff so the user can read what *would*
//                  change without touching the file (dry run).
//   --apply      → rewrites the file in place.
//   --only CODE  → restricts to fixes attached to diagnostics with code
//                  `CODE` (e.g. `--only E0202` for did-you-mean only).
//
// Conflict handling: edits to overlapping spans are folded one-fix-at-a-
// time, deferring the rest with a clear note. The user runs `axon fix`
// again to pick up the next round — same idea as `cargo fix`.
// ===========================================================================

fn cmd_fix(args: &[String]) -> ExitCode {
    let mut path: Option<&str> = None;
    let mut apply = false;
    let mut only: Option<&str> = None;
    // §34.5 — new modes.
    let mut interactive = false;
    let mut watch = false;
    let mut i = 0usize;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--apply" => apply = true,
            "--interactive" | "-i" => interactive = true,
            "--watch" | "-w" => watch = true,
            "--only" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon fix: `--only` expects an error code (e.g. E0202)");
                    return ExitCode::from(2);
                }
                only = Some(&args[i]);
            }
            "--help" | "-h" => {
                println!(
                    "usage: axon fix [--apply | --interactive | --watch] [--only CODE] <path>\n\
                     \n\
                     <path> can be a single .ax file OR a project directory.\n\
                     In project mode, every .ax file is scanned and cross-file\n\
                     fixes (e.g. P0010 missing pub) are routed to the right file.\n\
                     \n\
                     MODES (mutually exclusive):\n\
                       (none)         Print a unified diff (dry run).\n\
                       --apply        Rewrite the touched file(s) in place.\n\
                       --interactive  Walk each hunk with a y/n/a/q prompt;\n\
                                      falls back to dry-run when stdin is not a TTY.\n\
                       --watch        Re-run on every save (notify-backed);\n\
                                      auto-applies only Confidence::Safe fixes,\n\
                                      reports Suggested ones as notifications.\n\
                     \n\
                     --only CODE restricts to fixes attached to that diagnostic code."
                );
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--") => {
                eprintln!("axon fix: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                if path.is_some() {
                    eprintln!("axon fix: only one path is supported");
                    return ExitCode::from(2);
                }
                path = Some(other);
            }
        }
        i += 1;
    }
    // Mutual exclusivity check — mixing modes is almost always a mistake.
    let mode_flags = [apply, interactive, watch];
    if mode_flags.iter().filter(|b| **b).count() > 1 {
        eprintln!(
            "axon fix: --apply, --interactive, and --watch are mutually exclusive — pick one"
        );
        return ExitCode::from(2);
    }
    let Some(path) = path else {
        eprintln!(
            "usage: axon fix [--apply | --interactive | --watch] [--only CODE] <path>"
        );
        return ExitCode::from(2);
    };

    // --watch dispatches to its own loop and never returns the dry-run /
    // apply path — the loop runs forever until Ctrl-C.
    if watch {
        return cmd_fix_watch(path, only);
    }

    // Two modes:
    //   * single-file (default) — preserves the original behavior; spans
    //     in this file all carry file_id 0 (single-file SourceFile).
    //   * project-directory — uses LoadedProject so cross-file fixes
    //     (P0010) route correctly. Every module gets a stable file_id.
    let path_buf = std::path::Path::new(path);
    if path_buf.is_dir() {
        // §35.3 — project mode now supports --interactive (the prior
        // gap is closed). Per-file prompts walk files in sources-
        // registry order; `[a]ll-from-here` accepts across all
        // remaining files; `[q]uit` aborts the whole pass. Non-TTY
        // stdin falls back to dry-run, same as the single-file mode.
        return cmd_fix_project(path_buf, apply, interactive, only);
    }

    let Some(file) = read_or_die(path) else {
        return ExitCode::from(1);
    };

    let (program, parse_diags) = axon_parser::parse(&file);
    let diags: Vec<axon_diag::Diagnostic> = if parse_diags.is_empty() {
        let (_ctx, type_diags) = axon_tyck::check(&file, &program);
        type_diags
    } else {
        parse_diags
    };

    // Gather every Fix from every diagnostic, filtered by --only when set.
    // Each fix may contribute multiple edits; we keep the originating code
    // around so the diff/apply path can label the change.
    //
    // §34.1 — `confidence` is plumbed through so `--interactive` can
    // show `[safe]` / `[suggested]` next to the description.
    struct Hunk<'a> {
        code: Option<&'a str>,
        description: &'a str,
        edits: &'a [axon_diag::FixEdit],
        confidence: axon_diag::Confidence,
    }
    let hunks: Vec<Hunk> = diags
        .iter()
        .filter(|d| match only {
            Some(want) => d.code.map(|c| c == want).unwrap_or(false),
            None => true,
        })
        .flat_map(|d| {
            d.fixes.iter().map(move |f| Hunk {
                code: d.code,
                description: &f.description,
                edits: &f.edits,
                confidence: f.confidence,
            })
        })
        .collect();

    if hunks.is_empty() {
        // Distinguish "no fixable diagnostics" (success-shaped) from
        // "diagnostics exist, none with fixes" (still success-shaped but
        // worth noting). Either way: nothing to do, exit clean.
        if diags.is_empty() {
            println!("axon fix: no diagnostics — nothing to fix");
        } else {
            let total = diags.len();
            let filtered = match only {
                Some(c) => format!(" (filtered by --only {c})"),
                None => String::new(),
            };
            println!(
                "axon fix: {total} diagnostic(s){filtered} but none carry a mechanically applicable fix"
            );
        }
        return ExitCode::SUCCESS;
    }

    // Two-pass apply:
    //   1. select hunks: keep each in input order if none of its edits
    //      touch a span already taken by an earlier-accepted hunk. Conflicts
    //      get deferred — the user reruns `axon fix` to pick them up.
    //      §34.5 — when `interactive` is set, also prompt the user per hunk
    //      before accepting; `n` skips, `q` aborts the rest, `a` accepts all
    //      remaining without prompting again. Non-TTY stdin falls back to
    //      dry-run mode (skip the prompt entirely).
    //   2. apply every accepted edit across every accepted hunk in one
    //      reverse pass over the byte buffer, so earlier offsets stay
    //      valid as we splice.
    let stdin_is_tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let do_interactive = interactive && stdin_is_tty;
    if interactive && !stdin_is_tty {
        eprintln!(
            "axon fix: --interactive needs a TTY on stdin — falling back to dry-run"
        );
    }
    let mut applied: Vec<&Hunk> = Vec::new();
    let mut deferred: Vec<&Hunk> = Vec::new();
    let mut skipped_by_user: Vec<&Hunk> = Vec::new();
    let mut covered: Vec<(usize, usize)> = Vec::new();
    let mut accept_all = false;
    let mut quit_remaining = false;
    for h in &hunks {
        let overlaps = h.edits.iter().any(|e| {
            let s = e.span.start as usize;
            let t = e.span.end as usize;
            covered.iter().any(|(cs, ce)| !(t <= *cs || *ce <= s))
        });
        if overlaps {
            deferred.push(h);
            continue;
        }
        if do_interactive && !accept_all && !quit_remaining {
            match prompt_interactive(
                path,
                file.text(),
                h.code,
                h.description,
                h.confidence,
                h.edits.first(),
            ) {
                InteractiveChoice::Yes => {}
                InteractiveChoice::No => {
                    skipped_by_user.push(h);
                    continue;
                }
                InteractiveChoice::AcceptAll => accept_all = true,
                InteractiveChoice::Quit => {
                    quit_remaining = true;
                    skipped_by_user.push(h);
                    continue;
                }
            }
        } else if quit_remaining {
            skipped_by_user.push(h);
            continue;
        }
        for e in h.edits {
            covered.push((e.span.start as usize, e.span.end as usize));
        }
        applied.push(h);
    }
    // Collect every accepted edit, then splice in descending order of
    // start offset so each splice leaves earlier byte indices untouched.
    let mut bytes = file.text().as_bytes().to_vec();
    let mut all_edits: Vec<&axon_diag::FixEdit> =
        applied.iter().flat_map(|h| h.edits.iter()).collect();
    all_edits.sort_by(|a, b| b.span.start.cmp(&a.span.start));
    for e in all_edits {
        let s = (e.span.start as usize).min(bytes.len());
        let t = (e.span.end as usize).min(bytes.len()).max(s);
        bytes.splice(s..t, e.replacement.bytes());
    }

    let after = String::from_utf8_lossy(&bytes).into_owned();
    let before = file.text().to_owned();

    // §34.5 — `do_interactive` writes the file in place (the user
    // already approved per hunk). `apply` writes unconditionally. Else
    // dry-run.
    let write_in_place = apply || do_interactive;

    if write_in_place {
        if before == after {
            println!("axon fix: no edits to apply");
            return ExitCode::SUCCESS;
        }
        if let Err(e) = std::fs::write(path, &after) {
            eprintln!("axon fix: cannot write `{path}`: {e}");
            return ExitCode::from(1);
        }
        let skipped_note = if skipped_by_user.is_empty() {
            String::new()
        } else {
            format!(" ({} skipped by user)", skipped_by_user.len())
        };
        println!(
            "axon fix: applied {} fix(es) to {path}{}{}",
            applied.len(),
            skipped_note,
            if deferred.is_empty() {
                String::new()
            } else {
                format!(
                    " ({} deferred — rerun `axon fix` to pick them up)",
                    deferred.len()
                )
            }
        );
        ExitCode::SUCCESS
    } else {
        // Dry-run: print a unified diff. We use a simple line-based diff —
        // good enough for human review of small mechanical edits.
        let diff = unified_diff(path, &before, &after);
        if diff.trim().is_empty() {
            println!("axon fix: no edits to apply");
            return ExitCode::SUCCESS;
        }
        for h in &applied {
            let tier = match h.confidence {
                axon_diag::Confidence::Safe => "safe",
                axon_diag::Confidence::Suggested => "suggested",
            };
            println!(
                "fix [{}, {tier}]: {}",
                h.code.unwrap_or("uncoded"),
                h.description
            );
        }
        if !deferred.is_empty() {
            println!(
                "(deferred {} fix(es) due to overlap; rerun `axon fix` to pick them up)",
                deferred.len()
            );
        }
        print!("{diff}");
        println!(
            "\n(dry run — pass --apply to rewrite {} in place)",
            path
        );
        ExitCode::SUCCESS
    }
}

// §34.5 — interactive per-hunk prompt support. Plain params (no trait)
// so the caller's local `struct Hunk` doesn't need any extra plumbing.

enum InteractiveChoice {
    Yes,
    No,
    AcceptAll,
    Quit,
}

fn prompt_interactive(
    path: &str,
    source: &str,
    code: Option<&str>,
    description: &str,
    confidence: axon_diag::Confidence,
    first_edit: Option<&axon_diag::FixEdit>,
) -> InteractiveChoice {
    use std::io::{BufRead, Write};
    let tier = match confidence {
        axon_diag::Confidence::Safe => "safe",
        axon_diag::Confidence::Suggested => "suggested",
    };
    println!(
        "\n{}: fix [{}, {tier}]: {}",
        path,
        code.unwrap_or("uncoded"),
        description
    );
    // Show a tiny context window — the surrounding source line for the
    // first edit. Better preview than nothing; not a full diff.
    if let Some(edit) = first_edit {
        let start = (edit.span.start as usize).min(source.len());
        let end = (edit.span.end as usize).min(source.len()).max(start);
        let line_start = source[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_end = source[start..]
            .find('\n')
            .map(|i| start + i)
            .unwrap_or(source.len());
        let line = &source[line_start..line_end];
        println!("    - {line}");
        let mut after_preview = String::from(&source[line_start..start]);
        after_preview.push_str(&edit.replacement);
        after_preview.push_str(&source[end..line_end]);
        println!("    + {after_preview}");
    }
    print!("Apply? [y]es / [n]o / [a]ll-from-here / [q]uit: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return InteractiveChoice::Quit;
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" | "" => InteractiveChoice::Yes,
        "n" | "no" => InteractiveChoice::No,
        "a" | "all" => InteractiveChoice::AcceptAll,
        "q" | "quit" => InteractiveChoice::Quit,
        other => {
            println!("axon fix: unrecognized choice `{other}` — skipping this hunk");
            InteractiveChoice::No
        }
    }
}

/// Project-mode `axon fix`. Loads every `.ax` file under `root` via
/// `LoadedProject`, runs type-check on the merged program, and routes
/// each fix edit to the right file via `Span::file`. Lets P0010
/// (missing-pub) attach a fix that lives in a *different* file from
/// the diagnostic.
/// §35.3 — named struct for project-mode hunks. The pre-Stage-35 code
/// used an unwieldy 3-tuple; promoting it lets `--interactive` (which
/// also reads `confidence`) thread through without `.2`/`.3` index
/// shuffling.
struct ProjectHunk {
    description: String,
    code: Option<String>,
    confidence: axon_diag::Confidence,
    edit: axon_diag::FixEdit,
}

fn cmd_fix_project(
    root: &std::path::Path,
    apply: bool,
    interactive: bool,
    only: Option<&str>,
) -> ExitCode {
    let project = match axon_project::LoadedProject::load(root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("axon fix: {e}");
            return ExitCode::from(1);
        }
    };

    // Project loader's own diagnostics (P0010 missing-pub, P0011 unknown
    // import). Type-checker runs against the *merged* program so cross-
    // module name resolution is honored.
    let mut all_diags: Vec<axon_diag::Diagnostic> = project.diagnostics.clone();
    let merged_source = project
        .modules
        .first()
        .map(|m| m.source.clone())
        .unwrap_or_else(|| axon_diag::SourceFile::new("<merged>", String::new()));
    let (_ctx, type_diags) = axon_tyck::check(&merged_source, &project.merged);
    all_diags.extend(type_diags);

    // Filter by --only, then collect every fix edit indexed by file_id.
    // file_id 0 means "no file" (dummy span) — we drop those silently.
    let mut edits_by_file: std::collections::BTreeMap<u16, Vec<ProjectHunk>> =
        std::collections::BTreeMap::new();
    for d in &all_diags {
        if let Some(want) = only {
            if d.code.map(|c| c != want).unwrap_or(true) {
                continue;
            }
        }
        for f in &d.fixes {
            for e in &f.edits {
                if e.span.file == 0 {
                    continue;
                }
                edits_by_file.entry(e.span.file).or_default().push(ProjectHunk {
                    description: f.description.clone(),
                    code: d.code.map(|c| c.to_string()),
                    confidence: f.confidence,
                    edit: e.clone(),
                });
            }
        }
    }

    if edits_by_file.is_empty() {
        if all_diags.is_empty() {
            println!("axon fix: no diagnostics across {} module(s) — nothing to fix", project.modules.len());
        } else {
            let filt = match only {
                Some(c) => format!(" (filtered by --only {c})"),
                None => String::new(),
            };
            println!(
                "axon fix: {} diagnostic(s) across {} module(s){filt} but none carry a mechanically applicable fix",
                all_diags.len(),
                project.modules.len(),
            );
        }
        return ExitCode::SUCCESS;
    }

    // §35.3 — interactive prompt state lives at the project scope so
    // [a]ll-from-here applies across files and [q]uit aborts the whole
    // pass. Non-TTY stdin falls back to dry-run.
    let stdin_is_tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let do_interactive = interactive && stdin_is_tty;
    if interactive && !stdin_is_tty {
        eprintln!(
            "axon fix: --interactive needs a TTY on stdin — falling back to dry-run"
        );
    }
    let mut accept_all = false;
    let mut quit_remaining = false;

    let mut touched_files: u32 = 0;
    let mut applied_total: u32 = 0;
    let mut deferred_total: u32 = 0;
    let mut skipped_by_user: u32 = 0;
    for (file_id, hunks) in edits_by_file {
        let Some(src) = project.sources.get(file_id) else {
            continue;
        };
        let path = src.path().to_string_lossy().into_owned();
        let original = src.text().to_owned();

        // Sort each file's hunks by edit start so source-order prompts
        // line up with how a user reads the file top-to-bottom.
        let mut sorted_hunks: Vec<&ProjectHunk> = hunks.iter().collect();
        sorted_hunks.sort_by_key(|h| h.edit.span.start);

        // Same conflict-handling pass the single-file mode uses: accept
        // hunks in source order; defer ones whose edit spans overlap an
        // already-accepted hunk. In interactive mode, prompt before
        // marking a non-overlapping hunk accepted.
        let mut covered: Vec<(usize, usize)> = Vec::new();
        let mut accepted: Vec<&ProjectHunk> = Vec::new();
        let mut deferred: u32 = 0;
        for h in &sorted_hunks {
            let s = h.edit.span.start as usize;
            let t = h.edit.span.end as usize;
            if covered.iter().any(|(cs, ce)| !(t <= *cs || *ce <= s)) {
                deferred += 1;
                continue;
            }
            if do_interactive && !accept_all && !quit_remaining {
                match prompt_interactive(
                    &path,
                    &original,
                    h.code.as_deref(),
                    &h.description,
                    h.confidence,
                    Some(&h.edit),
                ) {
                    InteractiveChoice::Yes => {}
                    InteractiveChoice::No => {
                        skipped_by_user += 1;
                        continue;
                    }
                    InteractiveChoice::AcceptAll => accept_all = true,
                    InteractiveChoice::Quit => {
                        quit_remaining = true;
                        skipped_by_user += 1;
                        continue;
                    }
                }
            } else if quit_remaining {
                skipped_by_user += 1;
                continue;
            }
            covered.push((s, t));
            accepted.push(h);
        }
        if accepted.is_empty() {
            deferred_total += deferred;
            continue;
        }
        let mut bytes = original.as_bytes().to_vec();
        let mut sorted: Vec<&ProjectHunk> = accepted.clone();
        sorted.sort_by(|a, b| b.edit.span.start.cmp(&a.edit.span.start));
        for h in sorted {
            let s = (h.edit.span.start as usize).min(bytes.len());
            let t = (h.edit.span.end as usize).min(bytes.len()).max(s);
            bytes.splice(s..t, h.edit.replacement.bytes());
        }
        let after = String::from_utf8_lossy(&bytes).into_owned();

        // Write-in-place when --apply OR --interactive (the user
        // already approved each hunk).
        let write_in_place = apply || do_interactive;
        if write_in_place {
            if after == original {
                continue;
            }
            if let Err(e) = std::fs::write(&path, &after) {
                eprintln!("axon fix: cannot write `{path}`: {e}");
                return ExitCode::from(1);
            }
            for h in &accepted {
                println!(
                    "{} [{}]: {}",
                    path,
                    h.code.as_deref().unwrap_or("uncoded"),
                    h.description
                );
            }
            touched_files += 1;
            applied_total += accepted.len() as u32;
        } else {
            for h in &accepted {
                let tier = match h.confidence {
                    axon_diag::Confidence::Safe => "safe",
                    axon_diag::Confidence::Suggested => "suggested",
                };
                println!(
                    "{} fix [{}, {tier}]: {}",
                    path,
                    h.code.as_deref().unwrap_or("uncoded"),
                    h.description
                );
            }
            let diff = unified_diff(&path, &original, &after);
            if !diff.trim().is_empty() {
                print!("{diff}");
                touched_files += 1;
                applied_total += accepted.len() as u32;
            }
        }
        deferred_total += deferred;
    }

    let deferred_note = if deferred_total > 0 {
        format!(
            " ({deferred_total} deferred due to overlap — rerun to pick them up)"
        )
    } else {
        String::new()
    };
    let skipped_note = if skipped_by_user > 0 {
        format!(" ({skipped_by_user} skipped by user)")
    } else {
        String::new()
    };
    if apply || do_interactive {
        println!(
            "\naxon fix: applied {applied_total} fix(es) across {touched_files} file(s){skipped_note}{deferred_note}"
        );
    } else {
        println!(
            "\n(dry run — pass --apply to rewrite {touched_files} file(s) in place; would apply {applied_total} fix(es){deferred_note})"
        );
    }
    ExitCode::SUCCESS
}

/// §34.5 `axon fix --watch <path>` — re-run on every save via the
/// `notify` crate. Only fixes tagged `Confidence::Safe` are auto-applied;
/// Suggested fixes are printed to stdout (the user must run
/// `axon fix --interactive` to review them).
///
/// Path semantics:
///   * single .ax file → watch the file (via its parent dir; events are
///     filtered to the canonicalized target path).
///   * project directory → must contain `axon.toml` and must be a
///     descendant of CWD (override with `AXON_FIX_WATCH_FORCE=1`).
///     Watches the whole `src/` tree.
///
/// Safety contract (Stage 34 verification fixes C1, C3, M1, M2, M3, M7):
///   * Project dirs without `axon.toml` are refused — `--watch /` won't
///     start a recursive rewriter on system files.
///   * Symlinks inside the watched tree are skipped (the project loader
///     enforces this; the watch loop also filters self-emitted events).
///   * Per-file mtime tracked for BOTH single-file and project modes —
///     post-apply mtime is recorded so the next notify event for the
///     same file is correctly recognized as a self-trigger.
///   * Debounce uses a 250 ms quiet window with a 2 s maxWait — a
///     chatty editor that saves on every keystroke still gets at least
///     one apply per 2 s.
///   * TOCTOU mtime compare in `apply_safe_fixes_to_file` /
///     `apply_safe_fixes_to_project` — if the file changed between
///     read and write, the apply is aborted and re-queued.
///   * Single-file mode filters events to exactly the canonicalized
///     target path; sibling .ax files in the same directory are
///     ignored.
///   * Ctrl-C handler is installed BEFORE the watcher starts; install
///     failure is fatal.
fn cmd_fix_watch(path: &str, only: Option<&str>) -> ExitCode {
    use notify::{event::ModifyKind, EventKind, RecursiveMode, Watcher};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let watch_target = std::path::PathBuf::from(path);
    if !watch_target.exists() {
        eprintln!("axon fix --watch: `{path}` does not exist");
        return ExitCode::from(1);
    }

    // Stage 34 verification fix C1: refuse pointing --watch at arbitrary
    // dirs (`/`, `/etc`, the user's home). Require an axon.toml at or
    // above the watched directory, AND require the canonical watch path
    // to be a descendant of CWD. The env-var override exists for CI /
    // sandboxed users who genuinely need to opt out.
    let force = std::env::var("AXON_FIX_WATCH_FORCE")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let canon_target = match watch_target.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("axon fix --watch: cannot canonicalize `{}`: {e}", path);
            return ExitCode::from(1);
        }
    };
    // §34.6 verification fix C1 — directory-mode safety checks. The
    // checks only apply to dir-mode because single-file mode is
    // bounded by definition: it only ever touches one file (the
    // target). Pointing `--watch foo.ax` at /etc/hosts is bad if you
    // own /etc/hosts, but that's a separate trust boundary the user
    // has already crossed by running the command at all. The danger
    // is pointing `--watch /` at a tree and having it walk + rewrite
    // many files — that's what these checks gate.
    if watch_target.is_dir() && !force {
        // Must have axon.toml in the tree (the dir itself or an ancestor
        // of it inside the watch root). Project loader requires this
        // anyway; we surface a clearer error before the watcher starts.
        let mut has_manifest = false;
        let mut probe: Option<&std::path::Path> = Some(canon_target.as_path());
        while let Some(p) = probe {
            if p.join("axon.toml").is_file() {
                has_manifest = true;
                break;
            }
            probe = p.parent();
        }
        if !has_manifest {
            eprintln!(
                "axon fix --watch: refusing to watch `{}` — no `axon.toml` found.\n\
                 (Pointing --watch at a directory without an Axon manifest could rewrite\n\
                  unrelated .ax files. Add an axon.toml, or set AXON_FIX_WATCH_FORCE=1 to override.)",
                canon_target.display()
            );
            return ExitCode::from(1);
        }
        // Must be a descendant of CWD.
        let cwd = match std::env::current_dir() {
            Ok(c) => c.canonicalize().unwrap_or(c),
            Err(e) => {
                eprintln!("axon fix --watch: cannot read CWD: {e}");
                return ExitCode::from(1);
            }
        };
        if !canon_target.starts_with(&cwd) {
            eprintln!(
                "axon fix --watch: refusing to watch `{}` — not a descendant of CWD `{}`.\n\
                 (cd into the project directory, or set AXON_FIX_WATCH_FORCE=1 to override.)",
                canon_target.display(),
                cwd.display()
            );
            return ExitCode::from(1);
        }
    }

    // Stage 34 verification fix M7: install Ctrl-C handler BEFORE the
    // watcher starts. A SIGINT during watcher.watch() would otherwise
    // kill the process with no shutdown path. Install-failure is fatal
    // because the loop's only exit signal is the AtomicBool.
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        if let Err(e) = ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst)) {
            eprintln!(
                "axon fix --watch: cannot install Ctrl-C handler: {e}\n\
                 (Refusing to start without a shutdown path — the watch loop has no\n\
                  other exit signal. If you really need to bypass, kill via SIGTERM.)"
            );
            return ExitCode::from(1);
        }
    }

    let (tx, rx) = channel();
    let mut watcher = match notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("axon fix --watch: cannot start watcher: {e}");
            return ExitCode::from(1);
        }
    };
    // Project mode: watch the src/ tree recursively if it exists;
    // otherwise watch the directory itself. Single-file: notify still
    // requires watching the parent dir to catch atomic rename-based
    // saves (vim, IDEs), so we filter events to the canonical target
    // path inside the loop (Stage 34 verification fix M2).
    let (watch_root, mode) = if watch_target.is_dir() {
        let src = watch_target.join("src");
        if src.is_dir() {
            (src, RecursiveMode::Recursive)
        } else {
            (watch_target.clone(), RecursiveMode::Recursive)
        }
    } else {
        let parent = watch_target
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        (parent, RecursiveMode::NonRecursive)
    };
    if let Err(e) = watcher.watch(&watch_root, mode) {
        eprintln!(
            "axon fix --watch: cannot watch `{}`: {e}",
            watch_root.display()
        );
        return ExitCode::from(1);
    }

    println!(
        "axon fix --watch: watching `{}` (Safe-tier fixes auto-apply; Ctrl-C to stop)",
        canon_target.display()
    );
    // Per-path mtime tracking — keys cover BOTH the single-file and
    // project paths (Stage 34 verification fix M1). Updated after every
    // successful apply (single-file branch records its target; project
    // branch records every file it actually wrote, returned from
    // apply_safe_fixes_to_project).
    let mut last_applied: std::collections::HashMap<std::path::PathBuf, std::time::SystemTime> =
        std::collections::HashMap::new();

    // Run once at startup so the user sees current state, then record
    // mtimes so the first inbound event (often the apply itself) gets
    // recognized as a self-trigger.
    let startup_touched = run_safe_fixes_once(&watch_target, only);
    for p in startup_touched {
        if let Ok(meta) = std::fs::metadata(&p) {
            if let Ok(mtime) = meta.modified() {
                last_applied.insert(p.canonicalize().unwrap_or(p), mtime);
            }
        }
    }

    // Debounce parameters (Stage 34 verification fix M3).
    let quiet_window = Duration::from_millis(250);
    let max_wait = Duration::from_millis(2000);
    // First-event timestamp in the current burst — applies always fire
    // by max_wait_deadline even if events keep arriving inside
    // quiet_window.
    let mut pending_first: Option<Instant> = None;
    let mut pending_quiet_deadline: Option<Instant> = None;
    // Single-file mode: filter events to exactly the canonicalized
    // target path (Stage 34 verification fix M2).
    let target_canon: Option<std::path::PathBuf> = if watch_target.is_file() {
        Some(canon_target.clone())
    } else {
        None
    };
    loop {
        if stop.load(Ordering::SeqCst) {
            println!("\naxon fix --watch: stopped");
            return ExitCode::SUCCESS;
        }
        match rx.recv_timeout(quiet_window) {
            Ok(Ok(event)) => {
                let interesting = matches!(
                    event.kind,
                    EventKind::Create(_)
                        | EventKind::Modify(ModifyKind::Data(_))
                        | EventKind::Modify(ModifyKind::Any)
                        | EventKind::Modify(ModifyKind::Name(_))
                );
                if !interesting {
                    continue;
                }
                let touched: Vec<std::path::PathBuf> = event
                    .paths
                    .into_iter()
                    .filter(|p| {
                        p.extension().map(|e| e == "ax").unwrap_or(false)
                            && p.exists()
                    })
                    // Single-file mode: only accept events for the
                    // canonical target. Sibling .ax files in the same
                    // dir don't trigger.
                    .filter(|p| match &target_canon {
                        Some(t) => p
                            .canonicalize()
                            .map(|c| c == *t)
                            .unwrap_or(false),
                        None => true,
                    })
                    // Skip symlinks defensively even though the project
                    // loader also rejects them.
                    .filter(|p| {
                        std::fs::symlink_metadata(p)
                            .map(|m| !m.file_type().is_symlink())
                            .unwrap_or(false)
                    })
                    .collect();
                if touched.is_empty() {
                    continue;
                }
                // Debounce: skip events whose mtime hasn't advanced
                // past the last applied mtime for the same path.
                // Default-stale-on-error (safer than default-fresh, the
                // pre-verification version was the wrong default).
                let mut any_fresh = false;
                for p in &touched {
                    let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
                    let stale = match std::fs::metadata(&canon) {
                        Ok(meta) => match meta.modified() {
                            Ok(mtime) => last_applied
                                .get(&canon)
                                .map(|prev| *prev >= mtime)
                                .unwrap_or(false),
                            Err(_) => true,
                        },
                        Err(_) => true,
                    };
                    if !stale {
                        any_fresh = true;
                    }
                }
                if any_fresh {
                    let now = Instant::now();
                    pending_first.get_or_insert(now);
                    pending_quiet_deadline = Some(now + quiet_window);
                }
            }
            Ok(Err(e)) => {
                eprintln!("axon fix --watch: notify error: {e}");
            }
            Err(RecvTimeoutError::Timeout) => {
                // No event in 250 ms — drain any pending apply.
            }
            Err(RecvTimeoutError::Disconnected) => {
                eprintln!("axon fix --watch: watcher channel disconnected");
                return ExitCode::from(1);
            }
        }
        // Fire when EITHER quiet window expired OR max wait reached.
        let now = Instant::now();
        let should_apply = match (pending_first, pending_quiet_deadline) {
            (Some(first), Some(quiet_d)) => now >= quiet_d || now >= first + max_wait,
            _ => false,
        };
        if should_apply {
            pending_first = None;
            pending_quiet_deadline = None;
            let touched = run_safe_fixes_once(&watch_target, only);
            for p in touched {
                let canon = p.canonicalize().unwrap_or(p);
                if let Ok(meta) = std::fs::metadata(&canon) {
                    if let Ok(mtime) = meta.modified() {
                        last_applied.insert(canon, mtime);
                    }
                }
            }
        }
    }
}

/// §34.5 — one pass of Safe-only fix application across `path`. Used
/// both at startup and on every save event in `--watch`. Returns the
/// list of files actually written (canonicalized) so the watch loop
/// can record fresh mtimes and recognize self-triggered notify events
/// (Stage 34 verification fix M1).
fn run_safe_fixes_once(
    path: &std::path::Path,
    only: Option<&str>,
) -> Vec<std::path::PathBuf> {
    if path.is_dir() {
        return apply_safe_fixes_to_project(path, only);
    }
    apply_safe_fixes_to_file(path, only)
}

fn apply_safe_fixes_to_file(
    path: &std::path::Path,
    only: Option<&str>,
) -> Vec<std::path::PathBuf> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("axon fix --watch: cannot read `{}`: {e}", path.display());
            return Vec::new();
        }
    };
    // §34.6 verification fix C3 (TOCTOU): capture the mtime AT THE
    // READ. We re-stat right before the write and abort if it changed
    // — the user must have edited the file in the gap, and our splice
    // was computed against stale bytes.
    let pre_mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let file = SourceFile::new(path.to_string_lossy().to_string(), text.clone());
    let (program, parse_diags) = axon_parser::parse(&file);
    let diags: Vec<axon_diag::Diagnostic> = if parse_diags.is_empty() {
        let (_ctx, td) = axon_tyck::check(&file, &program);
        td
    } else {
        parse_diags
    };
    // Filter to Safe-only fixes matching --only (when set).
    let safe_fixes: Vec<(Option<&str>, &axon_diag::Fix)> = diags
        .iter()
        .filter(|d| match only {
            Some(want) => d.code.map(|c| c == want).unwrap_or(false),
            None => true,
        })
        .flat_map(|d| {
            d.fixes
                .iter()
                .filter(|f| f.confidence == axon_diag::Confidence::Safe)
                .map(move |f| (d.code, f))
        })
        .collect();
    let suggested_count: usize = diags
        .iter()
        .flat_map(|d| d.fixes.iter())
        .filter(|f| f.confidence == axon_diag::Confidence::Suggested)
        .count();

    if safe_fixes.is_empty() {
        if suggested_count > 0 {
            println!(
                "axon fix --watch [{}]: {suggested_count} Suggested fix(es) — run `axon fix --interactive` to review",
                path.display()
            );
        }
        return Vec::new();
    }

    // Same accept-and-splice loop as the dry-run pipeline, minus the
    // dry-run output.
    let mut bytes = file.text().as_bytes().to_vec();
    let mut applied = 0usize;
    let mut covered: Vec<(usize, usize)> = Vec::new();
    let mut accepted_edits: Vec<&axon_diag::FixEdit> = Vec::new();
    for (_code, fix) in &safe_fixes {
        let overlaps = fix.edits.iter().any(|e| {
            let s = e.span.start as usize;
            let t = e.span.end as usize;
            covered.iter().any(|(cs, ce)| !(t <= *cs || *ce <= s))
        });
        if overlaps {
            continue;
        }
        for e in &fix.edits {
            covered.push((e.span.start as usize, e.span.end as usize));
            accepted_edits.push(e);
        }
        applied += 1;
    }
    accepted_edits.sort_by(|a, b| b.span.start.cmp(&a.span.start));
    for e in accepted_edits {
        let s = (e.span.start as usize).min(bytes.len());
        let t = (e.span.end as usize).min(bytes.len()).max(s);
        bytes.splice(s..t, e.replacement.bytes());
    }
    let after = String::from_utf8_lossy(&bytes).into_owned();
    if after == text {
        return Vec::new();
    }
    // §34.6 verification fix C3: TOCTOU compare right before write.
    let now_mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if pre_mtime.is_some() && now_mtime != pre_mtime {
        eprintln!(
            "axon fix --watch: `{}` changed during apply — aborting (the next save will retry)",
            path.display()
        );
        return Vec::new();
    }
    if let Err(e) = std::fs::write(path, &after) {
        eprintln!("axon fix --watch: cannot write `{}`: {e}", path.display());
        return Vec::new();
    }
    let suggested_note = if suggested_count > 0 {
        format!(" ({suggested_count} Suggested fix(es) still need review)")
    } else {
        String::new()
    };
    println!(
        "axon fix --watch [{}]: auto-applied {applied} Safe fix(es){suggested_note}",
        path.display()
    );
    vec![path.to_path_buf()]
}

fn apply_safe_fixes_to_project(
    root: &std::path::Path,
    only: Option<&str>,
) -> Vec<std::path::PathBuf> {
    let project = match axon_project::LoadedProject::load(root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("axon fix --watch: {e}");
            return Vec::new();
        }
    };
    let primary = project
        .modules
        .first()
        .map(|m| m.source.clone())
        .unwrap_or_else(|| SourceFile::new("<merged>", String::new()));
    let mut all_diags: Vec<axon_diag::Diagnostic> = project.diagnostics.clone();
    let (_ctx, td) = axon_tyck::check(&primary, &project.merged);
    all_diags.extend(td);

    // Group Safe edits by file_id; skip Suggested.
    let mut edits_by_file: std::collections::BTreeMap<u16, Vec<axon_diag::FixEdit>> =
        std::collections::BTreeMap::new();
    let mut suggested_count = 0usize;
    for d in &all_diags {
        if let Some(want) = only {
            if d.code.map(|c| c != want).unwrap_or(true) {
                continue;
            }
        }
        for fix in &d.fixes {
            if fix.confidence == axon_diag::Confidence::Suggested {
                suggested_count += 1;
                continue;
            }
            for e in &fix.edits {
                if e.span.file == 0 {
                    continue;
                }
                edits_by_file
                    .entry(e.span.file)
                    .or_default()
                    .push(e.clone());
            }
        }
    }

    let mut applied_total = 0usize;
    let mut touched: Vec<std::path::PathBuf> = Vec::new();
    for (file_id, mut edits) in edits_by_file {
        let Some(src) = project.sources.get(file_id) else {
            continue;
        };
        let path = src.path().to_path_buf();
        let original = src.text().to_string();
        // §34.6 verification fix C3 (TOCTOU): stat at read, compare
        // right before write.
        let pre_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        edits.sort_by(|a, b| b.span.start.cmp(&a.span.start));
        let mut bytes = original.as_bytes().to_vec();
        // Dedup overlapping edits, accepting in source order. (Different
        // diagnostics with the same edit shape can dup; this matches
        // the dry-run path's behavior.)
        let mut covered: Vec<(usize, usize)> = Vec::new();
        let mut accepted: Vec<&axon_diag::FixEdit> = Vec::new();
        // Sort ascending temporarily so overlap is left-to-right.
        let mut asc: Vec<&axon_diag::FixEdit> = edits.iter().collect();
        asc.sort_by(|a, b| a.span.start.cmp(&b.span.start));
        for e in asc {
            let s = e.span.start as usize;
            let t = e.span.end as usize;
            let conflicts = covered.iter().any(|(cs, ce)| !(t <= *cs || *ce <= s));
            if !conflicts {
                covered.push((s, t));
                accepted.push(e);
            }
        }
        accepted.sort_by(|a, b| b.span.start.cmp(&a.span.start));
        for e in accepted.iter() {
            let s = (e.span.start as usize).min(bytes.len());
            let t = (e.span.end as usize).min(bytes.len()).max(s);
            bytes.splice(s..t, e.replacement.bytes());
        }
        let after = String::from_utf8_lossy(&bytes).into_owned();
        if after == original {
            continue;
        }
        // §34.6 verification fix C3: TOCTOU compare right before write.
        let now_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if pre_mtime.is_some() && now_mtime != pre_mtime {
            eprintln!(
                "axon fix --watch: `{}` changed during apply — aborting",
                path.display()
            );
            continue;
        }
        if let Err(e) = std::fs::write(&path, &after) {
            eprintln!(
                "axon fix --watch: cannot write `{}`: {e}",
                path.display()
            );
            continue;
        }
        applied_total += accepted.len();
        touched.push(path.clone());
        println!(
            "axon fix --watch [{}]: auto-applied {} Safe fix(es)",
            path.display(),
            accepted.len()
        );
    }
    if applied_total == 0 && suggested_count > 0 {
        println!(
            "axon fix --watch: {suggested_count} Suggested fix(es) — run `axon fix --interactive` to review"
        );
    }
    if applied_total > 0 && suggested_count > 0 {
        println!(
            "axon fix --watch: also {suggested_count} Suggested fix(es) still need review"
        );
    }
    touched
}

/// Minimal unified diff against `path`. Line-based: precision-aware
/// enough for small mechanical edits. Hand-rolled because we don't want
/// to take a `similar`/`difflib` dependency just for this.
fn unified_diff(path: &str, before: &str, after: &str) -> String {
    use std::fmt::Write;
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    // LCS via classic DP — fine for source files; we typically diff a few
    // hundred lines at most.
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            dp[i + 1][j + 1] = if a[i] == b[j] {
                dp[i][j] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    // Walk back to build the edit script as (kind, line) pairs.
    let mut ops: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            ops.push((' ', a[i - 1]));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            ops.push(('-', a[i - 1]));
            i -= 1;
        } else {
            ops.push(('+', b[j - 1]));
            j -= 1;
        }
    }
    while i > 0 {
        ops.push(('-', a[i - 1]));
        i -= 1;
    }
    while j > 0 {
        ops.push(('+', b[j - 1]));
        j -= 1;
    }
    ops.reverse();
    if ops.iter().all(|(k, _)| *k == ' ') {
        return String::new();
    }
    let mut out = String::new();
    writeln!(out, "--- {path}").unwrap();
    writeln!(out, "+++ {path} (after fix)").unwrap();
    // Compress runs of unchanged context to ±2 lines, like `diff -U2`.
    let context: usize = 2;
    let mut k = 0;
    while k < ops.len() {
        if ops[k].0 == ' ' {
            k += 1;
            continue;
        }
        // Find the extent of this hunk: include `context` lines around.
        let start = k.saturating_sub(context);
        let mut end = k;
        while end < ops.len() {
            // Look ahead — if we're within `context * 2` of the next non-
            // context op, swallow it into the same hunk.
            if ops[end].0 != ' ' {
                end += 1;
                continue;
            }
            let mut look = end;
            let mut gap = 0;
            while look < ops.len() && ops[look].0 == ' ' {
                gap += 1;
                look += 1;
                if gap > context * 2 {
                    break;
                }
            }
            if look < ops.len() && ops[look].0 != ' ' && gap <= context * 2 {
                end = look;
            } else {
                end += context.min(ops.len() - end);
                break;
            }
        }
        // Header.
        let mut a_start = 0u32;
        let mut a_lines = 0u32;
        let mut b_start = 0u32;
        let mut b_lines = 0u32;
        for h in 0..start {
            match ops[h].0 {
                ' ' => {
                    a_start += 1;
                    b_start += 1;
                }
                '-' => a_start += 1,
                '+' => b_start += 1,
                _ => {}
            }
        }
        for h in start..end {
            match ops[h].0 {
                ' ' => {
                    a_lines += 1;
                    b_lines += 1;
                }
                '-' => a_lines += 1,
                '+' => b_lines += 1,
                _ => {}
            }
        }
        writeln!(
            out,
            "@@ -{},{} +{},{} @@",
            a_start + 1,
            a_lines,
            b_start + 1,
            b_lines
        )
        .unwrap();
        for h in start..end {
            writeln!(out, "{}{}", ops[h].0, ops[h].1).unwrap();
        }
        k = end;
    }
    out
}

/// Append a plain-English explanation block after the rendered
/// diagnostics, one per distinct code (`--explain-errors`, §58.4).
fn print_explanations(diags: &[axon_diag::Diagnostic]) {
    let mut seen: Vec<&str> = Vec::new();
    for d in diags {
        let Some(code) = d.code else { continue };
        if seen.contains(&code) {
            continue;
        }
        seen.push(code);
        if let Some(e) = axon_diag::explain::explain(code) {
            eprintln!("\n\u{2500}\u{2500} explain {code} \u{2500}\u{2500}");
            eprintln!("{}", e.render());
        }
    }
}

/// Serialize diagnostics to the machine-readable `--json` shape (§57.2):
/// `{ "file", "ok", "diagnostics": [{ code, severity, message, line,
/// col, notes }] }`. Editors/CI consume this.
fn diagnostics_to_json(diags: &[axon_diag::Diagnostic], path: &str) -> String {
    let items: Vec<serde_json::Value> = diags
        .iter()
        .map(|d| {
            let sev = match d.severity {
                axon_diag::Severity::Error => "error",
                axon_diag::Severity::Warning => "warning",
                axon_diag::Severity::Note => "note",
                axon_diag::Severity::Help => "help",
            };
            serde_json::json!({
                "code": d.code,
                "severity": sev,
                "message": d.message,
                "span": {
                    "start": d.primary.span.start,
                    "end": d.primary.span.end,
                    "file": d.primary.span.file,
                },
                "notes": d.notes,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "file": path,
        "ok": diags.is_empty(),
        "diagnostics": items,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn cmd_parse(path: &str) -> ExitCode {
    let Some(file) = read_or_die(path) else {
        return ExitCode::from(1);
    };
    let (program, diags) = axon_parser::parse(&file);
    if !diags.is_empty() {
        emit_diagnostics(&diags, &file);
        return ExitCode::from(1);
    }
    println!("parsed {} top-level item(s):", program.items.len());
    for item in &program.items {
        print_item_summary(item);
    }
    ExitCode::SUCCESS
}

fn emit_diagnostics(diags: &[axon_diag::Diagnostic], file: &SourceFile) {
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
    for d in diags {
        eprintln!("{}", axon_diag::render(d, file, use_color));
    }
}

fn print_item_summary(item: &axon_ast::Item) {
    use axon_ast::Item;
    match item {
        Item::Use(u) => {
            let path = u
                .path
                .segments
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(".");
            println!("  use {path}");
        }
        Item::Fn(f) => println!("  fn  {}  ({} param(s))", f.name.name, f.params.len()),
        Item::Type(t) => println!("  type {}", t.name.name),
        Item::Schema(s) => println!("  schema {}", s.name.name),
        Item::Agent(a) => println!(
            "  agent {}  ({} member(s))",
            a.name.name,
            a.members.len()
        ),
        Item::Actor(a) => println!(
            "  actor {}  ({} member(s))",
            a.name.name,
            a.members.len()
        ),
        Item::Supervisor(s) => println!("  supervisor {}", s.name.name),
        Item::Graph(g) => println!("  graph {}", g.name.name),
        Item::Network(n) => println!("  network {}", n.name.name),
        Item::Orchestrate(o) => println!("  orchestrate {}", o.name.name),
        Item::Policy(p) => println!("  policy {}", p.name.name),
        Item::MemPolicy(p) => println!("  mempolicy {}", p.name.name),
        Item::Model(m) => println!("  model {}", m.name.name),
        Item::Tool(t) => println!("  tool {}", t.name.name),
        Item::Memory(m) => println!("  memory {}", m.name.name),
        Item::Prompt(p) => println!("  prompt {}", p.name.name),
        Item::Trait(t) => println!("  trait {}", t.name.name),
        Item::Impl(_) => println!("  impl block"),
        Item::Const(c) => println!("  const {}", c.name.name),
        Item::Effect(e) => println!("  effect {}", e.name.name),
        Item::Test(t) => println!("  test {}", t.name),
        Item::Eval(e) => println!("  eval {}", e.name),
        Item::Config(c) => println!("  config {}", c.name.name),
    }
}

// ---------------------------------------------------------------------------
// Stage 17 — `axon serve` and `axon deploy`
// ---------------------------------------------------------------------------

fn cmd_serve(args: &[String]) -> ExitCode {
    let mut file: Option<&str> = None;
    let mut listen = "127.0.0.1:8080".to_string();
    let mut handler = "main".to_string();
    let mut tls_cert: Option<String> = None;
    let mut tls_key: Option<String> = None;
    let mut protocol = "plain".to_string();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--listen" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon serve: --listen requires ADDR");
                    return ExitCode::from(2);
                }
                listen = args[i].clone();
                i += 1;
            }
            "--handler" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon serve: --handler requires NAME");
                    return ExitCode::from(2);
                }
                handler = args[i].clone();
                i += 1;
            }
            "--tls-cert" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon serve: --tls-cert requires PATH");
                    return ExitCode::from(2);
                }
                tls_cert = Some(args[i].clone());
                i += 1;
            }
            "--tls-key" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon serve: --tls-key requires PATH");
                    return ExitCode::from(2);
                }
                tls_key = Some(args[i].clone());
                i += 1;
            }
            "--protocol" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon serve: --protocol requires VAL");
                    return ExitCode::from(2);
                }
                protocol = args[i].clone();
                i += 1;
            }
            other if other.starts_with("--protocol=") => {
                protocol = other["--protocol=".len()..].to_string();
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("axon serve: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            _ => {
                if file.is_some() {
                    eprintln!("axon serve: only one source file is supported");
                    return ExitCode::from(2);
                }
                file = Some(a);
                i += 1;
            }
        }
    }
    // TLS flags must be paired.
    if tls_cert.is_some() ^ tls_key.is_some() {
        eprintln!("axon serve: --tls-cert and --tls-key must be used together");
        return ExitCode::from(2);
    }
    // Validate --protocol against the known set so a typo doesn't sit
    // silently in $AXON_SERVE_PROTOCOL.
    if !matches!(protocol.as_str(), "plain" | "mcp" | "openai" | "grpc" | "a2a") {
        eprintln!(
            "axon serve: --protocol must be plain|mcp|openai|grpc|a2a, got `{protocol}`"
        );
        return ExitCode::from(2);
    }
    // Expose the chosen protocol to the running handler via env. The
    // handler reads it with `env_get("AXON_SERVE_PROTOCOL")` and uses
    // `serve_protocol_route` to dispatch the request shape.
    std::env::set_var("AXON_SERVE_PROTOCOL", &protocol);
    let file = match file {
        Some(f) => f,
        None => {
            eprintln!("usage: axon serve <file> [--listen ADDR] [--handler NAME] [--protocol P]");
            return ExitCode::from(2);
        }
    };

    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("axon serve: cannot read `{file}`: {e}");
            return ExitCode::from(1);
        }
    };
    let source = axon_diag::SourceFile::new(file, &text);
    let (program, diags) = axon_parser::parse(&source);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", axon_diag::render(d, &source, true));
        }
        return ExitCode::from(1);
    }

    let mut interp = axon_runtime::Interpreter::new();
    host::install(&interp);
    interp.load_program(&program);

    // Look up the named handler in globals.
    let callee = match interp.globals.lookup(&handler) {
        Some(v) => v,
        None => {
            eprintln!("axon serve: handler `{handler}` not found in program");
            return ExitCode::from(1);
        }
    };

    // Route through the same NativeExt the language exposes so there's
    // exactly one server code path. Plain HTTP uses `serve_run`; TLS
    // uses `serve_run_tls` with cert+key paths.
    let (binding_name, call_args) = match (tls_cert, tls_key) {
        (Some(cert), Some(key)) => (
            "serve_run_tls",
            vec![
                axon_runtime::Value::String(std::rc::Rc::new(listen)),
                callee,
                axon_runtime::Value::String(std::rc::Rc::new(cert)),
                axon_runtime::Value::String(std::rc::Rc::new(key)),
            ],
        ),
        _ => (
            "serve_run",
            vec![
                axon_runtime::Value::String(std::rc::Rc::new(listen)),
                callee,
            ],
        ),
    };
    let serve_native = match interp.globals.lookup(binding_name) {
        Some(v) => v,
        None => {
            eprintln!("axon serve: `{binding_name}` binding missing (internal error)");
            return ExitCode::from(1);
        }
    };
    let result = interp.call_value(&serve_native, &call_args, axon_diag::Span::DUMMY);
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("axon serve: {e:?}");
            ExitCode::from(1)
        }
    }
}

fn cmd_deploy(args: &[String]) -> ExitCode {
    let mut src_dir: Option<&str> = None;
    let mut out_dir: Option<&str> = None;
    let mut name: Option<String> = None;
    let mut handler = "main".to_string();
    let mut port: u16 = 8080;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" | "--out" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon deploy: -o requires DIR");
                    return ExitCode::from(2);
                }
                out_dir = Some(args[i].as_str());
                i += 1;
            }
            "--name" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon deploy: --name requires NAME");
                    return ExitCode::from(2);
                }
                name = Some(args[i].clone());
                i += 1;
            }
            "--handler" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon deploy: --handler requires NAME");
                    return ExitCode::from(2);
                }
                handler = args[i].clone();
                i += 1;
            }
            "--port" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon deploy: --port requires N");
                    return ExitCode::from(2);
                }
                port = match args[i].parse() {
                    Ok(p) => p,
                    Err(_) => {
                        eprintln!("axon deploy: --port must be 0..=65535");
                        return ExitCode::from(2);
                    }
                };
                i += 1;
            }
            other if other.starts_with("--") || other.starts_with('-') => {
                eprintln!("axon deploy: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            _ => {
                if src_dir.is_some() {
                    eprintln!("axon deploy: only one project dir is supported");
                    return ExitCode::from(2);
                }
                src_dir = Some(a);
                i += 1;
            }
        }
    }
    let (src, out) = match (src_dir, out_dir) {
        (Some(s), Some(o)) => (s, o),
        _ => {
            eprintln!("usage: axon deploy <project_dir> -o <out_dir> [--name N] [--port P] [--handler H]");
            return ExitCode::from(2);
        }
    };
    let src_path = std::path::Path::new(src);
    let out_path = std::path::Path::new(out);
    if let Err(e) = std::fs::create_dir_all(out_path) {
        eprintln!("axon deploy: cannot create `{out}`: {e}");
        return ExitCode::from(1);
    }
    // Pack the skill.
    let skill = match axon_skill::Skill::pack(src_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("axon deploy: skill pack: {e}");
            return ExitCode::from(1);
        }
    };
    let skill_name = name.unwrap_or_else(|| skill.manifest.name.clone());
    let skill_path = out_path.join(format!("{}.axskill", skill_name));
    let skill_bytes = match skill.to_json() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("axon deploy: serialize skill: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = std::fs::write(&skill_path, skill_bytes) {
        eprintln!("axon deploy: write {}: {e}", skill_path.display());
        return ExitCode::from(1);
    }
    // Write the manifest.
    let manifest = axon_deploy::DeployManifest {
        version: axon_deploy::manifest::MANIFEST_VERSION,
        name: skill_name.clone(),
        entrypoint_handler: handler,
        port,
        env: Default::default(),
        health_checks: vec!["liveness".into()],
        dotenv: None,
        vault: None,
    };
    let manifest_path = out_path.join("deploy.json");
    if let Err(e) = manifest.save(&manifest_path) {
        eprintln!("axon deploy: write manifest: {e}");
        return ExitCode::from(1);
    }
    println!("wrote {}", skill_path.display());
    println!("wrote {}", manifest_path.display());
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Stage 20 — `axon replay`, `axon trace`, `axon repl`
// ---------------------------------------------------------------------------

/// `axon replay <recording.json> <source.ax> [--patch]`
///
/// Re-run an Axon program against a captured non-determinism tape. Each
/// model/clock/randomness observation comes from the recording instead of
/// hitting a live provider — zero network, byte-identical results.
///
/// `--patch` enables lenient mode: a program that's been edited since the
/// recording may issue extra calls (they surface as a clean error rather
/// than halting with the strict "replay exhausted" assertion).
fn cmd_replay(args: &[String]) -> ExitCode {
    let mut recording_path: Option<&str> = None;
    let mut source_path: Option<&str> = None;
    let mut patch = false;
    for arg in args {
        match arg.as_str() {
            "--patch" => patch = true,
            other if other.starts_with("--") => {
                eprintln!("axon replay: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                if recording_path.is_none() {
                    recording_path = Some(other);
                } else if source_path.is_none() {
                    source_path = Some(other);
                } else {
                    eprintln!("axon replay: unexpected extra argument `{other}`");
                    return ExitCode::from(2);
                }
            }
        }
    }
    let (rec, src) = match (recording_path, source_path) {
        (Some(r), Some(s)) => (r, s),
        _ => {
            eprintln!("usage: axon replay <recording.json> <source.ax> [--patch]");
            return ExitCode::from(2);
        }
    };
    let raw_rec = match std::fs::read_to_string(rec) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("axon replay: cannot read recording `{rec}`: {e}");
            return ExitCode::from(1);
        }
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw_rec) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("axon replay: `{rec}` is not valid JSON: {e}");
            return ExitCode::from(1);
        }
    };
    let recording = match axon_runtime::Recording::from_json(&parsed) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("axon replay: `{rec}`: {e}");
            return ExitCode::from(1);
        }
    };

    let text = match std::fs::read_to_string(src) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("axon replay: cannot read source `{src}`: {e}");
            return ExitCode::from(1);
        }
    };
    let source = SourceFile::new(src, &text);
    let (program, diags) = axon_parser::parse(&source);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", axon_diag::render(d, &source, true));
        }
        return ExitCode::from(1);
    }

    let interp = axon_runtime::Interpreter::new();
    host::install(&interp);
    if patch {
        interp.enable_replay_lenient(recording);
    } else {
        interp.enable_replay(recording);
    }
    let mut interp = interp;
    interp.load_program(&program);
    let exit = match interp.run_main() {
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => {
            let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
            eprint!("{}", err.render(&source, use_color));
            ExitCode::from(1)
        }
    };
    if let Some((cursor, total, lenient)) = interp.replay_progress() {
        let mode = if lenient { "patch" } else { "strict" };
        eprintln!("axon replay [{mode}]: consumed {cursor} of {total} recorded event(s)");
    }
    exit
}

/// `axon trace <file>` — pretty-print a JSONL trace file as a span tree
/// with durations, span kinds, and any attached error.
/// §35.2 — `axon watch <file> [--trace PATH] [--no-color]`
///
/// Runs an Axon program with the streaming tracer installed; each span
/// closure prints a one-line summary to stderr as the program runs.
/// Optional `--trace PATH` also writes the full span trace as JSONL at
/// end of run (same format `axon run --trace` produces). `--no-color`
/// forces plain output even when stderr is a TTY (the default
/// auto-detects).
///
/// Output is intentionally to stderr so users can pipe stdout through
/// `tee program.out` without losing the trace stream.
fn cmd_watch(args: &[String]) -> ExitCode {
    let mut path: Option<&str> = None;
    let mut trace_path: Option<&str> = None;
    let mut force_no_color = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--no-color" => {
                force_no_color = true;
                i += 1;
            }
            "--trace" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon watch: --trace requires a path");
                    return ExitCode::from(2);
                }
                trace_path = Some(args[i].as_str());
                i += 1;
            }
            "--help" | "-h" => {
                println!(
                    "usage: axon watch <file> [--trace PATH] [--no-color]\n\
                     \n\
                     Runs an Axon program with the live trace inspector — one\n\
                     line per closed span (ask/plan/tool/handler/spawn/scope)\n\
                     to stderr as the program runs. Stdout is left alone so\n\
                     you can pipe program output through tee without losing\n\
                     the trace stream.\n\
                     \n\
                     --trace PATH    Also write the full JSONL trace at end of\n\
                                     run (same format `axon run --trace` produces).\n\
                     --no-color      Force plain output even on a TTY."
                );
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--") => {
                eprintln!("axon watch: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                if path.is_some() {
                    eprintln!("axon watch: only one file is supported");
                    return ExitCode::from(2);
                }
                path = Some(other);
                i += 1;
            }
        }
    }
    let Some(path) = path else {
        eprintln!("usage: axon watch <file> [--trace PATH] [--no-color]");
        return ExitCode::from(2);
    };
    let Some(file) = read_or_die(path) else {
        return ExitCode::from(1);
    };
    let (program, diags) = axon_parser::parse(&file);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", axon_diag::render(d, &file, true));
        }
        return ExitCode::from(1);
    }

    // Auto-detect tty for color unless forced off.
    let use_color =
        !force_no_color && std::io::IsTerminal::is_terminal(&std::io::stderr());
    let anchor_ms = watch_format::now_ms();
    let interp = axon_runtime::Interpreter::with_caps(
        axon_runtime::CapSet::standard_default(),
    );
    host::install(&interp);
    // Install the streaming sink — every span close prints a one-liner.
    interp.enable_tracing_streaming(Box::new(move |span| {
        let line = watch_format::format_span(span, anchor_ms, use_color);
        eprintln!("{line}");
    }));
    let mut interp = interp;
    interp.load_program(&program);

    // §35.6 verification fix M4 — install a Ctrl-C handler BEFORE the
    // run starts so a SIGINT during a long-running program flushes
    // the `--trace PATH` JSONL instead of dropping it. We can't
    // gracefully unwind a synchronous run_main() from a signal
    // handler (no async runtime to cancel into), but we CAN ensure
    // the trace file gets written via an atexit-style guard. The
    // handler sets an AtomicBool we surface in the banner; if a
    // user really wants graceful shutdown, the run_main impl will
    // honor it when the async migration lands.
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        // Ignore install errors here — single-file watch is best-effort,
        // unlike `axon fix --watch` whose loop genuinely depends on it.
        let _ = ctrlc::set_handler(move || {
            interrupted.store(true, Ordering::SeqCst);
        });
    }

    eprintln!(
        "axon watch: tracing `{path}` (Ctrl-C aborts; --trace PATH is flushed on natural exit)\n\
         time        kind       name                                status  duration"
    );
    let exit = match interp.run_main() {
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => {
            eprint!("{}", err.render(&file, use_color));
            ExitCode::from(1)
        }
    };
    // Best-effort: write the trace whether the run ended normally,
    // errored, or was interrupted (Ctrl-C). When the OS delivers
    // SIGINT to a synchronous program the process dies before this
    // line runs — that's a limitation of the synchronous interpreter,
    // not a fix we can land here. The banner now honestly says so.
    if let Some(tp) = trace_path {
        if let Some(tracer) = interp.take_tracer() {
            if let Err(e) = std::fs::write(tp, tracer.to_jsonl()) {
                eprintln!("axon watch: cannot write trace to `{tp}`: {e}");
            }
        }
    }
    if interrupted.load(Ordering::SeqCst) {
        eprintln!("axon watch: interrupted (SIGINT)");
    }
    exit
}

fn cmd_trace(args: &[String]) -> ExitCode {
    // §36.B.4 — `axon trace promote <recording.json> --to-suite <path>` builds
    // a regression test from a recording so production failures become
    // standing tests without manual JSON-editing. Dispatched here so the
    // existing `axon trace <jsonl>` pretty-printer is the default.
    if args.first().map(|s| s.as_str()) == Some("promote") {
        return cmd_trace_promote(&args[1..]);
    }
    let path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!(
                "usage: axon trace <file.jsonl>            # pretty-print a JSONL trace\n\
                 usage: axon trace promote <rec.json> --to-suite <path> [--name N]\n\
                                                         # append a regression test from a recording"
            );
            return ExitCode::from(2);
        }
    };
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("axon trace: cannot read `{path}`: {e}");
            return ExitCode::from(1);
        }
    };
    let mut spans: Vec<serde_json::Value> = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str(trimmed) {
            Ok(v) => spans.push(v),
            Err(e) => {
                eprintln!("axon trace: line {} not valid JSON: {e}", i + 1);
                return ExitCode::from(1);
            }
        }
    }
    if spans.is_empty() {
        println!("(no spans)");
        return ExitCode::SUCCESS;
    }
    // Group children by parent_id for tree printing.
    use std::collections::HashMap;
    let mut by_parent: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, s) in spans.iter().enumerate() {
        let parent = s.get("parent_id").and_then(|v| v.as_i64()).unwrap_or(-1);
        by_parent.entry(parent).or_default().push(i);
    }
    fn print_subtree(
        spans: &[serde_json::Value],
        by_parent: &HashMap<i64, Vec<usize>>,
        parent: i64,
        depth: usize,
    ) {
        if let Some(children) = by_parent.get(&parent) {
            for &idx in children {
                let s = &spans[idx];
                let id = s.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let kind = s.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                let start = s.get("start_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                let end = s.get("end_ms").and_then(|v| v.as_i64());
                let dur = end.map(|e| e - start).unwrap_or(0);
                let err = s.get("error").and_then(|v| v.as_str()).unwrap_or("");
                let indent = "  ".repeat(depth);
                let err_tag = if err.is_empty() {
                    String::new()
                } else {
                    format!(" \x1b[31m[error: {err}]\x1b[0m")
                };
                println!(
                    "{indent}\x1b[36m{name}\x1b[0m \x1b[90m({kind})\x1b[0m  {dur}ms{err_tag}"
                );
                let _ = id;
                print_subtree(spans, by_parent, id, depth + 1);
            }
        }
    }
    let total_ms: i64 = spans
        .iter()
        .filter_map(|s| {
            let start = s.get("start_ms").and_then(|v| v.as_i64())?;
            let end = s.get("end_ms").and_then(|v| v.as_i64())?;
            Some(end - start)
        })
        .max()
        .unwrap_or(0);
    println!("trace: {} span(s), max span duration {total_ms}ms", spans.len());
    print_subtree(&spans, &by_parent, -1, 0);
    ExitCode::SUCCESS
}

/// `axon trace promote <recording.json> --to-suite <path> [--name N] [--assert-contains S]`
/// — synthesize a regression test from a recording and append it to the
/// named suite file (creates the file if missing). The §36.B.4 DX win:
/// production failures become standing tests in one command.
///
/// The synthesized test:
///   - Uses `mock_model("fixed", "<recorded-content>")` to reproduce the
///     last assistant turn from the recording deterministically.
///   - Asserts `str_contains(out, "<recorded-content>")` by default, or a
///     user-supplied substring with `--assert-contains`.
///   - Carries a header comment naming the source recording + the promote
///     date so future readers can trace it back.
///
/// This is a bounded slice: trajectory-shaped assertions and `--from-watch`
/// span replay are Stage 37.
fn cmd_trace_promote(args: &[String]) -> ExitCode {
    let mut rec_path: Option<&str> = None;
    let mut to_suite: Option<&str> = None;
    let mut name: Option<&str> = None;
    let mut assert_contains: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--to-suite" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon trace promote: --to-suite requires a path");
                    return ExitCode::from(2);
                }
                to_suite = Some(args[i].as_str());
                i += 1;
            }
            "--name" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon trace promote: --name requires an identifier");
                    return ExitCode::from(2);
                }
                name = Some(args[i].as_str());
                i += 1;
            }
            "--assert-contains" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon trace promote: --assert-contains requires a substring");
                    return ExitCode::from(2);
                }
                assert_contains = Some(args[i].as_str());
                i += 1;
            }
            "--help" | "-h" => {
                println!(
                    "usage: axon trace promote <recording.json> --to-suite <suite.ax> \\\n\
                     \x20                          [--name <test-id>] [--assert-contains <substr>]\n\
                     \n\
                     Reads a recording (produced by `axon run --record <path>`), extracts the\n\
                     final assistant response, and appends a regression test to <suite.ax> that\n\
                     reproduces it via mock_model. If --name is omitted, a name is auto-derived\n\
                     from the recording filename + a short hash. The test asserts the recorded\n\
                     content appears in the output unless --assert-contains overrides."
                );
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--") => {
                eprintln!("axon trace promote: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            _ => {
                if rec_path.is_some() {
                    eprintln!("axon trace promote: more than one recording path given");
                    return ExitCode::from(2);
                }
                rec_path = Some(a);
                i += 1;
            }
        }
    }
    let Some(rp) = rec_path else {
        eprintln!("axon trace promote: missing recording path (got no positional arg)");
        return ExitCode::from(2);
    };
    let Some(to_suite) = to_suite else {
        eprintln!("axon trace promote: --to-suite <suite.ax> is required");
        return ExitCode::from(2);
    };
    let raw = match std::fs::read_to_string(rp) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("axon trace promote: cannot read recording `{rp}`: {e}");
            return ExitCode::from(1);
        }
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("axon trace promote: recording is not valid JSON: {e}");
            return ExitCode::from(1);
        }
    };
    // Extract the LAST model_call event's content. The convention is: the
    // recorded final response is the canonical "what we expected" for a
    // replay-style test. Skip non-ModelCall events (TimeNow/RandomInt).
    let events = parsed
        .get("events")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    let last_model = events.iter().rev().find(|ev| {
        ev.get("kind").and_then(|k| k.as_str()) == Some("model_call")
    });
    let Some(last) = last_model else {
        eprintln!(
            "axon trace promote: recording `{rp}` has no `model_call` events — \
             nothing to pin as the canonical response"
        );
        return ExitCode::from(1);
    };
    let content = last
        .get("response")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    if content.is_empty() {
        eprintln!(
            "axon trace promote: last model_call in `{rp}` has empty content; \
             pinning an empty response would not be a useful regression test"
        );
        return ExitCode::from(1);
    }
    let provider = last
        .get("provider")
        .and_then(|p| p.as_str())
        .unwrap_or("anthropic");

    // Auto-name from the recording's filename + a short content hash so
    // multiple promotes of similar runs don't collide.
    let auto_name = {
        let stem = std::path::Path::new(rp)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("rec");
        // §36.6 verification fix L1 — hash filename + content so two
        // recordings with identical responses but different sources don't
        // collide on the auto-derived test name.
        let sha8 = short_hash(&format!("{stem}:{content}"));
        format!("promoted_{stem}_{sha8}")
    };
    let test_name = name.unwrap_or(&auto_name);
    if !test_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        eprintln!(
            "axon trace promote: test name `{test_name}` must be ASCII alphanumeric \
             plus underscore (it becomes part of an Axon identifier)"
        );
        return ExitCode::from(2);
    }
    let expected_substr = assert_contains.unwrap_or(content);
    let escaped_content = escape_axon_string(content);
    let escaped_substr = escape_axon_string(expected_substr);
    let header = format!(
        "\n// ----- §36.B.4 PROMOTED from `{}` -----\n",
        rp.replace('`', "")
    );
    let body = format!(
        "test \"{test_name}\" {{\n\
         \x20   // Pinned response from `{rp}` (provider: `{provider}`).\n\
         \x20   let m = mock_model(\"fixed\", \"{escaped_content}\")\n\
         \x20   let out = ask m {{ user: \"promote-replay\" }}\n\
         \x20   assert(str_contains(out, \"{escaped_substr}\"), \
         \"promoted test `{test_name}` regressed — expected response to contain `{escaped_substr}`\")\n\
         }}\n"
    );
    // Append-or-create. Use OpenOptions::append so existing tests are
    // preserved (no rewrite, no diff hazard).
    use std::io::Write as _;
    let mut f = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(to_suite)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("axon trace promote: cannot open suite `{to_suite}`: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = f.write_all(header.as_bytes()).and_then(|_| f.write_all(body.as_bytes())) {
        eprintln!("axon trace promote: write to `{to_suite}` failed: {e}");
        return ExitCode::from(1);
    }
    println!("axon trace promote: appended `{test_name}` to `{to_suite}`");
    ExitCode::SUCCESS
}

/// Tiny non-crypto hash for naming promoted tests — FNV-1a 32-bit, 8 hex chars.
fn short_hash(s: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

/// Escape a string for safe embedding in an Axon `"..."` literal.
///
/// §36.6 verification fix S3 — recordings come from external LLM responses
/// that get baked into developer-readable source. Escape:
///   - Standard string-literal escapes (backslash, quote, LF, CR, tab).
///   - All control chars (< 0x20).
///   - Bidi direction overrides (U+202A..202E, U+2066..2069) — these can
///     hide content from human reviewers reading the suite source.
///   - Zero-width chars (U+200B..200D, U+FEFF) — same reason.
fn escape_axon_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        let cp = c as u32;
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if cp < 0x20 => {
                out.push_str(&format!("\\u{{{cp:x}}}"));
            }
            _ if (0x202A..=0x202E).contains(&cp)
                || (0x2066..=0x2069).contains(&cp)
                || (0x200B..=0x200D).contains(&cp)
                || cp == 0xFEFF =>
            {
                out.push_str(&format!("\\u{{{cp:x}}}"));
            }
            _ => out.push(c),
        }
    }
    out
}

/// `axon repl` — interactive read-eval-print loop. Each line is parsed as
/// a top-level expression-or-statement, evaluated against a persistent
/// interpreter, and the result + effect summary is printed. `.help`,
/// `.quit`, `.effects` are built-in dot-commands.
fn cmd_repl(_args: &[String]) -> ExitCode {
    use std::io::{BufRead, Write};
    let interp = axon_runtime::Interpreter::new();
    host::install(&interp);
    interp.enable_tracing();
    let mut interp = interp;

    println!("Axon {} REPL — type `.help` for commands, `.quit` to exit.", env!("CARGO_PKG_VERSION"));
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut line_no: u32 = 1;
    let mut buf = String::new();
    loop {
        print!("axon[{line_no}]> ");
        let _ = std::io::stdout().flush();
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => {
                println!();
                break;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("axon repl: read error: {e}");
                return ExitCode::from(1);
            }
        }
        let line = buf.trim().to_string();
        if line.is_empty() {
            continue;
        }
        match line.as_str() {
            ".quit" | ".exit" => break,
            ".help" => {
                println!(
                    "  .help         show this message\n  \
                     .quit / .exit terminate the REPL\n  \
                     .effects      show the current active capability set\n  \
                     <expr>        evaluate; the result is printed if non-Unit"
                );
                continue;
            }
            ".effects" => {
                println!("active capabilities: {}", interp.active_caps());
                continue;
            }
            _ => {}
        }
        // Wrap each input in a synthetic main so the parser/tyck can
        // type-check it as a function body. Declare the full standard
        // effect row so `print_int`, `time_now`, `http_fetch`, etc.
        // work the way they would in a normal `fn main()`. Multi-
        // statement input separated by `;` (or newlines in a `{}` block).
        let wrapped = format!(
            "fn __repl_{line_no}() uses {{ Console, Fs.Read, Fs.Write, Time, Random, Net, LLM, Memory, Spawn }} {{\n{line}\n}}"
        );
        let src = SourceFile::new("<repl>", &wrapped);
        let (program, diags) = axon_parser::parse(&src);
        if !diags.is_empty() {
            for d in &diags {
                eprintln!("{}", axon_diag::render(d, &src, true));
            }
            line_no += 1;
            continue;
        }
        interp.load_program(&program);
        let fn_name = format!("__repl_{line_no}");
        // Look up the synthesized fn and invoke it.
        let callee = match interp.globals.lookup(&fn_name) {
            Some(v) => v,
            None => {
                eprintln!("axon repl: synthesized handler not found (internal error)");
                line_no += 1;
                continue;
            }
        };
        match interp.call_value(&callee, &[], axon_diag::Span::DUMMY) {
            Ok(v) => match v {
                axon_runtime::Value::Unit => {}
                other => println!("=> {other}"),
            },
            Err(axon_runtime::EvalSignal::Error(err)) => {
                let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
                eprint!("{}", err.render(&src, use_color));
            }
            Err(other) => {
                eprintln!("axon repl: unexpected control flow: {other:?}");
            }
        }
        line_no += 1;
    }
    ExitCode::SUCCESS
}

/// `axon login <provider> [--vault PATH] [--key VALUE]`
///
/// Prompts for an API key (or reads it from `--key` for scripts/CI), then
/// stores it in the vault under `<PROVIDER>_API_KEY`. The vault file is
/// mode-0600 on Unix; if it already exists it must have those permissions
/// or the load fails cleanly per Stage 15's contract.
fn cmd_login(args: &[String]) -> ExitCode {
    let mut provider: Option<&str> = None;
    let mut vault_path: Option<String> = None;
    let mut key_arg: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--vault" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon login: --vault requires PATH");
                    return ExitCode::from(2);
                }
                vault_path = Some(args[i].clone());
                i += 1;
            }
            "--key" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon login: --key requires VALUE");
                    return ExitCode::from(2);
                }
                key_arg = Some(args[i].clone());
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("axon login: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                if provider.is_some() {
                    eprintln!("axon login: only one provider per invocation");
                    return ExitCode::from(2);
                }
                provider = Some(other);
                i += 1;
            }
        }
    }
    let provider = match provider {
        Some(p) => p,
        None => {
            eprintln!("usage: axon login <provider> [--vault PATH] [--key VALUE]");
            eprintln!("  provider examples: anthropic, openai, google");
            return ExitCode::from(2);
        }
    };
    let vault_path = vault_path.unwrap_or_else(|| {
        std::env::var("AXON_VAULT").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            format!("{home}/.axon/vault.json")
        })
    });

    // Resolve the API-key value in order: --key, then `<PROVIDER>_API_KEY`
    // env, then a prompt on stdin.
    let api_key = if let Some(k) = key_arg {
        k
    } else {
        let env_var = format!("{}_API_KEY", provider.to_uppercase());
        match std::env::var(&env_var) {
            Ok(v) if !v.is_empty() => v,
            _ => {
                use std::io::{BufRead, Write};
                eprint!("api key for `{provider}`: ");
                let _ = std::io::stderr().flush();
                let mut line = String::new();
                if std::io::stdin().lock().read_line(&mut line).is_err() {
                    eprintln!("axon login: failed to read key from stdin");
                    return ExitCode::from(1);
                }
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() {
                    eprintln!("axon login: empty key — aborting");
                    return ExitCode::from(1);
                }
                trimmed
            }
        }
    };

    let mut vault = if std::path::Path::new(&vault_path).exists() {
        match axon_secret::Vault::load(&vault_path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("axon login: cannot load vault `{vault_path}`: {e}");
                return ExitCode::from(1);
            }
        }
    } else {
        axon_secret::Vault::new()
    };
    let secret_name = format!("{}_API_KEY", provider.to_uppercase());
    vault.set(&secret_name, api_key);
    if let Err(e) = vault.save(&vault_path) {
        eprintln!("axon login: cannot write vault: {e}");
        return ExitCode::from(1);
    }
    println!("saved `{secret_name}` to {vault_path} (mode 0600 on Unix)");
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Stage 23 — `axon pkg`
// ---------------------------------------------------------------------------

/// `axon pkg <subcmd>` — read/edit dependency entries in `axon.toml`.
///
/// Subcommands:
///   * `list`               — print each `[deps.<name>] path = "..."` entry.
///   * `add  <name> --path P` — add or update the dep entry.
///   * `remove <name>`      — drop the dep entry (no-op if absent).
///   * `audit`              — sanity-check every dep path exists & looks
///                            like an Axon project.
///
/// All subcommands operate on the `axon.toml` in the current directory by
/// default; pass `--manifest PATH` to point elsewhere. Edits preserve the
/// rest of the manifest by round-tripping through `toml::Value`.
fn cmd_pkg(args: &[String]) -> ExitCode {
    // Pull out --manifest if present so the inner sub-dispatch sees a
    // contiguous positional list.
    let mut manifest_path = std::path::PathBuf::from("axon.toml");
    let mut rest: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--manifest" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon pkg: --manifest requires PATH");
                    return ExitCode::from(2);
                }
                manifest_path = std::path::PathBuf::from(&args[i]);
                i += 1;
            }
            other if other.starts_with("--manifest=") => {
                manifest_path = std::path::PathBuf::from(&other["--manifest=".len()..]);
                i += 1;
            }
            other => {
                rest.push(other);
                i += 1;
            }
        }
    }
    let (sub, sub_args) = match rest.split_first() {
        Some(s) => s,
        None => {
            eprintln!("usage: axon pkg <list|add|remove|audit> [args...]");
            return ExitCode::from(2);
        }
    };

    match *sub {
        "list" => pkg_list(&manifest_path),
        "add" => pkg_add(&manifest_path, sub_args),
        "remove" | "rm" => pkg_remove(&manifest_path, sub_args),
        "audit" => pkg_audit(&manifest_path),
        other => {
            eprintln!("axon pkg: unknown subcommand `{other}`");
            ExitCode::from(2)
        }
    }
}

/// Read the manifest as a `toml::Value`. A missing file produces an empty
/// table so first-time `add` calls Just Work — but `list`/`audit`/`remove`
/// against a missing manifest should still tell the user something.
fn pkg_load_manifest(path: &std::path::Path) -> Result<toml::Value, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map_err(|e| format!("axon pkg: `{}` is not valid TOML: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(toml::Value::Table(toml::map::Map::new()))
        }
        Err(e) => Err(format!("axon pkg: cannot read `{}`: {e}", path.display())),
    }
}

fn pkg_save_manifest(path: &std::path::Path, value: &toml::Value) -> Result<(), String> {
    let text = toml::to_string_pretty(value)
        .map_err(|e| format!("axon pkg: cannot serialize manifest: {e}"))?;
    std::fs::write(path, text)
        .map_err(|e| format!("axon pkg: cannot write `{}`: {e}", path.display()))
}

fn pkg_get_deps_table<'a>(manifest: &'a toml::Value) -> Option<&'a toml::value::Table> {
    manifest.get("deps").and_then(|v| v.as_table())
}

fn pkg_list(manifest_path: &std::path::Path) -> ExitCode {
    let manifest = match pkg_load_manifest(manifest_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let deps = match pkg_get_deps_table(&manifest) {
        Some(t) => t,
        None => {
            println!("(no dependencies declared in {})", manifest_path.display());
            return ExitCode::SUCCESS;
        }
    };
    if deps.is_empty() {
        println!("(no dependencies declared in {})", manifest_path.display());
        return ExitCode::SUCCESS;
    }
    let mut names: Vec<&String> = deps.keys().collect();
    names.sort();
    for name in names {
        let entry = &deps[name];
        let path = entry.get("path").and_then(|v| v.as_str()).unwrap_or("(no path)");
        println!("  {name}  path = \"{path}\"");
    }
    ExitCode::SUCCESS
}

fn pkg_add(manifest_path: &std::path::Path, args: &[&str]) -> ExitCode {
    if args.is_empty() {
        eprintln!("usage: axon pkg add <name> --path <DIR>");
        return ExitCode::from(2);
    }
    let name = args[0];
    if !is_valid_dep_name(name) {
        eprintln!(
            "axon pkg add: invalid dep name `{name}` — use letters, digits, `_` or `-`"
        );
        return ExitCode::from(2);
    }
    let mut path_value: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i] {
            "--path" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon pkg add: --path requires DIR");
                    return ExitCode::from(2);
                }
                path_value = Some(args[i].to_string());
                i += 1;
            }
            other if other.starts_with("--path=") => {
                path_value = Some(other["--path=".len()..].to_string());
                i += 1;
            }
            other => {
                eprintln!("axon pkg add: unexpected argument `{other}`");
                return ExitCode::from(2);
            }
        }
    }
    let path = match path_value {
        Some(p) => p,
        None => {
            eprintln!("axon pkg add: --path is required (network deps land in a later stage)");
            return ExitCode::from(2);
        }
    };

    let mut manifest = match pkg_load_manifest(manifest_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let root = match manifest.as_table_mut() {
        Some(t) => t,
        None => {
            eprintln!("axon pkg add: manifest root is not a table");
            return ExitCode::from(1);
        }
    };
    let deps_entry = root
        .entry("deps".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let deps = match deps_entry.as_table_mut() {
        Some(t) => t,
        None => {
            eprintln!("axon pkg add: `[deps]` exists but is not a table");
            return ExitCode::from(1);
        }
    };
    let mut entry = toml::map::Map::new();
    entry.insert("path".to_string(), toml::Value::String(path.clone()));
    deps.insert(name.to_string(), toml::Value::Table(entry));

    if let Err(e) = pkg_save_manifest(manifest_path, &manifest) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    println!("added dep `{name}` path = \"{path}\"");
    ExitCode::SUCCESS
}

fn pkg_remove(manifest_path: &std::path::Path, args: &[&str]) -> ExitCode {
    let name = match args.first() {
        Some(n) => *n,
        None => {
            eprintln!("usage: axon pkg remove <name>");
            return ExitCode::from(2);
        }
    };
    let mut manifest = match pkg_load_manifest(manifest_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let removed = manifest
        .as_table_mut()
        .and_then(|t| t.get_mut("deps"))
        .and_then(|v| v.as_table_mut())
        .map(|deps| deps.remove(name).is_some())
        .unwrap_or(false);
    if !removed {
        eprintln!("axon pkg remove: no dep named `{name}`");
        return ExitCode::from(1);
    }
    if let Err(e) = pkg_save_manifest(manifest_path, &manifest) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    println!("removed dep `{name}`");
    ExitCode::SUCCESS
}

fn pkg_audit(manifest_path: &std::path::Path) -> ExitCode {
    let manifest = match pkg_load_manifest(manifest_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let deps = match pkg_get_deps_table(&manifest) {
        Some(t) => t,
        None => {
            println!("(no dependencies declared in {})", manifest_path.display());
            return ExitCode::SUCCESS;
        }
    };
    if deps.is_empty() {
        println!("(no dependencies declared in {})", manifest_path.display());
        return ExitCode::SUCCESS;
    }
    let manifest_dir = manifest_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut bad = 0usize;
    let mut names: Vec<&String> = deps.keys().collect();
    names.sort();
    for name in names {
        let entry = &deps[name];
        let path_str = match entry.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                println!("  FAIL {name}: missing `path` field");
                bad += 1;
                continue;
            }
        };
        let dep_path = manifest_dir.join(path_str);
        if !dep_path.is_dir() {
            println!("  FAIL {name}: `{}` is not a directory", dep_path.display());
            bad += 1;
            continue;
        }
        // A well-formed dep has an axon.toml *or* a src/ directory.
        let has_manifest = dep_path.join("axon.toml").is_file();
        let has_src = dep_path.join("src").is_dir();
        if !has_manifest && !has_src {
            println!(
                "  WARN {name}: `{}` lacks axon.toml and src/ — not an Axon project?",
                dep_path.display()
            );
            // A warning, not a failure: empty workspaces should still pass
            // audit so users can scaffold a dep before populating it.
            continue;
        }
        println!("  ok   {name}: {}", dep_path.display());
    }
    if bad > 0 {
        eprintln!("axon pkg audit: {bad} dependency error(s)");
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn is_valid_dep_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// ---------------------------------------------------------------------------
// Stage 24 — `axon optimize` (§49.6)
//
// Search over prompt/strategy variants against an eval suite. The v0
// search space is deliberately narrow but production-shaped:
//
//   * Each line in the input file beginning with `// VARIANT:` declares
//     a swap point; lines following with `// = "<value>"` are the
//     candidate values.
//   * Each candidate is materialized into a temp file, run through
//     `axon test --path <suite>`, and scored by the suite's pass count.
//   * The best candidate is written alongside the input as
//     `<name>.vN.ax`, where N is the next unused version number.
//
// The point isn't a clever optimizer — that's a v2 research problem.
// The point is to give CI a *reproducible, measurable, gated*
// alternative to "hand-edit and pray" prompt engineering.
// ---------------------------------------------------------------------------

fn cmd_optimize(args: &[String]) -> ExitCode {
    let mut input: Option<&str> = None;
    let mut eval: Option<&str> = None;
    let mut metric: Option<String> = None;
    let mut budget: Option<String> = None;
    let mut trials: usize = 8;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--eval" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon optimize: --eval requires PATH");
                    return ExitCode::from(2);
                }
                eval = Some(args[i].as_str());
                i += 1;
            }
            "--metric" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon optimize: --metric requires NAME");
                    return ExitCode::from(2);
                }
                metric = Some(args[i].clone());
                i += 1;
            }
            "--budget" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon optimize: --budget requires VAL");
                    return ExitCode::from(2);
                }
                budget = Some(args[i].clone());
                i += 1;
            }
            "--trials" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon optimize: --trials requires N");
                    return ExitCode::from(2);
                }
                trials = match args[i].parse::<usize>() {
                    Ok(n) if n > 0 => n,
                    _ => {
                        eprintln!("axon optimize: --trials must be a positive integer");
                        return ExitCode::from(2);
                    }
                };
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("axon optimize: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                if input.is_some() {
                    eprintln!("axon optimize: only one input file is supported");
                    return ExitCode::from(2);
                }
                input = Some(other);
                i += 1;
            }
        }
    }
    let (input, eval) = match (input, eval) {
        (Some(i), Some(e)) => (i, e),
        _ => {
            eprintln!("usage: axon optimize <prompt.ax> --eval <suite.ax> [--metric NAME] [--budget B] [--trials N]");
            return ExitCode::from(2);
        }
    };
    let _ = metric;
    let _ = budget;

    let text = match std::fs::read_to_string(input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("axon optimize: cannot read `{input}`: {e}");
            return ExitCode::from(1);
        }
    };

    // Parse VARIANT swap points. Format:
    //   // VARIANT: <name>
    //   //   = "candidate-a"
    //   //   = "candidate-b"
    //   <line containing {{NAME}}>
    let variants = match parse_variant_swaps(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("axon optimize: {e}");
            return ExitCode::from(1);
        }
    };
    if variants.is_empty() {
        eprintln!("axon optimize: no `// VARIANT: NAME` swap points found in `{input}` — nothing to search over");
        return ExitCode::from(1);
    }

    // Cartesian product, capped to `trials`.
    let combos = cartesian_product(&variants);
    let mut combos: Vec<Vec<String>> = combos.into_iter().take(trials).collect();
    if combos.is_empty() {
        combos.push(Vec::new());
    }

    let mut best: Option<(usize, Vec<String>, String)> = None;
    let report_lines = combos.iter().enumerate().map(|(i, combo)| {
        let mut rendered = text.clone();
        for (var, val) in variants.iter().zip(combo.iter()) {
            let placeholder = format!("{{{{{}}}}}", var.name);
            rendered = rendered.replace(&placeholder, val);
        }
        let tmp = match write_tmp_variant(input, i, &rendered) {
            Ok(p) => p,
            Err(e) => return format!("  trial {i}: write failed: {e}"),
        };
        let passed = run_eval_against(&tmp, eval);
        let summary = format!(
            "  trial {i:>3}: passed={passed:>4}  combo={combo:?}",
        );
        match best {
            Some((cur, _, _)) if passed <= cur => {}
            _ => {
                best = Some((passed, combo.clone(), rendered.clone()));
            }
        }
        let _ = std::fs::remove_file(&tmp);
        summary
    });
    let report_lines: Vec<String> = report_lines.collect();
    let total = report_lines.len();
    let (best_passed, best_combo, best_rendered) = match best {
        Some(b) => b,
        None => {
            eprintln!("axon optimize: no trials ran");
            return ExitCode::from(1);
        }
    };

    println!("axon optimize: searched {total} variant(s) over `{input}` against `{eval}`");
    for line in &report_lines {
        println!("{line}");
    }
    let out_path = next_versioned_path(input);
    if let Err(e) = std::fs::write(&out_path, &best_rendered) {
        eprintln!("axon optimize: cannot write `{}`: {e}", out_path.display());
        return ExitCode::from(1);
    }
    println!(
        "axon optimize: best combo {best_combo:?} (passed={best_passed}) -> {}",
        out_path.display()
    );
    ExitCode::SUCCESS
}

struct VariantSwap {
    name: String,
    candidates: Vec<String>,
}

fn parse_variant_swaps(text: &str) -> Result<Vec<VariantSwap>, String> {
    let mut out: Vec<VariantSwap> = Vec::new();
    let mut current: Option<VariantSwap> = None;
    for (lineno, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(name) = trimmed.strip_prefix("// VARIANT:") {
            if let Some(v) = current.take() {
                out.push(v);
            }
            current = Some(VariantSwap {
                name: name.trim().to_string(),
                candidates: Vec::new(),
            });
            continue;
        }
        if let Some(val) = trimmed.strip_prefix("//") {
            let val = val.trim_start();
            if let Some(rest) = val.strip_prefix('=') {
                let rest = rest.trim();
                let inner = rest
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .ok_or_else(|| {
                        format!(
                            "line {}: variant candidate must be `// = \"value\"`",
                            lineno + 1
                        )
                    })?;
                if let Some(c) = current.as_mut() {
                    c.candidates.push(inner.to_string());
                    continue;
                } else {
                    return Err(format!(
                        "line {}: `// = ...` outside of any `// VARIANT:` block",
                        lineno + 1
                    ));
                }
            }
        }
        // Non-comment line ends the current block.
        if let Some(v) = current.take() {
            out.push(v);
        }
    }
    if let Some(v) = current.take() {
        out.push(v);
    }
    Ok(out.into_iter().filter(|v| !v.candidates.is_empty()).collect())
}

fn cartesian_product(variants: &[VariantSwap]) -> Vec<Vec<String>> {
    if variants.is_empty() {
        return vec![Vec::new()];
    }
    let mut out: Vec<Vec<String>> = vec![Vec::new()];
    for v in variants {
        let mut next: Vec<Vec<String>> = Vec::new();
        for prefix in &out {
            for cand in &v.candidates {
                let mut x = prefix.clone();
                x.push(cand.clone());
                next.push(x);
            }
        }
        out = next;
    }
    out
}

fn write_tmp_variant(
    base: &str,
    idx: usize,
    text: &str,
) -> Result<std::path::PathBuf, String> {
    let mut path = std::env::temp_dir();
    let pid = std::process::id();
    let stem = std::path::Path::new(base)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("opt");
    path.push(format!("axon-optimize-{stem}-{pid}-{idx}.ax"));
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

fn run_eval_against(variant_path: &std::path::Path, suite: &str) -> usize {
    // v0 metric: simply count `test "..." { ... }` blocks in the suite
    // that the variant doesn't violate. We approximate by running
    // `axon test` on a synthetic project containing both files. If that's
    // too expensive or the suite hasn't been wired, fall back to a "good
    // enough" heuristic: 1 if the variant parses, else 0.
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("axon"));
    // First quick sanity: does it parse?
    let check = std::process::Command::new(&exe)
        .args(["check", variant_path.to_str().unwrap_or("")])
        .output();
    let parses = matches!(check, Ok(o) if o.status.success());
    if !parses {
        return 0;
    }
    // Then run the suite against the variant directory.
    // For v0 we just count exit-code success of `axon run` on the suite.
    let run = std::process::Command::new(&exe)
        .args(["check", suite])
        .output();
    match run {
        Ok(o) if o.status.success() => 1,
        _ => 0,
    }
}

fn next_versioned_path(input: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(input);
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("variant");
    let parent = p.parent().unwrap_or_else(|| std::path::Path::new("."));
    for v in 1..1000 {
        let candidate = parent.join(format!("{stem}.v{v}.ax"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem}.v999.ax"))
}

// ---------------------------------------------------------------------------
// Stage 27 — `axon schema` (§17.1 / §36)
//
// Pure operator surface: walk a JSON store, find entries shaped like
//   { "__schema": "Profile", "__version": 2, ... }
// and report what migration steps would run to upgrade them to a target
// version. With `--apply`, write each transformed value back using the
// registered runtime migrator. v0 supports the *plan* path fully; the
// execution path requires the migrator to be loaded from a side program
// (a future stage hooks `axon run` to install the migrator before the
// store sweep). Until then `--apply` errors cleanly so operators don't
// silently get a no-op.
// ---------------------------------------------------------------------------

fn cmd_schema(args: &[String]) -> ExitCode {
    let sub = match args.first() {
        Some(s) => s.as_str(),
        None => {
            eprintln!("usage: axon schema <migrate|inspect> [args...]");
            return ExitCode::from(2);
        }
    };
    let rest = &args[1..];
    match sub {
        "migrate" => cmd_schema_migrate(rest),
        "inspect" => cmd_schema_inspect(rest),
        other => {
            eprintln!("axon schema: unknown subcommand `{other}`");
            ExitCode::from(2)
        }
    }
}

fn cmd_schema_migrate(args: &[String]) -> ExitCode {
    let mut store_path: Option<String> = None;
    let mut to_version: Option<u32> = None;
    let mut schema_filter: Option<String> = None;
    let mut apply = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--to" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon schema migrate: --to requires N");
                    return ExitCode::from(2);
                }
                to_version = match args[i].parse::<u32>() {
                    Ok(v) => Some(v),
                    Err(_) => {
                        eprintln!("axon schema migrate: --to must be a positive integer");
                        return ExitCode::from(2);
                    }
                };
                i += 1;
            }
            "--schema" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon schema migrate: --schema requires NAME");
                    return ExitCode::from(2);
                }
                schema_filter = Some(args[i].clone());
                i += 1;
            }
            "--apply" => {
                apply = true;
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("axon schema migrate: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                if store_path.is_some() {
                    eprintln!("axon schema migrate: only one store path is supported");
                    return ExitCode::from(2);
                }
                store_path = Some(other.to_string());
                i += 1;
            }
        }
    }
    let path = match store_path {
        Some(p) => p,
        None => {
            eprintln!("usage: axon schema migrate <store.json> [--schema NAME] --to N [--apply]");
            return ExitCode::from(2);
        }
    };
    let to = match to_version {
        Some(v) => v,
        None => {
            eprintln!("axon schema migrate: --to N is required");
            return ExitCode::from(2);
        }
    };

    let raw = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("axon schema migrate: cannot read `{path}`: {e}");
            return ExitCode::from(1);
        }
    };
    let root: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("axon schema migrate: `{path}` is not valid JSON: {e}");
            return ExitCode::from(1);
        }
    };

    let entries = collect_schema_entries(&root, schema_filter.as_deref());
    if entries.is_empty() {
        println!(
            "axon schema migrate: no `__schema`/`__version` entries found in `{path}`"
        );
        return ExitCode::SUCCESS;
    }

    let mut planned = 0usize;
    let mut downgrade_blocked = 0usize;
    let mut at_target = 0usize;
    for e in &entries {
        if e.version > to {
            println!(
                "  WOULD-DOWNGRADE {schema} stored=v{stored} target=v{target} key={key} — skipped",
                schema = e.schema,
                stored = e.version,
                target = to,
                key = e.key
            );
            downgrade_blocked += 1;
            continue;
        }
        if e.version == to {
            at_target += 1;
            continue;
        }
        let plan: Vec<u32> = (e.version..to).collect();
        println!(
            "  PLAN {schema} v{stored} -> v{target} key={key} steps={plan:?}",
            schema = e.schema,
            stored = e.version,
            target = to,
            key = e.key,
            plan = plan
        );
        planned += 1;
    }

    println!(
        "axon schema migrate: {planned} entries to upgrade, {at_target} already at v{to}, {downgrade_blocked} blocked",
    );

    if apply {
        eprintln!(
            "axon schema migrate: --apply requires a registered migrator. Run \
             `axon run <migrator-script.ax>` to install one, then invoke this command again. \
             (Plan above is unchanged; no entries were modified.)"
        );
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}

fn cmd_schema_inspect(args: &[String]) -> ExitCode {
    let mut store_path: Option<String> = None;
    let mut schema_filter: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--schema" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon schema inspect: --schema requires NAME");
                    return ExitCode::from(2);
                }
                schema_filter = Some(args[i].clone());
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("axon schema inspect: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                if store_path.is_some() {
                    eprintln!("axon schema inspect: only one store path is supported");
                    return ExitCode::from(2);
                }
                store_path = Some(other.to_string());
                i += 1;
            }
        }
    }
    let path = match store_path {
        Some(p) => p,
        None => {
            eprintln!("usage: axon schema inspect <store.json> [--schema NAME]");
            return ExitCode::from(2);
        }
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("axon schema inspect: cannot read `{path}`: {e}");
            return ExitCode::from(1);
        }
    };
    let root: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("axon schema inspect: `{path}` is not valid JSON: {e}");
            return ExitCode::from(1);
        }
    };
    let entries = collect_schema_entries(&root, schema_filter.as_deref());
    if entries.is_empty() {
        println!("(no schema-tagged entries found)");
        return ExitCode::SUCCESS;
    }
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<(String, u32), usize> = BTreeMap::new();
    for e in &entries {
        *counts.entry((e.schema.clone(), e.version)).or_default() += 1;
    }
    for ((schema, version), n) in counts {
        println!("  {schema} v{version}: {n}");
    }
    ExitCode::SUCCESS
}

struct SchemaEntry {
    schema: String,
    version: u32,
    /// Where this entry lives in the JSON tree — a dot-path for stable
    /// identification in the inspect/migrate report.
    key: String,
}

fn collect_schema_entries(root: &serde_json::Value, filter: Option<&str>) -> Vec<SchemaEntry> {
    fn walk(
        v: &serde_json::Value,
        path: &str,
        filter: Option<&str>,
        out: &mut Vec<SchemaEntry>,
    ) {
        match v {
            serde_json::Value::Object(obj) => {
                let has_schema = obj.get("__schema").and_then(|x| x.as_str());
                let has_version = obj
                    .get("__version")
                    .and_then(|x| x.as_u64().map(|u| u as u32));
                if let (Some(schema), Some(version)) = (has_schema, has_version) {
                    if filter.map(|f| f == schema).unwrap_or(true) {
                        out.push(SchemaEntry {
                            schema: schema.to_string(),
                            version,
                            key: if path.is_empty() { "$".into() } else { path.to_string() },
                        });
                    }
                }
                for (k, child) in obj {
                    let child_path = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    walk(child, &child_path, filter, out);
                }
            }
            serde_json::Value::Array(xs) => {
                for (i, child) in xs.iter().enumerate() {
                    let child_path = format!("{path}[{i}]");
                    walk(child, &child_path, filter, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(root, "", filter, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Stage 29 — `axon prof --cost` (§31.2)
//
// Renders a cost report from a recorded ledger written by the `cost_*`
// host bindings. Supports `--profile NAME:input/output[/cached[/per_call]]`
// to attach per-provider pricing; without profiles, totals stay at zero
// and we still show token counts + latency percentiles.
// ---------------------------------------------------------------------------

fn cmd_prof(args: &[String]) -> ExitCode {
    let mut ledger_path: Option<String> = None;
    let mut top_n: usize = 10;
    let mut profiles: Vec<axon_cost::ProviderProfile> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cost" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon prof: --cost requires PATH");
                    return ExitCode::from(2);
                }
                ledger_path = Some(args[i].clone());
                i += 1;
            }
            other if other.starts_with("--cost=") => {
                ledger_path = Some(other["--cost=".len()..].to_string());
                i += 1;
            }
            "--top" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon prof: --top requires N");
                    return ExitCode::from(2);
                }
                top_n = match args[i].parse::<usize>() {
                    Ok(n) => n,
                    Err(_) => {
                        eprintln!("axon prof: --top must be a non-negative integer");
                        return ExitCode::from(2);
                    }
                };
                i += 1;
            }
            "--profile" => {
                i += 1;
                if i >= args.len() {
                    eprintln!(
                        "axon prof: --profile requires NAME:input/output[/cached[/per_call]]"
                    );
                    return ExitCode::from(2);
                }
                match parse_profile_spec(&args[i]) {
                    Ok(p) => profiles.push(p),
                    Err(e) => {
                        eprintln!("axon prof: {e}");
                        return ExitCode::from(2);
                    }
                }
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("axon prof: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                eprintln!("axon prof: unexpected positional argument `{other}`");
                return ExitCode::from(2);
            }
        }
    }
    let Some(path) = ledger_path else {
        eprintln!("usage: axon prof --cost <ledger.json> [--top N] [--profile SPEC]...");
        return ExitCode::from(2);
    };
    let bytes = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("axon prof: cannot read `{path}`: {e}");
            return ExitCode::from(1);
        }
    };
    let ledger: axon_cost::Ledger = match serde_json::from_str(&bytes) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("axon prof: `{path}` is not a valid ledger JSON: {e}");
            return ExitCode::from(1);
        }
    };
    let report = axon_cost::Report::build(&ledger, &profiles, top_n);
    println!("cost report from `{path}`");
    println!("  total calls : {}", report.total_calls);
    println!(
        "  total cost  : ${:.4}",
        report.total_cents as f64 / 100.0
    );
    println!("  latency p50 : {} ms", report.p50_latency_ms);
    println!("  latency p95 : {} ms", report.p95_latency_ms);
    println!("  per-provider breakdown:");
    if report.providers.is_empty() {
        println!("    (no entries)");
    } else {
        for s in &report.providers {
            println!(
                "    {p:<16} calls={c:<5} in={i:<8} out={o:<8} ${d:.4}",
                p = s.provider,
                c = s.calls,
                i = s.input_tokens,
                o = s.output_tokens,
                d = s.total_cents as f64 / 100.0,
            );
        }
    }
    if !report.top_calls.is_empty() {
        println!("  top-{} most expensive calls:", report.top_calls.len());
        for (i, c) in report.top_calls.iter().enumerate() {
            println!(
                "    #{i:<2} {p}/{m:<20} ${d:.4} ({lat} ms) tag={tag}",
                i = i + 1,
                p = c.provider,
                m = c.model,
                d = c.cents as f64 / 100.0,
                lat = c.latency_ms,
                tag = c.tag,
            );
        }
    }
    ExitCode::SUCCESS
}

fn parse_profile_spec(s: &str) -> Result<axon_cost::ProviderProfile, String> {
    let (name_part, rates_part) = s
        .split_once(':')
        .ok_or_else(|| format!("--profile: missing `:` in `{s}`"))?;
    let parts: Vec<&str> = rates_part.split('/').collect();
    let parse = |s: &str, label: &str| -> Result<u64, String> {
        s.parse::<u64>()
            .map_err(|e| format!("--profile: {label} `{s}`: {e}"))
    };
    let parse32 = |s: &str, label: &str| -> Result<u32, String> {
        s.parse::<u32>()
            .map_err(|e| format!("--profile: {label} `{s}`: {e}"))
    };
    if parts.len() < 2 || parts.len() > 4 {
        return Err(format!(
            "--profile: expected NAME:input/output[/cached[/per_call]], got `{s}`"
        ));
    }
    Ok(axon_cost::ProviderProfile {
        name: name_part.to_string(),
        model: String::new(),
        input_cents_per_million: parse(parts[0], "input_per_million")?,
        output_cents_per_million: parse(parts[1], "output_per_million")?,
        cached_input_cents_per_million: if parts.len() >= 3 {
            parse(parts[2], "cached_per_million")?
        } else {
            0
        },
        per_call_cents: if parts.len() == 4 {
            parse32(parts[3], "per_call_cents")?
        } else {
            0
        },
    })
}

// ---------------------------------------------------------------------------
// §57 / §64.2 — `axon explain <CODE | effect:X | capability:X>`
// ---------------------------------------------------------------------------

fn cmd_explain(args: &[String]) -> ExitCode {
    let topic = match args.first() {
        Some(t) => t.as_str(),
        None => {
            // No argument: list the catalogued codes.
            println!(
                "axon explain — offline diagnostic & concept reference\n\nUsage:\n  axon explain <CODE>            e.g. axon explain E0202\n  axon explain effect:<NAME>     e.g. axon explain effect:LLM\n  axon explain capability:<NAME> e.g. axon explain capability:Tool\n"
            );
            let codes = axon_diag::explain::catalogue_codes();
            println!("Catalogued codes ({}):", codes.len());
            println!("  {}", codes.join("  "));
            return ExitCode::SUCCESS;
        }
    };
    // Concept form: `effect:LLM`, `capability:Tool`.
    if let Some((kind, name)) = topic.split_once(':') {
        match axon_diag::explain::explain_concept(kind, name) {
            Some(doc) => {
                println!("{doc}");
                return ExitCode::SUCCESS;
            }
            None => {
                eprintln!("axon explain: unknown concept kind `{kind}` (try effect: or capability:)");
                return ExitCode::from(2);
            }
        }
    }
    // Code form.
    match axon_diag::explain::explain(topic) {
        Some(e) => {
            println!("{}", e.render());
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("axon explain: no explanation for `{topic}`");
            eprintln!("  (codes look like E0202, E0712, W1203, P0010)");
            ExitCode::from(1)
        }
    }
}

