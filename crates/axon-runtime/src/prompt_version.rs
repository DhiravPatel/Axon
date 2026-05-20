//! Prompt versioning for A/B testing (§24.3).
//!
//! Production agents ship many prompt revisions and evaluate each one
//! against the same dataset. A `prompt support_answer @version("v8") { ... }`
//! declaration records `v8` so:
//!
//!   * `axon test --eval evals/support.ax --prompt-version v8` runs the
//!     suite against that revision exactly;
//!   * `axon optimize` can list every registered version of a prompt
//!     and compare evaluation scores;
//!   * production traces carry the `prompt_version` attribute so a
//!     regression's blast radius is "calls of v8" rather than "calls of
//!     handle_ticket".
//!
//! The library is a typed registry — `PromptVersionRegistry::register`
//! / `pick` / `versions_for` — keyed by `(prompt_name, version)`.
//! Persists to JSON so versions survive process restarts and can be
//! diffed across deploys.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptVersion {
    pub prompt_name: String,
    pub version: String,
    /// Free-form body — the host passes the rendered prompt text in.
    /// Empty for a registration that's just declaring the version
    /// label exists.
    #[serde(default)]
    pub body: String,
    /// Optional commit / tag / changelog entry the author can attach.
    #[serde(default)]
    pub notes: String,
    /// Wall-clock nanoseconds when this version was registered. Used
    /// to break ties when multiple versions share a name in source
    /// across loaded modules.
    #[serde(default)]
    pub registered_at_ns: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptVersionRegistry {
    /// `(prompt_name, version)` → entry.
    pub entries: BTreeMap<String, PromptVersion>,
    /// Per-prompt default-version pointer. `axon test` and
    /// `axon optimize` honor this when no version is given on the CLI.
    pub defaults: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptVersionError {
    UnknownPrompt(String),
    UnknownVersion { prompt: String, version: String },
    EmptyName,
    DuplicateRegistration { key: String },
}

impl std::fmt::Display for PromptVersionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromptVersionError::UnknownPrompt(p) => write!(f, "no prompt named `{p}`"),
            PromptVersionError::UnknownVersion { prompt, version } => {
                write!(f, "prompt `{prompt}` has no version `{version}`")
            }
            PromptVersionError::EmptyName => f.write_str("prompt name and version must be non-empty"),
            PromptVersionError::DuplicateRegistration { key } => {
                write!(f, "duplicate prompt+version registration `{key}`")
            }
        }
    }
}

impl std::error::Error for PromptVersionError {}

fn key_of(name: &str, version: &str) -> String {
    format!("{name}::{version}")
}

impl PromptVersionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a (prompt, version, body) triple. Duplicate
    /// registrations of an exact key are rejected — re-running a
    /// project shouldn't silently overwrite a registered version.
    pub fn register(
        &mut self,
        prompt: impl Into<String>,
        version: impl Into<String>,
        body: impl Into<String>,
        notes: impl Into<String>,
        now_ns: i64,
    ) -> Result<(), PromptVersionError> {
        let prompt: String = prompt.into();
        let version: String = version.into();
        if prompt.is_empty() || version.is_empty() {
            return Err(PromptVersionError::EmptyName);
        }
        let key = key_of(&prompt, &version);
        if self.entries.contains_key(&key) {
            return Err(PromptVersionError::DuplicateRegistration { key });
        }
        // First version registered for a prompt becomes its default
        // unless one is already set.
        self.defaults
            .entry(prompt.clone())
            .or_insert_with(|| version.clone());
        self.entries.insert(
            key,
            PromptVersion {
                prompt_name: prompt,
                version,
                body: body.into(),
                notes: notes.into(),
                registered_at_ns: now_ns,
            },
        );
        Ok(())
    }

    /// Mark `version` as the default for `prompt`.
    pub fn set_default(
        &mut self,
        prompt: &str,
        version: &str,
    ) -> Result<(), PromptVersionError> {
        if !self.entries.contains_key(&key_of(prompt, version)) {
            return Err(PromptVersionError::UnknownVersion {
                prompt: prompt.to_string(),
                version: version.to_string(),
            });
        }
        self.defaults.insert(prompt.to_string(), version.to_string());
        Ok(())
    }

    pub fn pick(&self, prompt: &str, version: Option<&str>) -> Result<&PromptVersion, PromptVersionError> {
        let v = match version {
            Some(v) => v.to_string(),
            None => self
                .defaults
                .get(prompt)
                .cloned()
                .ok_or_else(|| PromptVersionError::UnknownPrompt(prompt.to_string()))?,
        };
        self.entries
            .get(&key_of(prompt, &v))
            .ok_or(PromptVersionError::UnknownVersion {
                prompt: prompt.to_string(),
                version: v,
            })
    }

    pub fn versions_for(&self, prompt: &str) -> Vec<&PromptVersion> {
        let mut out: Vec<&PromptVersion> = self
            .entries
            .values()
            .filter(|e| e.prompt_name == prompt)
            .collect();
        out.sort_by_key(|e| e.registered_at_ns);
        out
    }

    pub fn prompts(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .entries
            .values()
            .map(|e| e.prompt_name.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> PromptVersionRegistry {
        let mut r = PromptVersionRegistry::new();
        r.register("support_answer", "v1", "be terse", "first cut", 100)
            .unwrap();
        r.register(
            "support_answer",
            "v2",
            "be terse and cite",
            "added citation guidance",
            200,
        )
        .unwrap();
        r.register("triage", "v1", "classify the ticket", "", 50)
            .unwrap();
        r
    }

    #[test]
    fn first_registration_becomes_default() {
        let r = make();
        assert_eq!(r.pick("support_answer", None).unwrap().version, "v1");
        assert_eq!(r.pick("triage", None).unwrap().version, "v1");
    }

    #[test]
    fn explicit_version_pick_returns_that_version() {
        let r = make();
        let v = r.pick("support_answer", Some("v2")).unwrap();
        assert_eq!(v.body, "be terse and cite");
        assert_eq!(v.notes, "added citation guidance");
    }

    #[test]
    fn set_default_promotes_a_later_version() {
        let mut r = make();
        r.set_default("support_answer", "v2").unwrap();
        assert_eq!(r.pick("support_answer", None).unwrap().version, "v2");
    }

    #[test]
    fn unknown_prompt_errors() {
        let r = make();
        assert!(matches!(
            r.pick("does-not-exist", None).unwrap_err(),
            PromptVersionError::UnknownPrompt(_)
        ));
        assert!(matches!(
            r.pick("support_answer", Some("v99")).unwrap_err(),
            PromptVersionError::UnknownVersion { .. }
        ));
    }

    #[test]
    fn duplicate_registration_rejected() {
        let mut r = make();
        assert!(matches!(
            r.register("support_answer", "v1", "x", "", 999).unwrap_err(),
            PromptVersionError::DuplicateRegistration { .. }
        ));
    }

    #[test]
    fn empty_name_rejected() {
        let mut r = PromptVersionRegistry::new();
        assert!(matches!(
            r.register("", "v1", "", "", 0).unwrap_err(),
            PromptVersionError::EmptyName
        ));
        assert!(matches!(
            r.register("p", "", "", "", 0).unwrap_err(),
            PromptVersionError::EmptyName
        ));
    }

    #[test]
    fn versions_for_returns_chronological() {
        let r = make();
        let vs = r.versions_for("support_answer");
        assert_eq!(vs.len(), 2);
        assert_eq!(vs[0].version, "v1");
        assert_eq!(vs[1].version, "v2");
    }

    #[test]
    fn prompts_lists_unique_names_sorted() {
        let r = make();
        assert_eq!(
            r.prompts(),
            vec!["support_answer".to_string(), "triage".to_string()]
        );
    }

    #[test]
    fn round_trip_through_json() {
        let r = make();
        let s = serde_json::to_string(&r).unwrap();
        let back: PromptVersionRegistry = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }
}
