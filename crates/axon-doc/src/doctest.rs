//! §35.5 — extract `` ```axon `` blocks from `///` doc comments and
//! synthesize them as `test "doc(item_name)" { ... }` declarations.
//!
//! Run via `axon test --doc` (or `--doc-only` to skip the user's
//! regular tests). The doc-test runner is exactly the same path as the
//! normal `cmd_test` loop — synthetic tests inherit per-stage features
//! like `--record-trajectory` automatically.
//!
//! Fence flags (Rust-style):
//!   * `` ```axon ``         — run the snippet as a test
//!   * `` ```axon,ignore ``  — skip entirely (the fence still renders in docs)
//!   * `` ```axon,no_run ``  — only synthesize; the synthesis itself
//!     proves the snippet parses, but the test body wraps it in an
//!     `if false { ... }` block so it never executes
//!
//! Output is a list of `DocSnippet { item_name, body }` the CLI can
//! synthesize into source text.

use axon_ast::Item;
use axon_project::LoadedProject;

use crate::extract_doc_pairs;

/// One extracted fenced code block.
#[derive(Clone, Debug)]
pub struct DocSnippet {
    /// Best-effort display name for the originating item (e.g.
    /// `"helpers.greet"`). Used as the test name.
    pub item_name: String,
    /// Source text of the fenced block. May be a single expression, a
    /// sequence of statements, or top-level items — the test wrapper
    /// only requires it parses as a block body.
    pub body: String,
    /// Was the block tagged `ignore` or `no_run`?
    pub disposition: SnippetDisposition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnippetDisposition {
    Run,
    Ignore,
    NoRun,
}

/// Walk every module's doc comments, extract `` ```axon `` fences, and
/// return them keyed by the item they were attached to. Module path is
/// prefixed onto the item name so two modules can have an `fn helper`
/// each without test-name collision.
pub fn extract_from_project(project: &LoadedProject) -> Vec<DocSnippet> {
    let mut out = Vec::new();
    for module in &project.modules {
        let mod_prefix = if module.module_path.is_empty() {
            String::new()
        } else {
            format!("{}.", module.module_path)
        };
        // Module-level docs (`//!`) aren't currently captured by
        // extract_doc_pairs; v0 covers only item-level `///`.
        let pairs = extract_doc_pairs(module);
        for pair in pairs {
            let Some(item) = module.program.items.get(pair.item_index) else {
                continue;
            };
            let item_name = item_label(item).unwrap_or_else(|| "<unknown>".to_string());
            let full_name = format!("{mod_prefix}{item_name}");
            for snippet in scan_fences(&pair.doc) {
                out.push(DocSnippet {
                    item_name: full_name.clone(),
                    body: snippet.body,
                    disposition: snippet.disposition,
                });
            }
        }
    }
    out
}

struct RawSnippet {
    body: String,
    disposition: SnippetDisposition,
}

/// Find every `` ```axon[,flag] `` … `` ``` `` fenced block in the
/// given doc text. Three-or-more backticks open/close the fence; the
/// info string after the opening fence is parsed for `axon` and an
/// optional comma-separated flag list.
fn scan_fences(text: &str) -> Vec<RawSnippet> {
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let Some(info) = trimmed.strip_prefix("```") else {
            continue;
        };
        // info string: `axon` or `axon,ignore` or `axon,no_run`.
        let info = info.trim();
        let disposition = match classify_info(info) {
            Some(d) => d,
            None => continue, // not an axon fence; skip
        };
        // Collect body lines until the closing fence (`` ``` ``).
        let mut body = String::new();
        for body_line in lines.by_ref() {
            if body_line.trim_start().starts_with("```") {
                break;
            }
            // Strip leading `/// ` style continuation if present — when
            // pairs.doc comes through, lines may carry the comment
            // prefix because extract_doc_pairs joined raw doc-comment
            // text with newlines. Be conservative: only strip a leading
            // `/// ` or `///`.
            let cleaned = body_line
                .strip_prefix("/// ")
                .or_else(|| body_line.strip_prefix("///"))
                .unwrap_or(body_line);
            body.push_str(cleaned);
            body.push('\n');
        }
        out.push(RawSnippet { body, disposition });
    }
    out
}

fn classify_info(info: &str) -> Option<SnippetDisposition> {
    let parts: Vec<&str> = info.split(',').map(str::trim).collect();
    if parts.is_empty() || parts[0] != "axon" {
        return None;
    }
    for flag in &parts[1..] {
        match *flag {
            "ignore" => return Some(SnippetDisposition::Ignore),
            "no_run" => return Some(SnippetDisposition::NoRun),
            _ => {} // unknown flag — accept the snippet but ignore the flag
        }
    }
    Some(SnippetDisposition::Run)
}

/// Identifier-shaped label for an item, used as part of the synthesized
/// test name. None for items the doc generator can't meaningfully
/// label (Use, Impl).
pub fn item_label(item: &Item) -> Option<String> {
    Some(match item {
        Item::Fn(f) => f.name.name.clone(),
        Item::Type(t) => t.name.name.clone(),
        Item::Schema(s) => s.name.name.clone(),
        Item::Agent(a) => a.name.name.clone(),
        Item::Actor(a) => a.name.name.clone(),
        Item::Tool(t) => t.name.name.clone(),
        Item::Model(m) => m.name.name.clone(),
        Item::Memory(m) => m.name.name.clone(),
        Item::Prompt(p) => p.name.name.clone(),
        Item::Trait(t) => t.name.name.clone(),
        Item::Const(c) => c.name.name.clone(),
        Item::Effect(e) => e.name.name.clone(),
        Item::Supervisor(s) => s.name.name.clone(),
        Item::Graph(g) => g.name.name.clone(),
        Item::Network(n) => n.name.name.clone(),
        Item::Orchestrate(o) => o.name.name.clone(),
        Item::Policy(p) => p.name.name.clone(),
        Item::MemPolicy(p) => p.name.name.clone(),
        Item::Config(c) => c.name.name.clone(),
        _ => return None,
    })
}

/// §35.6 verification fix C3 — refuse to splice fence bodies whose
/// brace structure would break out of the synthesized `test "..." {
/// ... }` wrapper. A malicious or sloppy fence body that contains an
/// unmatched `}` could otherwise inject arbitrary top-level items
/// (tools, fns, tests) into the merged program. Counts `{` and `}`
/// outside of string literals; the body is valid iff the running
/// depth stays non-negative and ends at zero.
fn body_braces_are_balanced(body: &str) -> bool {
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut in_raw = false;
    let mut prev = '\0';
    for c in body.chars() {
        // String-literal handling (best-effort — same rules the lexer
        // uses for `"..."` and `r"..."`). We don't try to parse all
        // edge cases; the check just needs to be conservative.
        if in_str {
            if c == '\\' && !in_raw {
                prev = c;
                continue;
            }
            if c == '"' && prev != '\\' {
                in_str = false;
                in_raw = false;
            }
            prev = c;
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                in_raw = prev == 'r';
            }
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
        prev = c;
    }
    !in_str && depth == 0
}

/// Synthesize a single combined Axon source string from the snippet
/// list. Each snippet becomes a `test "doc(name)" { <body> }` block.
/// `Ignore` snippets are skipped; `NoRun` snippets get wrapped in
/// `if false { ... }` so they parse + typecheck but never execute.
///
/// `existing_test_names` is the set of test names the user already
/// declared in the project — used to suffix collisions so a doc-test
/// never silently shadows or breaks a real test. Pass `&[]` if you
/// don't have the list (e.g. unit tests).
pub fn synthesize_with_existing(
    snippets: &[DocSnippet],
    existing_test_names: &[String],
) -> String {
    use std::collections::HashMap;
    let mut out = String::new();
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    let existing: std::collections::HashSet<&str> =
        existing_test_names.iter().map(|s| s.as_str()).collect();
    for s in snippets {
        if s.disposition == SnippetDisposition::Ignore {
            continue;
        }
        // §35.6 verification fix C3: refuse to splice an unbalanced
        // body. Emit a synthetic stub that contains a clear error
        // message so the test runner surfaces the issue instead of
        // silently dropping the snippet.
        if !body_braces_are_balanced(&s.body) {
            let test_name = format!("doc({}__brace_imbalance)", s.item_name);
            let escaped = test_name.replace('"', "\\\"");
            out.push_str(&format!(
                "test \"{escaped}\" {{ assert(false, \"doc snippet for `{}` has unbalanced braces — splice refused (would break out of the test wrapper)\") }}\n\n",
                s.item_name.replace('"', "\\\"")
            ));
            continue;
        }
        // Deduplicate test names — two snippets on the same item get
        // numeric suffixes; collisions with USER-declared tests also
        // suffix so doc-test synthesis never produces an E0204.
        let count = name_counts.entry(s.item_name.clone()).or_insert(0);
        *count += 1;
        let mut suffix = if *count == 1 {
            String::new()
        } else {
            format!("#{}", *count)
        };
        let mut candidate = format!("doc({}{})", s.item_name, suffix);
        // §35.6 verification fix M3: keep bumping the suffix until the
        // generated name doesn't collide with an existing user test.
        while existing.contains(candidate.as_str()) {
            *count += 1;
            suffix = format!("#{}", *count);
            candidate = format!("doc({}{})", s.item_name, suffix);
        }
        let escaped = candidate.replace('"', "\\\"");
        if s.disposition == SnippetDisposition::NoRun {
            out.push_str(&format!(
                "test \"{escaped}\" {{ if false {{\n{}}} }}\n\n",
                s.body
            ));
        } else {
            out.push_str(&format!("test \"{escaped}\" {{\n{}}}\n\n", s.body));
        }
    }
    out
}

/// Convenience wrapper preserving the v0 signature (no collision
/// check). Calls `synthesize_with_existing(snippets, &[])`.
pub fn synthesize(snippets: &[DocSnippet]) -> String {
    synthesize_with_existing(snippets, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_basic_fence() {
        let text = "Use the helper:\n```axon\nlet x = 1\n```";
        let snips = scan_fences(text);
        assert_eq!(snips.len(), 1);
        assert!(snips[0].body.contains("let x = 1"));
        assert_eq!(snips[0].disposition, SnippetDisposition::Run);
    }

    #[test]
    fn scan_handles_ignore_and_no_run_flags() {
        let text = "Skipped:\n```axon,ignore\nlet x = 1\n```\nTypecheck only:\n```axon,no_run\nlet y = 2\n```";
        let snips = scan_fences(text);
        assert_eq!(snips.len(), 2);
        assert_eq!(snips[0].disposition, SnippetDisposition::Ignore);
        assert_eq!(snips[1].disposition, SnippetDisposition::NoRun);
    }

    #[test]
    fn scan_ignores_non_axon_fences() {
        let text = "```python\nprint('x')\n```\n```\nplain\n```";
        let snips = scan_fences(text);
        assert!(snips.is_empty());
    }

    #[test]
    fn synthesize_wraps_snippets_as_test_blocks() {
        let snippets = vec![
            DocSnippet {
                item_name: "f".into(),
                body: "let x = 1\n".into(),
                disposition: SnippetDisposition::Run,
            },
            DocSnippet {
                item_name: "g".into(),
                body: "let y = 2\n".into(),
                disposition: SnippetDisposition::Ignore, // dropped
            },
            DocSnippet {
                item_name: "h".into(),
                body: "let z = 3\n".into(),
                disposition: SnippetDisposition::NoRun,
            },
        ];
        let src = synthesize(&snippets);
        assert!(src.contains("test \"doc(f)\""));
        assert!(!src.contains("test \"doc(g)\""), "ignore should be dropped");
        assert!(src.contains("test \"doc(h)\""));
        assert!(src.contains("if false"), "no_run should wrap in if false");
    }

    #[test]
    fn synthesize_dedupes_repeated_item_names() {
        let snippets = vec![
            DocSnippet {
                item_name: "f".into(),
                body: "let a = 1\n".into(),
                disposition: SnippetDisposition::Run,
            },
            DocSnippet {
                item_name: "f".into(),
                body: "let b = 2\n".into(),
                disposition: SnippetDisposition::Run,
            },
        ];
        let src = synthesize(&snippets);
        assert!(src.contains("test \"doc(f)\""));
        assert!(src.contains("test \"doc(f#2)\""), "{src}");
    }
}
