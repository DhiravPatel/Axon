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

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = match args.next() {
        Some(c) => c,
        None => {
            print_help();
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
        "check" => match args.next() {
            Some(path) => cmd_check(&path),
            None => {
                eprintln!("usage: axon check <file>");
                ExitCode::from(2)
            }
        },
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
                          Start an HTTP/1.1 server. POST /invoke dispatches\n\
                          to the named handler with the request body; GET\n\
                          /healthz and /readyz return JSON health status.\n\
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
                            (no flag)                  Grant the standard default set.\n\
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
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .unwrap_or(".");
    let path = std::path::Path::new(path);
    let project = match axon_project::LoadedProject::load(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    if !project.diagnostics.is_empty() {
        emit_project_diagnostics_via_registry(&project.diagnostics, &project.sources);
        return ExitCode::from(1);
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
        return ExitCode::SUCCESS;
    }

    println!("running {} test{}", tests.len(), if tests.len() == 1 { "" } else { "s" });
    let start = std::time::Instant::now();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());

    for t in &tests {
        let mut interp = axon_runtime::Interpreter::with_caps(caps.clone());
        host::install(&interp);
        interp.load_program(&project.merged);
        let closure = axon_runtime::Closure::new(
            Some(format!("test:{}", t.name)),
            Vec::new(),
            axon_runtime::ClosureBody::Block(t.body.clone()),
            interp.globals.clone(),
            t.span,
            Some(Vec::new()),
        );
        let result = interp.call_value(
            &axon_runtime::Value::Fn(std::rc::Rc::new(closure)),
            &[],
            t.span,
        );
        match result {
            Ok(_) => {
                println!("  ok   `{}`", t.name);
                passed += 1;
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
            }
            Err(other) => {
                failed += 1;
                println!("  FAIL `{}` — unexpected control flow: {other:?}", t.name);
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
    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
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
    let mut trace_path: Option<String> = None;
    let mut record_path: Option<String> = None;
    let mut replay_path: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--isolated" => {
                isolated = true;
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
    let path = match path {
        Some(p) => p,
        None => {
            eprintln!("usage: axon run [--with E,...] [--isolated] [--vm] <file|dir>");
            return ExitCode::from(2);
        }
    };
    let path_buf = std::path::Path::new(path);
    let project = match axon_project::LoadedProject::load(path_buf) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
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
        let exit = match interp.run_main() {
            Ok(_) => ExitCode::SUCCESS,
            Err(err) => {
                let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
                eprint!("{}", err.render(&file, use_color));
                ExitCode::from(1)
            }
        };
        if let (Some(tp), Some(tracer)) = (trace_path.as_ref(), interp.take_tracer()) {
            let _ = std::fs::write(tp, tracer.to_jsonl());
        }
        if let (Some(rp), Some(rec)) = (record_path.as_ref(), interp.take_recording()) {
            let json = rec.to_json();
            let _ = std::fs::write(rp, serde_json::to_string_pretty(&json).unwrap_or_default());
        }
        exit
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

fn cmd_check(path: &str) -> ExitCode {
    let Some(file) = read_or_die(path) else {
        return ExitCode::from(1);
    };
    let (program, parse_diags) = axon_parser::parse(&file);
    if !parse_diags.is_empty() {
        emit_diagnostics(&parse_diags, &file);
        return ExitCode::from(1);
    }
    let (ctx, type_diags) = axon_tyck::check(&file, &program);
    if !type_diags.is_empty() {
        emit_diagnostics(&type_diags, &file);
        return ExitCode::from(1);
    }
    println!("type-checked {} item(s) successfully", ctx.len());
    ExitCode::SUCCESS
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
    let file = match file {
        Some(f) => f,
        None => {
            eprintln!("usage: axon serve <file> [--listen ADDR] [--handler NAME]");
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

    // Programmatically call serve_run(listen, callee). The simpler path
    // would be to expose a Rust-side `serve` directly, but routing
    // through the same NativeExt the language uses means there's exactly
    // one server code path.
    let serve_native = match interp.globals.lookup("serve_run") {
        Some(v) => v,
        None => {
            eprintln!("axon serve: serve_run binding missing (internal error)");
            return ExitCode::from(1);
        }
    };
    let result = interp.call_value(
        &serve_native,
        &[
            axon_runtime::Value::String(std::rc::Rc::new(listen)),
            callee,
        ],
        axon_diag::Span::DUMMY,
    );
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
