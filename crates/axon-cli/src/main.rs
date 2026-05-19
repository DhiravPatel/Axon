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
                          Start an HTTP/1.1 server. POST /invoke dispatches\n\
                          to the named handler with the request body; GET\n\
                          /healthz and /readyz return JSON health status.\n\
                          With --tls-* flags, terminate TLS via rustls.\n\
                          SIGINT/SIGTERM triggers graceful shutdown with\n\
                          in-flight handler drain.\n\
           login  <provider> [--vault PATH] [--key VALUE]\n\
                          Save an API key to the local vault (mode 0600 on\n\
                          Unix). Reads --key, then $PROVIDER_API_KEY env,\n\
                          then prompts on stdin.\n\
           pkg    <subcmd>  Manage dependencies declared in `axon.toml`.\n\
                          Subcommands: `list`, `add NAME --path P`,\n\
                          `remove NAME`, `audit`.\n\
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
    let mut tls_cert: Option<String> = None;
    let mut tls_key: Option<String> = None;
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
fn cmd_trace(args: &[String]) -> ExitCode {
    let path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: axon trace <file.jsonl>");
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

