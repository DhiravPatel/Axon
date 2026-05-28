//! Offline diagnostic explanations (§57).
//!
//! Every diagnostic code maps to a stable, offline explanation: a
//! one-line title, *why* the rule exists, the most common fix, and the
//! family it belongs to. `axon explain <CODE>` prints these; the
//! `--explain-errors` flag inlines them after each diagnostic.
//!
//! Codes are **stable across compiler versions** and never reused —
//! deleted codes are tombstoned. The family ranges follow §57.4:
//!
//! | Range  | Family                              |
//! |--------|-------------------------------------|
//! | E01xx  | Lexing & syntax                     |
//! | E02xx  | Types & generics                    |
//! | E03xx  | Effect rows & budgets               |
//! | E04xx  | Capabilities, policies, taint       |
//! | E05xx  | Agents/actors/scheduling            |
//! | E06xx  | Schemas, validation, generation     |
//! | E07xx  | Tools & FFI                         |
//! | E08xx  | Modules, packages, manifest         |
//! | E09xx  | Replay & determinism                |
//! | P0xxx  | Project / privacy                   |
//! | W1xxx  | Lints (warnings)                    |

/// A resolved explanation for a diagnostic code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explanation {
    pub code: &'static str,
    pub family: &'static str,
    pub title: &'static str,
    /// Why the rule exists — the design rationale a first-time user
    /// needs to *understand* rather than memorize the error.
    pub why: &'static str,
    /// The most common fix.
    pub fix: &'static str,
    /// Offline-doc-page slug (the hosted URL is `…/CODE`).
    pub learn: &'static str,
}

impl Explanation {
    /// Render the explanation as a plain-text block suitable for the
    /// terminal (used by `axon explain` and `--explain-errors`).
    pub fn render(&self) -> String {
        format!(
            "{code} — {title}\n  family: {family}\n\n  why:  {why}\n  fix:  {fix}\n  learn: https://docs.axon-lang.org/{learn}",
            code = self.code,
            title = self.title,
            family = self.family,
            why = self.why,
            fix = self.fix,
            learn = self.learn,
        )
    }

    /// A single-line summary, inlined after a diagnostic under
    /// `--explain-errors`.
    pub fn one_liner(&self) -> String {
        format!("{}: {} — {}", self.code, self.title, self.why)
    }
}

/// Look up the explanation for a diagnostic code. Falls back to a
/// family-level explanation when the exact code isn't catalogued yet,
/// so `axon explain` is useful even for codes without a bespoke page.
pub fn explain(code: &str) -> Option<Explanation> {
    if let Some(e) = CATALOGUE.iter().find(|e| e.code == code) {
        return Some(e.clone());
    }
    family_fallback(code)
}

/// Family name for a code prefix.
pub fn family_of(code: &str) -> &'static str {
    let c = code.trim();
    if let Some(rest) = c.strip_prefix('E') {
        match rest.get(0..2) {
            Some("01") => "Lexing & syntax",
            Some("02") => "Types & generics",
            Some("03") => "Effect rows & budgets",
            Some("04") => "Capabilities, policies, taint",
            Some("05") => "Agents, actors & scheduling",
            Some("06") => "Schemas, validation & generation",
            Some("07") => "Tools & FFI",
            Some("08") => "Modules, packages & manifest",
            Some("09") => "Replay & determinism",
            _ => "Compiler error",
        }
    } else if c.starts_with('W') {
        "Lint"
    } else if c.starts_with('P') {
        "Project & privacy"
    } else {
        "Diagnostic"
    }
}

fn family_fallback(code: &str) -> Option<Explanation> {
    let family = family_of(code);
    if family == "Diagnostic" {
        return None;
    }
    // Leak a 'static code string so the Explanation can borrow it. The
    // CLI calls this at most a handful of times per invocation, so the
    // tiny intentional leak is acceptable and keeps the type simple.
    let code_static: &'static str = Box::leak(code.to_string().into_boxed_str());
    Some(Explanation {
        code: code_static,
        family,
        title: "No bespoke explanation page yet",
        why: "This code belongs to the family above but doesn't have a dedicated explanation entry. The diagnostic message itself describes the specific problem.",
        fix: "Read the inline message and caret; consult the family docs.",
        learn: "diagnostics",
    })
}

/// Explain a *concept* rather than a code: `axon explain effect:LLM`,
/// `axon explain capability:Tool`.
pub fn explain_concept(kind: &str, name: &str) -> Option<String> {
    match kind {
        "effect" => effect_doc(name),
        "capability" | "cap" => capability_doc(name),
        _ => None,
    }
}

fn effect_doc(name: &str) -> Option<String> {
    let body = match name {
        "Console" => "Write to stdout/stderr (`print`, `println`, `eprint`).",
        "Net" => "Make outbound network requests (`http_fetch`, A2A calls).",
        "Fs" | "Fs.Read" | "Fs.Write" => "Read or write the filesystem. Split into `Fs.Read` and `Fs.Write` so least-privilege grants are precise.",
        "LLM" => "Call a language model (`ask`, `generate`, `plan`). Billed + latency-bearing; usually budgeted.",
        "Memory" => "Read or write agent memory / vector stores.",
        "Time" => "Read the wall clock (`time_now`). Recorded for deterministic replay.",
        "Random" => "Draw randomness (`random_int`, `random_float`). Seeded + recorded for replay.",
        "Spawn" => "Spawn agents/actors.",
        "Process" => "Launch subprocesses (sandboxed FFI, extern tools).",
        _ => return Some(format!(
            "`{name}` is an effect: a capability a function declares in its `uses {{ … }}` row. The compiler tracks it; the runtime enforces it. Effects are attenuable but never strengthenable."
        )),
    };
    Some(format!(
        "effect `{name}`\n\n  {body}\n\n  Declared in a function's `uses {{ … }}` row; checked at compile time AND enforced at run time. A callee can only receive effects its caller already holds (attenuation)."
    ))
}

fn capability_doc(name: &str) -> Option<String> {
    Some(format!(
        "capability `{name}`\n\n  A capability is an unforgeable, attenuable token granting access to an external resource (a tool, the network, the filesystem). Capabilities are *values granted explicitly* — never ambient. An agent receives exactly the capabilities passed to its constructor and can pass strictly fewer to its callees, never more."
    ))
}

/// The static catalogue. Bespoke entries for the codes the compiler
/// actually emits today; everything else falls back to the family.
static CATALOGUE: &[Explanation] = &[
    Explanation {
        code: "E0201",
        family: "Types & generics",
        title: "type mismatch",
        why: "An expression produced a value whose type doesn't match what the surrounding context requires. Axon is statically typed: a `String` can't flow where an `Int` is expected without an explicit conversion.",
        fix: "Convert the value (`int(x)`, `str(x)`), fix the binding's declared type, or change the expression to produce the expected type.",
        learn: "E0201",
    },
    Explanation {
        code: "E0202",
        family: "Types & generics",
        title: "cannot find name in scope",
        why: "A referenced identifier isn't bound anywhere visible: not a local, parameter, top-level item, or imported name. Often a typo or a missing `use`.",
        fix: "Check the spelling (the compiler suggests near-matches), add a `use` import, or declare the binding before use.",
        learn: "E0202",
    },
    Explanation {
        code: "E0203",
        family: "Types & generics",
        title: "cannot find type in scope",
        why: "A type annotation names a type that isn't declared or imported.",
        fix: "Import the type, declare it, or check for a typo against built-ins (`Int`, `String`, `List`, …).",
        learn: "E0203",
    },
    Explanation {
        code: "E0204",
        family: "Types & generics",
        title: "duplicate definition",
        why: "Two items in the same scope share a name. Names must be unique within a module so references are unambiguous.",
        fix: "Rename one of the definitions, or move it to a different module.",
        learn: "E0204",
    },
    Explanation {
        code: "E0205",
        family: "Types & generics",
        title: "no such field",
        why: "Field access (`x.field`) named a field that the value's type doesn't have.",
        fix: "Check the field name against the type's declaration; the compiler lists the available fields.",
        learn: "E0205",
    },
    Explanation {
        code: "E0206",
        family: "Types & generics",
        title: "no such method",
        why: "Method call (`x.method(...)`) named a method not defined on the receiver's type or any trait it implements.",
        fix: "Check the method name, or bring the defining trait into scope with `use`.",
        learn: "E0206",
    },
    Explanation {
        code: "E0207",
        family: "Types & generics",
        title: "value is not callable",
        why: "A call expression `f(...)` was applied to something that isn't a function, tool, or closure.",
        fix: "Make sure `f` is a function/closure/tool value; you may have shadowed it with a non-callable binding.",
        learn: "E0207",
    },
    Explanation {
        code: "E0213",
        family: "Types & generics",
        title: "binary operator not defined",
        why: "An operator was applied to operand types it isn't defined for (e.g. `+` on a `Bool` and an `Int`).",
        fix: "Convert the operands to compatible types, or use the operator that matches the types you have.",
        learn: "E0213",
    },
    Explanation {
        code: "E0230",
        family: "Types & generics",
        title: "type cannot be iterated",
        why: "A `for` loop's iterable expression has a type that isn't a list, set, map, stream, or channel.",
        fix: "Iterate over a collection; convert scalars to a list first if needed.",
        learn: "E0230",
    },
    Explanation {
        code: "E0241",
        family: "Agents, actors & scheduling",
        title: "not an agent or actor",
        why: "`spawn X(...)` requires `X` to be an `agent` or `actor` declaration. The named item is something else.",
        fix: "Spawn an `agent`/`actor`, or call the item directly if it's a plain function.",
        learn: "E0241",
    },
    Explanation {
        code: "E0301",
        family: "Effect rows & budgets",
        title: "missing effect in `uses` row",
        why: "A function performs an effect (network, LLM, filesystem, …) that it doesn't declare in its `uses { … }` row. Effects are tracked statically so callers always know what a function may do.",
        fix: "Add the effect to the function's `uses { … }` clause, or stop performing it.",
        learn: "E0301",
    },
    Explanation {
        code: "E0421",
        family: "Capabilities, policies, taint",
        title: "unknown capability name",
        why: "A `uses { … }` row or grant named a capability the compiler doesn't recognize.",
        fix: "Use a known capability (`Net`, `Fs.Read`, `LLM`, …); the compiler suggests near-matches.",
        learn: "E0421",
    },
    Explanation {
        code: "E0712",
        family: "Capabilities, policies, taint",
        title: "effect not granted by policy",
        why: "A policy-bound agent attempted an effect its policy doesn't allow. Policies are runtime-enforced guardrails that application code cannot bypass.",
        fix: "Add an `allow` rule to the policy (optionally with a `when` clause), or route the effect through an agent that holds the grant.",
        learn: "E0712",
    },
    Explanation {
        code: "P0001",
        family: "Project & privacy",
        title: "name declared in two modules",
        why: "The merged program has the same item name declared in more than one module. The flat global namespace requires unique names.",
        fix: "Rename one declaration, or namespace it.",
        learn: "P0001",
    },
    Explanation {
        code: "P0010",
        family: "Project & privacy",
        title: "item is not `pub`",
        why: "A `use` imported an item that isn't declared `pub` in its source module. Non-`pub` items are module-private.",
        fix: "Add `pub` to the item's declaration to expose it across modules.",
        learn: "P0010",
    },
    Explanation {
        code: "P0011",
        family: "Project & privacy",
        title: "module has no such item",
        why: "A `use` referenced an item that doesn't exist in the named module.",
        fix: "Check the item name and the module path.",
        learn: "P0011",
    },
];

/// Number of bespoke catalogue entries — surfaced by `axon explain`
/// with no argument.
pub fn catalogue_len() -> usize {
    CATALOGUE.len()
}

/// All catalogued codes, sorted, for listing.
pub fn catalogue_codes() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = CATALOGUE.iter().map(|e| e.code).collect();
    v.sort_unstable();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_code_resolves_to_bespoke_entry() {
        let e = explain("E0202").unwrap();
        assert_eq!(e.title, "cannot find name in scope");
        assert!(e.render().contains("E0202"));
    }

    #[test]
    fn unknown_code_in_known_family_falls_back() {
        let e = explain("E0299").unwrap();
        assert_eq!(e.family, "Types & generics");
        assert!(e.title.contains("No bespoke"));
    }

    #[test]
    fn family_of_maps_prefixes() {
        assert_eq!(family_of("E0101"), "Lexing & syntax");
        assert_eq!(family_of("E0305"), "Effect rows & budgets");
        assert_eq!(family_of("E0421"), "Capabilities, policies, taint");
        assert_eq!(family_of("E0712"), "Tools & FFI");
        assert_eq!(family_of("W1203"), "Lint");
        assert_eq!(family_of("P0010"), "Project & privacy");
    }

    #[test]
    fn totally_unknown_code_returns_none() {
        assert!(explain("ZZZ999").is_none());
    }

    #[test]
    fn effect_concept_doc() {
        let d = explain_concept("effect", "LLM").unwrap();
        assert!(d.contains("language model"));
        assert!(d.contains("attenuation") || d.contains("uses"));
    }

    #[test]
    fn capability_concept_doc() {
        let d = explain_concept("capability", "Tool").unwrap();
        assert!(d.contains("unforgeable"));
    }

    #[test]
    fn one_liner_is_compact() {
        let e = explain("E0201").unwrap();
        let ol = e.one_liner();
        assert!(ol.starts_with("E0201:"));
        assert!(!ol.contains('\n'));
    }
}
