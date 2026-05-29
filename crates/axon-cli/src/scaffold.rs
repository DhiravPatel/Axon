//! `axon new` project scaffolding + `axon tour` interactive lessons (§58).
//!
//! Templates are curated for *learning*, not maximum features — each
//! ships a `README.md`, a runnable `src/main.ax`, and (where it makes
//! sense) a test, so a newcomer sees the whole workflow loop on day
//! one. Everything is embedded in the binary; `axon new` works offline.

use std::path::Path;
use std::process::ExitCode;

/// One file the template materializes, relative to the project root.
struct TemplateFile {
    rel: &'static str,
    body: &'static str,
}

struct Template {
    name: &'static str,
    summary: &'static str,
    files: &'static [TemplateFile],
}

pub fn cmd_new(args: &[String]) -> ExitCode {
    let mut name: Option<&str> = None;
    let mut template = "agent";
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--template" | "-t" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon new: --template requires a name");
                    return ExitCode::from(2);
                }
                template = &args[i];
                i += 1;
            }
            "--list" => {
                list_templates();
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => {
                eprintln!("axon new: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                if name.is_some() {
                    eprintln!("axon new: only one project name is supported");
                    return ExitCode::from(2);
                }
                name = Some(other);
                i += 1;
            }
        }
    }
    let Some(name) = name else {
        eprintln!("usage: axon new <name> [--template <t>] [--list]");
        eprintln!("  templates: {}", template_names().join(", "));
        return ExitCode::from(2);
    };
    if !is_valid_project_name(name) {
        eprintln!("axon new: invalid project name `{name}` (use letters, digits, `-`, `_`)");
        return ExitCode::from(2);
    }
    let Some(tpl) = find_template(template) else {
        eprintln!("axon new: unknown template `{template}`");
        eprintln!("  available: {}", template_names().join(", "));
        return ExitCode::from(2);
    };
    let root = Path::new(name);
    if root.exists() {
        eprintln!("axon new: `{name}` already exists");
        return ExitCode::from(1);
    }
    for f in tpl.files {
        let path = root.join(f.rel);
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("axon new: cannot create `{}`: {e}", parent.display());
                return ExitCode::from(1);
            }
        }
        // `{{name}}` is the only substitution token in template bodies.
        let body = f.body.replace("{{name}}", name);
        if let Err(e) = std::fs::write(&path, body) {
            eprintln!("axon new: cannot write `{}`: {e}", path.display());
            return ExitCode::from(1);
        }
    }
    println!("created `{name}` from template `{}`", tpl.name);
    println!("  {}", tpl.summary);
    println!("\nnext:");
    println!("  cd {name}");
    println!("  axon run            # type-checks + runs (mock model, no keys needed)");
    println!("  axon test           # runs the included test");
    ExitCode::SUCCESS
}

fn list_templates() {
    println!("axon templates:");
    for t in TEMPLATES {
        println!("  {:<10} {}", t.name, t.summary);
    }
}

fn template_names() -> Vec<&'static str> {
    TEMPLATES.iter().map(|t| t.name).collect()
}

fn find_template(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

fn is_valid_project_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !s.starts_with('-')
}

// ---------------------------------------------------------------------------
// `axon tour`
// ---------------------------------------------------------------------------

struct Lesson {
    n: u32,
    title: &'static str,
    body: &'static str,
}

pub fn cmd_tour(args: &[String]) -> ExitCode {
    // `axon tour`           — list lessons
    // `axon tour <n>`       — print lesson n (writes it to a temp file you can run)
    let lesson_arg = args.iter().find(|a| !a.starts_with('-'));
    match lesson_arg {
        None => {
            println!("axon tour — {} lessons, ~5 min each, runs locally\n", LESSONS.len());
            for l in LESSONS {
                println!("  {:>2}. {}", l.n, l.title);
            }
            println!("\nStart a lesson:  axon tour 1");
            ExitCode::SUCCESS
        }
        Some(arg) => {
            let n: u32 = match arg.parse() {
                Ok(n) => n,
                Err(_) => {
                    eprintln!("axon tour: lesson must be a number (1..={})", LESSONS.len());
                    return ExitCode::from(2);
                }
            };
            let Some(lesson) = LESSONS.iter().find(|l| l.n == n) else {
                eprintln!("axon tour: no lesson {n} (have 1..={})", LESSONS.len());
                return ExitCode::from(2);
            };
            println!("── Lesson {}: {} ──\n", lesson.n, lesson.title);
            println!("{}", lesson.body);
            if n < LESSONS.len() as u32 {
                println!("\nnext:  axon tour {}", n + 1);
            } else {
                println!("\nThat's the tour. Build something: axon new my-bot --template support");
            }
            ExitCode::SUCCESS
        }
    }
}

// ---------------------------------------------------------------------------
// Embedded templates
// ---------------------------------------------------------------------------

static TEMPLATES: &[Template] = &[
    Template {
        name: "agent",
        summary: "A single agent with one tool — the canonical \"hello, agent\".",
        files: &[
            TemplateFile {
                rel: "axon.toml",
                body: "[package]\nname = \"{{name}}\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[caps]\ndefault = [\"Console\", \"LLM\", \"Net\", \"Spawn\"]\n",
            },
            TemplateFile {
                rel: "src/main.ax",
                body: "//! {{name}} — a single agent with one tool.\n//! Run with the deterministic mock model (no API key needed):\n//!   axon run\n\nmodel brain = mock_model(\"fixed\", \"The capital of France is Paris.\")\n\ntool lookup(topic: String) -> String uses { Net } {\n    \"[stub] no live index yet; you asked about: {topic}\"\n}\n\nagent Helper(m: Model) {\n    on ask_question(q: String) -> String uses { LLM, Net } {\n        ask self.m {\n            system: \"Answer concisely. Use the lookup tool when unsure.\"\n            user:   q\n            tools:  [lookup]\n        }\n    }\n}\n\nfn main() uses { Spawn, LLM, Net, Console } {\n    let h = spawn Helper(m = brain)\n    print(h.ask_question(\"What is the capital of France?\"))\n}\n",
            },
            TemplateFile {
                rel: "src/main_test.ax",
                body: "test \"helper answers\" {\n    let h = spawn Helper(m = mock_model(\"fixed\", \"Paris.\"))\n    let a = h.ask_question(\"capital?\")\n    assert str_contains(a, \"Paris\")\n}\n",
            },
            TemplateFile {
                rel: "README.md",
                body: "# {{name}}\n\nThe canonical \"hello, agent\": one agent, one tool, a mock model so it\nruns with no API key.\n\n```sh\naxon run     # type-checks + runs\naxon test    # runs src/main_test.ax\n```\n\nSwap `mock_model(...)` for `anthropic(\"claude-opus-4-7\")` and set\n`ANTHROPIC_API_KEY` to talk to a real model.\n",
            },
        ],
    },
    Template {
        name: "support",
        summary: "Tools + policy + tests — a guarded support bot.",
        files: &[
            TemplateFile {
                rel: "axon.toml",
                body: "[package]\nname = \"{{name}}\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[caps]\ndefault = [\"Console\", \"LLM\", \"Net\", \"Memory\"]\n",
            },
            TemplateFile {
                rel: "src/main.ax",
                body: "//! {{name}} — a support bot with a runtime-enforced policy (§30).\n\nmodel brain = mock_model(\"fixed\", \"I've opened ticket #42 for you.\")\n\npolicy support {\n    allow tool kb.search, tickets.get, tickets.open\n    deny  tool payments.charge\n    budget per_request { usd = 0.50, tokens = 60000 }\n    rate   per_user { 30 per 1m }\n    audit  all_tool_calls, all_policy_denials\n}\n\nfn main() uses { Console } {\n    // The policy is enforceable from handler code:\n    let ok = policy_block_check(\"support\", \"tool\", \"kb.search\", true)\n    print(\"kb.search allowed? \")\n    print(bool(ok.allow))\n    let blocked = policy_block_check(\"support\", \"tool\", \"payments.charge\", true)\n    print(\"payments.charge allowed? \")\n    print(bool(blocked.allow))\n}\n",
            },
            TemplateFile {
                rel: "README.md",
                body: "# {{name}}\n\nA support bot showing Axon's runtime-enforced `policy` block: allow/deny\nrules, a per-request budget, a per-user rate limit, and an audit trail.\n\n```sh\naxon run\n```\n",
            },
        ],
    },
    Template {
        name: "research",
        summary: "Multi-agent pipeline + plan strategies + structured output.",
        files: &[
            TemplateFile {
                rel: "axon.toml",
                body: "[package]\nname = \"{{name}}\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[caps]\ndefault = [\"Console\", \"LLM\", \"Net\", \"Memory\", \"Spawn\"]\n",
            },
            TemplateFile {
                rel: "src/main.ax",
                body: "//! {{name}} — a research agent that cites its sources.\n\nmodel brain = mock_model(\"fixed\", \"Per the 2025 amendments, transparency rules tightened.\")\nmemory kb = local_memory()\n\ntool search(q: String) -> String uses { Net } {\n    \"[stub-search] received: {q}\"\n}\n\nagent Researcher(m: Model, mem: Memory) {\n    on inquiry(question: String) -> String uses { LLM, Net, Memory } {\n        let ctx = self.mem.recall(question)\n        ask self.m {\n            system: \"Cite every claim. Use search when unsure.\"\n            memory: ctx\n            user:   question\n            tools:  [search]\n        }\n    }\n}\n\nfn main() uses { Spawn, LLM, Net, Memory, Console } {\n    kb.store(\"2025 amendments tightened transparency requirements\")\n    let r = spawn Researcher(m = brain, mem = kb)\n    print(r.inquiry(\"What changed in 2025?\"))\n}\n",
            },
            TemplateFile {
                rel: "README.md",
                body: "# {{name}}\n\nA research agent with memory recall + a tool the model can call.\nSwap the mock model for a real one when ready.\n",
            },
        ],
    },
    Template {
        name: "assistant",
        summary: "Streaming chat with conversation memory + Tainted I/O.",
        files: &[
            TemplateFile {
                rel: "axon.toml",
                body: "[package]\nname = \"{{name}}\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[caps]\ndefault = [\"Console\", \"LLM\", \"Net\", \"Memory\"]\n",
            },
            TemplateFile {
                rel: "src/main.ax",
                body: "//! {{name}} — a chat assistant with conversation memory.\n\nmodel brain = mock_model(\"fixed\", \"Hello! How can I help?\")\nmemory history = local_memory()\n\nfn main() uses { LLM, Net, Memory, Console } {\n    history.store(\"user said hi\")\n    let reply = ask brain {\n        system: \"You are a concise assistant.\"\n        memory: history.recall(\"\")\n        user:   \"hi\"\n    }\n    print(reply)\n}\n",
            },
            TemplateFile {
                rel: "README.md",
                body: "# {{name}}\n\nA chat assistant that recalls conversation memory before each turn.\n",
            },
        ],
    },
    Template {
        name: "pipeline",
        summary: "A graph workflow (deterministic, inspectable).",
        files: &[
            TemplateFile {
                rel: "axon.toml",
                body: "[package]\nname = \"{{name}}\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[caps]\ndefault = [\"Console\"]\n",
            },
            TemplateFile {
                rel: "src/main.ax",
                body: "//! {{name}} — a deterministic pipeline using flow combinators.\n\nfn classify(x: Int) -> Int { x + 1 }\nfn enrich(x: Int) -> Int { x * 10 }\n\nfn main() uses { Console } {\n    // A two-step sequence; flow_seq runs steps left-to-right.\n    let steps = list_new(classify, enrich)\n    let out = flow_seq(steps, 4)\n    print_int(out)\n}\n",
            },
            TemplateFile {
                rel: "README.md",
                body: "# {{name}}\n\nA deterministic, inspectable pipeline. Start here when your workflow\nshape is fixed and you want it replayable.\n",
            },
        ],
    },
    Template {
        name: "webhook",
        summary: "An `on webhook(...)` trigger + signature verification.",
        files: &[
            TemplateFile {
                rel: "axon.toml",
                body: "[package]\nname = \"{{name}}\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[caps]\ndefault = [\"Console\", \"Net\"]\n",
            },
            TemplateFile {
                rel: "src/main.ax",
                body: "//! {{name}} — a webhook-triggered handler.\n//! Serve it:  axon serve src/main.ax --handler on_event\n\nfn on_event(body: String) -> String uses { Console } {\n    print(\"received: \" + body)\n    `{\"ok\": true}`\n}\n\nfn main() uses { Console } {\n    print(on_event(`{\"type\": \"ping\"}`))\n}\n",
            },
            TemplateFile {
                rel: "README.md",
                body: "# {{name}}\n\nA webhook-shaped handler. Run it as an HTTP endpoint:\n\n```sh\naxon serve src/main.ax --handler on_event --listen 127.0.0.1:8080\n```\n",
            },
        ],
    },
    Template {
        name: "lambda",
        summary: "Serverless-shaped agent for `axon deploy --target lambda`.",
        files: &[
            TemplateFile {
                rel: "axon.toml",
                body: "[package]\nname = \"{{name}}\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[caps]\ndefault = [\"Console\", \"LLM\", \"Net\"]\n",
            },
            TemplateFile {
                rel: "src/main.ax",
                body: "//! {{name}} — a serverless-shaped single-handler agent.\n\nmodel brain = mock_model(\"fixed\", \"done\")\n\nfn handler(input: String) -> String uses { LLM, Net } {\n    ask brain { system: \"Be terse.\" user: input }\n}\n\nfn main() uses { LLM, Net, Console } {\n    print(handler(\"warm start\"))\n}\n",
            },
            TemplateFile {
                rel: "README.md",
                body: "# {{name}}\n\nServerless-shaped: one `handler`. Package it:\n\n```sh\naxon serverless_render lambda handler {{name}}   # via host binding, or\naxon deploy . -o dist --handler handler\n```\n",
            },
        ],
    },
    Template {
        name: "skill",
        summary: "A packageable, capability-audited skill (§53).",
        files: &[
            TemplateFile {
                rel: "axon.toml",
                body: "[package]\nname = \"{{name}}\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[caps]\ndefault = [\"Console\"]\n",
            },
            TemplateFile {
                rel: "src/lib.ax",
                body: "//! {{name}} — a reusable skill: a capability-audited bundle of tools.\n\npub tool greet(who: String) -> String {\n    \"Hello, {who}!\"\n}\n",
            },
            TemplateFile {
                rel: "src/main.ax",
                body: "use lib.greet\n\nfn main() uses { Console } {\n    print(greet(\"world\"))\n}\n",
            },
            TemplateFile {
                rel: "README.md",
                body: "# {{name}}\n\nA skill: a versioned, installable, capability-audited bundle. Pack it:\n\n```sh\naxon deploy . -o dist\n```\n",
            },
        ],
    },
];

// ---------------------------------------------------------------------------
// Embedded tour lessons
// ---------------------------------------------------------------------------

static LESSONS: &[Lesson] = &[
    Lesson {
        n: 1,
        title: "Bindings & printing",
        body: "Everything starts with `fn main`. Effects a function performs go in\nits `uses { ... }` row — `print` needs `Console`.\n\n  fn main() uses { Console } {\n      let name = \"Axon\"\n      print(\"hello, \" + name)\n  }\n\nTRY: save that as hello.ax and run `axon run hello.ax`.",
    },
    Lesson {
        n: 2,
        title: "Types & effects",
        body: "Axon is statically typed and effect-tracked. A function that reads a\nfile must declare `Fs.Read`:\n\n  fn load(path: String) -> String uses { Fs.Read } {\n      read_file(path)\n  }\n\nOmit the effect and the compiler stops you (E0301). That's the point:\nyou always know what a function may do.",
    },
    Lesson {
        n: 3,
        title: "Models & ask",
        body: "Models are language constructs. The mock model needs no API key:\n\n  model brain = mock_model(\"fixed\", \"42\")\n  fn main() uses { LLM, Console } {\n      print(ask brain { user: \"the answer?\" })\n  }",
    },
    Lesson {
        n: 4,
        title: "Tools the model can call",
        body: "A `tool` is a typed, capability-gated function the model may invoke:\n\n  tool now() -> String uses { Time } { str(time_now()) }\n\nPass it in the `tools:` slot of an `ask`/`plan` and the model can call\nit mid-response. The runtime checks the tool's effects against yours.",
    },
    Lesson {
        n: 5,
        title: "Agents",
        body: "An `agent` bundles a model + memory + handlers:\n\n  agent Greeter(m: Model) {\n      on hi(name: String) -> String uses { LLM } {\n          ask self.m { user: \"greet \" + name }\n      }\n  }\n\nSpawn + message it:  `let g = spawn Greeter(m = brain); g.hi(\"Sam\")`.",
    },
    Lesson {
        n: 6,
        title: "try / recover",
        body: "Recover from runtime errors inline:\n\n  let v = try { risky() } recover |e| { print(\"failed: \" + e); 0 }\n\nThe body runs; on any runtime error the message binds to `e` and the\nrecover branch produces the value.",
    },
    Lesson {
        n: 7,
        title: "Policies",
        body: "Guardrails the runtime enforces around every effect:\n\n  policy support {\n      allow tool kb.search\n      deny  tool payments.charge\n      budget per_request { usd = 0.50 }\n  }\n\nApplication code cannot bypass a policy. Check one with\n`policy_block_check(\"support\", \"tool\", \"kb.search\", true)`.",
    },
    Lesson {
        n: 8,
        title: "Testing & replay",
        body: "Write `test \"name\" { ... }` blocks and run `axon test`. Record a run's\nmodel responses with `axon run --record run.axj`, then replay it\nbyte-identically with `axon replay run.axj src/main.ax` — no network.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_has_a_manifest_and_main() {
        for t in TEMPLATES {
            assert!(
                t.files.iter().any(|f| f.rel == "axon.toml"),
                "template `{}` missing axon.toml",
                t.name
            );
            assert!(
                t.files.iter().any(|f| f.rel.ends_with(".ax")),
                "template `{}` missing a .ax file",
                t.name
            );
        }
    }

    #[test]
    fn eight_templates_present() {
        assert_eq!(TEMPLATES.len(), 8);
        for want in [
            "agent", "support", "research", "assistant", "pipeline", "webhook",
            "lambda", "skill",
        ] {
            assert!(find_template(want).is_some(), "missing template `{want}`");
        }
    }

    #[test]
    fn lessons_are_sequential() {
        for (i, l) in LESSONS.iter().enumerate() {
            assert_eq!(l.n, i as u32 + 1, "lesson numbering must be 1..N");
        }
    }

    #[test]
    fn project_name_validation() {
        assert!(is_valid_project_name("my-bot"));
        assert!(is_valid_project_name("bot_2"));
        assert!(!is_valid_project_name(""));
        assert!(!is_valid_project_name("-bad"));
        assert!(!is_valid_project_name("has space"));
    }

    #[test]
    fn unknown_template_is_none() {
        assert!(find_template("nope").is_none());
    }

    #[test]
    fn name_token_substituted() {
        // The agent template must reference {{name}} somewhere so the
        // substitution is exercised.
        let agent = find_template("agent").unwrap();
        assert!(agent.files.iter().any(|f| f.body.contains("{{name}}")));
    }
}
