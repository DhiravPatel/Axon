//! Runtime capability tracking.
//!
//! A [`CapSet`] is the set of *effects* the runtime currently has authority
//! to perform. Side-effect built-ins (network, file system, console I/O,
//! the LLM, memory, ...) check the active set before doing real work; the
//! check yields a clean [`RuntimeError`](crate::error::RuntimeError) when
//! the required effect is missing, rather than silently succeeding or
//! falling through to some host primitive.
//!
//! On function entry the runtime *attenuates* the active set to the
//! function's declared `uses {...}` row, so a caller that holds `Net + Fs`
//! cannot accidentally pass that authority into a callee that only declared
//! `Net`. This is the "no ambient authority" half of the spec's capability
//! model — the other half, value-level capability tokens with prefix-scoped
//! attenuation, lands when real tools / file-system bindings do (Stage 6+).

use std::collections::BTreeSet;
use std::fmt;

/// A set of active effect names. Backed by a sorted set so diagnostics are
/// stable across runs and equality is cheap.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapSet {
    effects: BTreeSet<String>,
}

impl CapSet {
    pub fn empty() -> Self {
        Self::default()
    }

    /// The default set granted by `axon run` to local scripts. Strip with
    /// `--isolated` or pick a subset with `--with`. Tools that embed the
    /// Axon runtime build their own set from policy.
    pub fn standard_default() -> Self {
        Self::from_iter([
            "Console",
            "Fs",
            "Fs.Read",
            "Fs.Write",
            "Time",
            "Random",
            "Net",
            "LLM",
            "Memory",
            "Tool",
            "Spawn",
            "Channel",
            "Crypto",
            "Process",
            "Env",
            "Audit",
            "Log",
            "Db",
            "Db.Read",
            "Db.Write",
            // §35 computer-use: click/type/screenshot tools.
            "Computer",
        ])
    }

    pub fn from_iter<I, S>(it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            effects: it.into_iter().map(Into::into).collect(),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        // Granting a parent effect (e.g. `Fs`) also grants any dotted child
        // (`Fs.Read`, `Fs.Write`). The CLI/host typically grants the leaf
        // forms directly, but allowing parent dominance keeps `--with Fs`
        // ergonomic.
        if self.effects.contains(name) {
            return true;
        }
        if let Some(idx) = name.rfind('.') {
            let parent = &name[..idx];
            if self.effects.contains(parent) {
                return true;
            }
        }
        false
    }

    pub fn add(&mut self, name: impl Into<String>) {
        self.effects.insert(name.into());
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.effects.iter().map(|s| s.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub fn len(&self) -> usize {
        self.effects.len()
    }

    /// New set containing the effects in `declared` that are also present in
    /// `self`. Used at function-entry attenuation.
    pub fn intersect_with_declared(&self, declared: &[String]) -> CapSet {
        let mut out = CapSet::empty();
        for d in declared {
            if self.has(d) {
                out.effects.insert(d.clone());
            }
        }
        out
    }

    /// Effects in `declared` that are missing from `self`. Used to build
    /// "function requires capability X which is not in scope" errors.
    pub fn missing(&self, declared: &[String]) -> Vec<String> {
        declared
            .iter()
            .filter(|d| !self.has(d))
            .cloned()
            .collect()
    }
}

impl fmt::Display for CapSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{")?;
        for (i, e) in self.effects.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            f.write_str(e)?;
        }
        f.write_str("}")
    }
}

/// Parse a comma-separated list of effect names (`Console,Net,Fs.Read`).
/// Empty input yields an empty set.
pub fn parse_cap_list(input: &str) -> CapSet {
    CapSet::from_iter(
        input
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    )
}
