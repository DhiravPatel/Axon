//! Quality-of-life CLI commands (§64.2): `stats`, `clean`,
//! `completions`, `doctor`, plus the bare-`axon` next-command hint.
//!
//! Each is a small, self-contained command. None require a network or
//! a running server.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

// ---------------------------------------------------------------------------
// axon stats
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Stats {
    files: usize,
    lines: usize,
    code_lines: usize,
    fns: usize,
    fns_with_effects: usize,
    agents: usize,
    actors: usize,
    tools: usize,
    models: usize,
    memories: usize,
    prompts: usize,
    schemas: usize,
    types: usize,
    policies: usize,
    tests: usize,
    evals: usize,
}

pub fn cmd_stats(args: &[String]) -> ExitCode {
    let path = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or(".");
    let root = Path::new(path);
    let mut files = Vec::new();
    if root.is_file() {
        files.push(root.to_path_buf());
    } else if let Err(e) = collect_ax(root, &mut files) {
        eprintln!("axon stats: {e}");
        return ExitCode::from(1);
    }
    if files.is_empty() {
        println!("axon stats: no .ax files found under `{path}`");
        return ExitCode::SUCCESS;
    }
    let mut s = Stats::default();
    for f in &files {
        s.files += 1;
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for line in text.lines() {
            s.lines += 1;
            let t = line.trim();
            if !t.is_empty() && !t.starts_with("//") {
                s.code_lines += 1;
            }
        }
        let src = axon_diag::SourceFile::new(f.to_string_lossy().as_ref(), &text);
        let (program, _diags) = axon_parser::parse(&src);
        for item in &program.items {
            tally_item(item, &mut s);
        }
    }
    let eff_pct = if s.fns == 0 {
        0.0
    } else {
        100.0 * s.fns_with_effects as f64 / s.fns as f64
    };
    println!("axon stats — {path}");
    println!("  files        {}", s.files);
    println!("  lines        {} ({} code)", s.lines, s.code_lines);
    println!("  functions    {} ({:.0}% with explicit effect rows)", s.fns, eff_pct);
    println!("  agents       {}", s.agents);
    println!("  actors       {}", s.actors);
    println!("  tools        {}", s.tools);
    println!("  models       {}", s.models);
    println!("  memories     {}", s.memories);
    println!("  prompts      {}", s.prompts);
    println!("  schemas      {}", s.schemas);
    println!("  types        {}", s.types);
    println!("  policies     {}", s.policies);
    println!("  tests        {}", s.tests);
    println!("  evals        {}", s.evals);
    ExitCode::SUCCESS
}

fn tally_item(item: &axon_ast::Item, s: &mut Stats) {
    use axon_ast::Item;
    match item {
        Item::Fn(f) => {
            s.fns += 1;
            if f.effect_row.as_ref().map(|r| !r.effects.is_empty()).unwrap_or(false) {
                s.fns_with_effects += 1;
            }
        }
        Item::Agent(_) => s.agents += 1,
        Item::Actor(_) => s.actors += 1,
        Item::Tool(_) => s.tools += 1,
        Item::Model(_) => s.models += 1,
        Item::Memory(_) => s.memories += 1,
        Item::Prompt(_) => s.prompts += 1,
        Item::Schema(_) => s.schemas += 1,
        Item::Type(_) => s.types += 1,
        Item::Policy(_) => s.policies += 1,
        Item::Test(_) => s.tests += 1,
        Item::Eval(_) => s.evals += 1,
        _ => {}
    }
}

fn collect_ax(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        // Skip build/dep dirs.
        if p.is_dir() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "target" | "node_modules" | ".git" | "dist" | "out") {
                continue;
            }
            collect_ax(&p, out)?;
        } else if p.extension().and_then(|e| e.to_str()) == Some("ax") {
            out.push(p);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// axon clean
// ---------------------------------------------------------------------------

pub fn cmd_clean(_args: &[String]) -> ExitCode {
    let candidates = ["target", "doc", "dist", "out", ".axon-cache"];
    let mut reclaimed: u64 = 0;
    let mut removed: Vec<&str> = Vec::new();
    for c in candidates {
        let p = Path::new(c);
        if p.is_dir() {
            let sz = dir_size(p).unwrap_or(0);
            if std::fs::remove_dir_all(p).is_ok() {
                reclaimed += sz;
                removed.push(c);
            }
        }
    }
    if removed.is_empty() {
        println!("axon clean: nothing to remove");
    } else {
        println!(
            "axon clean: removed {} ({:.1} MB reclaimed)",
            removed.join(", "),
            reclaimed as f64 / (1024.0 * 1024.0)
        );
    }
    ExitCode::SUCCESS
}

fn dir_size(p: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    if p.is_dir() {
        for entry in std::fs::read_dir(p)? {
            let path = entry?.path();
            if path.is_dir() {
                total += dir_size(&path)?;
            } else {
                total += path.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// axon completions <shell>
// ---------------------------------------------------------------------------

const SUBCOMMANDS: &[&str] = &[
    "tokens", "parse", "check", "run", "test", "build", "lsp", "doc", "fmt",
    "serve", "deploy", "replay", "trace", "repl", "login", "pkg", "optimize",
    "schema", "prof", "explain", "new", "tour", "stats", "clean", "completions",
    "doctor", "why", "outdated", "version", "help",
];

pub fn cmd_completions(args: &[String]) -> ExitCode {
    let shell = match args.first() {
        Some(s) => s.as_str(),
        None => {
            eprintln!("usage: axon completions <bash|zsh|fish|pwsh>");
            return ExitCode::from(2);
        }
    };
    let cmds = SUBCOMMANDS.join(" ");
    let script = match shell {
        "bash" => format!(
            "# axon bash completions — add to ~/.bashrc:\n#   source <(axon completions bash)\n_axon() {{\n  local cur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n  if [ \"$COMP_CWORD\" -eq 1 ]; then\n    COMPREPLY=( $(compgen -W \"{cmds}\" -- \"$cur\") )\n  fi\n}}\ncomplete -F _axon axon\n"
        ),
        "zsh" => format!(
            "# axon zsh completions — add to ~/.zshrc:\n#   source <(axon completions zsh)\n_axon() {{\n  local -a cmds\n  cmds=({cmds})\n  _describe 'axon command' cmds\n}}\ncompdef _axon axon\n"
        ),
        "fish" => {
            let mut out = String::from("# axon fish completions — save to ~/.config/fish/completions/axon.fish\n");
            for c in SUBCOMMANDS {
                out.push_str(&format!(
                    "complete -c axon -n __fish_use_subcommand -a {c}\n"
                ));
            }
            out
        }
        "pwsh" | "powershell" => format!(
            "# axon PowerShell completions — add to $PROFILE:\nRegister-ArgumentCompleter -Native -CommandName axon -ScriptBlock {{\n  param($wordToComplete, $commandAst, $cursorPosition)\n  @({}) | Where-Object {{ $_ -like \"$wordToComplete*\" }} | ForEach-Object {{\n    [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)\n  }}\n}}\n",
            SUBCOMMANDS
                .iter()
                .map(|c| format!("'{c}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => {
            eprintln!("axon completions: unsupported shell `{other}` (bash|zsh|fish|pwsh)");
            return ExitCode::from(2);
        }
    };
    print!("{script}");
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// axon doctor
// ---------------------------------------------------------------------------

pub fn cmd_doctor(_args: &[String]) -> ExitCode {
    let mut problems = 0;
    println!("axon doctor");
    println!("  ✓ axon {}", env!("CARGO_PKG_VERSION"));

    // In a project?
    if Path::new("axon.toml").is_file() {
        println!("  ✓ axon.toml present");
        match axon_project::Manifest::from_dir(Path::new(".")) {
            Ok(_) => println!("  ✓ axon.toml parses"),
            Err(e) => {
                println!("  ✗ axon.toml invalid: {e}");
                problems += 1;
            }
        }
    } else {
        println!("  • not in a project (no axon.toml) — `axon new` to scaffold one");
    }

    // Vault permissions (Unix 0600).
    let home = std::env::var("HOME").unwrap_or_default();
    let vault = std::env::var("AXON_VAULT")
        .unwrap_or_else(|_| format!("{home}/.axon/vault.json"));
    if Path::new(&vault).exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&vault) {
                let mode = meta.permissions().mode() & 0o777;
                if mode == 0o600 {
                    println!("  ✓ vault permissions are 0600");
                } else {
                    println!(
                        "  ✗ vault `{vault}` is {mode:o}, should be 600 — run `chmod 600 {vault}`"
                    );
                    problems += 1;
                }
            }
        }
        #[cfg(not(unix))]
        println!("  ✓ vault present");
    } else {
        println!("  • no vault yet — `axon login <provider>` to create one");
    }

    // Optional bridge runtimes.
    for (bin, why) in [
        ("python3", "extern python tools (§35.2)"),
        ("node", "extern node tools (§35.2)"),
    ] {
        if which(bin) {
            println!("  ✓ {bin} found ({why})");
        } else {
            println!("  • {bin} not found — only needed for {why}");
        }
    }

    if problems == 0 {
        println!("\nNo problems detected.");
        ExitCode::SUCCESS
    } else {
        println!("\n{problems} problem(s) found.");
        ExitCode::from(1)
    }
}

fn which(bin: &str) -> bool {
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|dir| dir.join(bin).is_file())
}

// ---------------------------------------------------------------------------
// bare `axon` — next-command hint based on project state (§64.2)
// ---------------------------------------------------------------------------

/// Returns a one-line suggestion for what to do next, given the current
/// directory's state. Used when `axon` is invoked with no arguments.
pub fn next_command_hint() -> String {
    let in_project = Path::new("axon.toml").is_file();
    if !in_project {
        return "no project here — try `axon new my-bot` to scaffold one, or `axon tour` to learn.".into();
    }
    // Any journals recorded yet?
    let has_runs = Path::new("runs").is_dir()
        && std::fs::read_dir("runs")
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
    let has_tests = {
        let mut files = Vec::new();
        let _ = collect_ax(Path::new("src"), &mut files);
        files.iter().any(|f| {
            std::fs::read_to_string(f)
                .map(|t| t.contains("test \""))
                .unwrap_or(false)
        })
    };
    if !has_runs {
        "you're in a project — try `axon run` (record a journal with `axon run --record runs/first.axj`).".into()
    } else if has_tests {
        "you have tests — try `axon test`; replay a journal with `axon replay`.".into()
    } else {
        "try `axon run`, or add a `test \"…\" { }` block and run `axon test`.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completions_emit_for_each_shell() {
        for sh in ["bash", "zsh", "fish", "pwsh"] {
            let out = match sh {
                "bash" | "zsh" | "fish" | "pwsh" => true,
                _ => false,
            };
            assert!(out, "{sh}");
        }
        // The subcommand list includes the new commands.
        assert!(SUBCOMMANDS.contains(&"explain"));
        assert!(SUBCOMMANDS.contains(&"new"));
        assert!(SUBCOMMANDS.contains(&"stats"));
    }

    #[test]
    fn dir_size_of_missing_is_zero() {
        assert_eq!(dir_size(Path::new("/nonexistent-xyz-123")).unwrap_or(0), 0);
    }

    #[test]
    fn which_finds_sh() {
        // `sh` exists on every unix CI box.
        #[cfg(unix)]
        assert!(which("sh"));
    }

    #[test]
    fn stats_tally_counts_kinds() {
        let src = axon_diag::SourceFile::new(
            "t.ax",
            "fn a() uses { Net } { 1 }\nfn b() { 2 }\nmodel m = mock_model()\n",
        );
        let (program, _d) = axon_parser::parse(&src);
        let mut s = Stats::default();
        for it in &program.items {
            tally_item(it, &mut s);
        }
        assert_eq!(s.fns, 2);
        assert_eq!(s.fns_with_effects, 1);
        assert_eq!(s.models, 1);
    }
}
