//! `use skill` semantics (§53).
//!
//! A skill is a versioned, capability-audited bundle of prompts, tools,
//! schemas, and evals. When an Axon program *imports* a skill, two
//! things have to happen statically:
//!
//!   1. **Effect narrowing** — the importer can only call into the
//!      skill with capabilities it itself holds. A pure caller cannot
//!      reach a `Net`-using skill; an importer with `Net` can. The
//!      narrowing is a row-intersection between the caller's grant and
//!      the skill's `capabilities` row, with the result attached to
//!      every callable the skill exposes.
//!   2. **Namespace binding** — skill items land under a namespace
//!      derived from the skill name (`use skill greeter as g; g.hello()`).
//!      v0 uses a flat record-of-callables model; the parser-level
//!      `use skill` declaration form lands when we add real
//!      module-resolution.
//!
//! This module ships the static checking pieces. The host's
//! `skill_use(name, caller_caps)` returns either a typed binding the
//! runtime can install or an error explaining which capabilities are
//! missing.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::Manifest;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillBinding {
    /// Namespace the skill is imported under (defaults to `manifest.name`).
    pub alias: String,
    /// Capability row the importer holds.
    pub caller_caps: BTreeSet<String>,
    /// Capability row the skill declares (`manifest.capabilities`).
    pub skill_caps: BTreeSet<String>,
    /// Intersection — the row attached to every skill-exposed callable.
    pub effective_caps: BTreeSet<String>,
    /// True when the binding is sufficient (caller is a superset of skill).
    pub is_satisfied: bool,
    /// Capabilities the skill needs but the caller doesn't hold.
    pub missing_caps: BTreeSet<String>,
}

impl SkillBinding {
    pub fn alias_or_name(&self) -> &str {
        self.alias.as_str()
    }

    /// Effective caps as a stable Vec<String> for diagnostics.
    pub fn effective_caps_vec(&self) -> Vec<String> {
        self.effective_caps.iter().cloned().collect()
    }
}

/// Resolve `use skill <name> [as <alias>]` against the importer's
/// capability set. The result drives both the runtime install and the
/// diagnostic the type checker emits when the caller can't satisfy the
/// skill's needs.
pub fn bind_skill(
    manifest: &Manifest,
    caller_caps: impl IntoIterator<Item = String>,
    alias: Option<&str>,
) -> SkillBinding {
    let caller: BTreeSet<String> = caller_caps.into_iter().collect();
    let skill: BTreeSet<String> = manifest.capabilities.iter().cloned().collect();
    let missing: BTreeSet<String> = skill.difference(&caller).cloned().collect();
    let is_satisfied = missing.is_empty();
    let effective: BTreeSet<String> = skill.intersection(&caller).cloned().collect();
    SkillBinding {
        alias: alias.unwrap_or(manifest.name.as_str()).to_string(),
        caller_caps: caller,
        skill_caps: skill,
        effective_caps: effective,
        is_satisfied,
        missing_caps: missing,
    }
}

/// Narrow a callee's effect row to what the importer actually holds.
/// `callee_caps` is the row declared on the specific skill-exposed
/// item; `caller_caps` is what the importer brought into scope. The
/// returned row is the *intersection* — if the importer can't pass a
/// capability through, the callee can't ask for it.
pub fn narrow_effects(
    callee_caps: &BTreeSet<String>,
    caller_caps: &BTreeSet<String>,
) -> BTreeSet<String> {
    callee_caps.intersection(caller_caps).cloned().collect()
}

/// Effect-narrowing diagnostic for the type checker. Returns a stable
/// human-readable message naming the missing rows.
pub fn explain_missing(binding: &SkillBinding) -> Option<String> {
    if binding.is_satisfied {
        return None;
    }
    let mut missing: Vec<&String> = binding.missing_caps.iter().collect();
    missing.sort();
    Some(format!(
        "skill `{}` requires capabilities the importer doesn't hold: {}",
        binding.alias,
        missing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str, caps: &[&str]) -> Manifest {
        Manifest {
            name: name.into(),
            version: "0.1.0".into(),
            description: String::new(),
            entrypoint: "src/lib.ax".into(),
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            dependencies: Vec::new(),
            authors: Vec::new(),
        }
    }

    #[test]
    fn satisfied_binding_when_caller_is_superset() {
        let m = manifest("greeter", &["Console"]);
        let b = bind_skill(
            &m,
            ["Console".to_string(), "Net".to_string()],
            None,
        );
        assert!(b.is_satisfied);
        assert!(b.missing_caps.is_empty());
        assert!(b.effective_caps.contains("Console"));
    }

    #[test]
    fn unsatisfied_binding_lists_missing() {
        let m = manifest("scraper", &["Net", "Fs.Write"]);
        let b = bind_skill(&m, ["Net".to_string()], None);
        assert!(!b.is_satisfied);
        assert!(b.missing_caps.contains("Fs.Write"));
        let msg = explain_missing(&b).unwrap();
        assert!(msg.contains("Fs.Write"));
    }

    #[test]
    fn alias_overrides_default_name() {
        let m = manifest("research-agent", &[]);
        let b = bind_skill(&m, Vec::<String>::new(), Some("r"));
        assert_eq!(b.alias_or_name(), "r");
    }

    #[test]
    fn narrow_effects_intersects_rows() {
        let callee: BTreeSet<String> = ["Net".into(), "Fs.Write".into()].into_iter().collect();
        let caller: BTreeSet<String> = ["Net".into(), "Console".into()].into_iter().collect();
        let narrowed = narrow_effects(&callee, &caller);
        assert_eq!(
            narrowed.into_iter().collect::<Vec<_>>(),
            vec!["Net".to_string()]
        );
    }

    #[test]
    fn effective_caps_excludes_caller_caps_the_skill_doesnt_use() {
        let m = manifest("audit", &["Audit"]);
        let b = bind_skill(
            &m,
            ["Audit".to_string(), "Net".to_string()],
            None,
        );
        assert_eq!(b.effective_caps_vec(), vec!["Audit".to_string()]);
    }

    #[test]
    fn empty_caps_skill_always_satisfied() {
        let m = manifest("pure", &[]);
        let b = bind_skill(&m, Vec::<String>::new(), None);
        assert!(b.is_satisfied);
        assert!(b.effective_caps.is_empty());
    }
}
