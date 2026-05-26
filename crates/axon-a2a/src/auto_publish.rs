//! Agent card auto-publication (§54.1).
//!
//! The §54 spec promises that running an Axon agent through
//! `axon serve --protocol a2a` auto-derives an [`crate::AgentCard`]
//! from the agent declaration and serves it at
//! `/.well-known/agent-card.json`. This module ships the derivation +
//! the canonical well-known path constant + a small HTTP body helper
//! the deploy layer routes to.
//!
//! Why not a macro? An agent declaration's *types* aren't accessible
//! from regular Axon code at runtime; the parser/tyck *do* know them.
//! We model the input as `AgentSummary` — a typed snapshot of what the
//! compiler knows about an `agent` declaration — and let the host
//! produce it. Today the host populates a `Vec<HandlerSummary>` from
//! the parsed `Agent` item; tomorrow a parser-level derive can flow
//! into the same struct without changing the publication path.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{AgentCard, AuthScheme, Capability, CARD_FORMAT_VERSION};

/// What the compiler knows about an `agent` declaration after
/// type-checking — enough to derive a public-facing AgentCard from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSummary {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Public-facing message handlers — these become the AgentCard
    /// capabilities. The host strips private (`fn` without `pub`)
    /// handlers before passing the summary in.
    pub handlers: Vec<HandlerSummary>,
    /// Optional auth scheme; `None` defaults to `AuthScheme::None`.
    #[serde(default)]
    pub auth: Option<AuthScheme>,
    /// Optional path to JSON schemas served by the agent for input/
    /// output. The host fills these from `schema MyType { ... }`
    /// declarations.
    #[serde(default)]
    pub schemas: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandlerSummary {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Name of the input schema (looked up in `AgentSummary::schemas`
    /// → URL). `None` means "free-form JSON".
    #[serde(default)]
    pub input_schema: Option<String>,
    #[serde(default)]
    pub output_schema: Option<String>,
}

/// Derive an `AgentCard` from a compile-time summary and the agent's
/// HTTP base URL. The endpoint follows the §54.1 convention
/// `<base>/agent` so callers can find both the card *and* the JSON-RPC
/// endpoint deterministically.
pub fn derive_agent_card(summary: &AgentSummary, base_url: &str) -> AgentCard {
    let endpoint = if base_url.ends_with('/') {
        format!("{base_url}agent")
    } else {
        format!("{base_url}/agent")
    };
    let capabilities: Vec<Capability> = summary
        .handlers
        .iter()
        .map(|h| Capability {
            name: h.name.clone(),
            input_schema_url: h
                .input_schema
                .as_ref()
                .and_then(|name| summary.schemas.get(name).cloned()),
            output_schema_url: h
                .output_schema
                .as_ref()
                .and_then(|name| summary.schemas.get(name).cloned()),
            description: h.description.clone(),
        })
        .collect();
    AgentCard {
        format_version: CARD_FORMAT_VERSION,
        agent_id: format!("{}-{}", summary.name, summary.version),
        name: summary.name.clone(),
        version: summary.version.clone(),
        description: summary.description.clone(),
        endpoint,
        capabilities,
        auth: summary.auth.clone().unwrap_or(AuthScheme::None),
        pricing: None,
        rate_limits: None,
        metadata: BTreeMap::new(),
    }
}

/// Discovery path appended to a base URL. Same as
/// [`crate::WELL_KNOWN_PATH`], re-exported here so the serve layer
/// doesn't have to plumb both names around.
pub use crate::WELL_KNOWN_PATH;

/// Build the JSON body the serve layer returns from
/// `GET /.well-known/agent-card.json`. Pretty-printed for human
/// inspection. Errors only on the (unreachable) JSON-encode failure.
pub fn render_well_known(card: &AgentCard) -> Result<Vec<u8>, String> {
    card.to_json().map_err(|e| format!("encode: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> AgentSummary {
        let mut schemas = BTreeMap::new();
        schemas.insert(
            "ResearchInput".into(),
            "https://example.com/schemas/research-in.json".into(),
        );
        schemas.insert(
            "ResearchOutput".into(),
            "https://example.com/schemas/research-out.json".into(),
        );
        AgentSummary {
            name: "Research".into(),
            version: "1.2.3".into(),
            description: "Research and summarize docs.".into(),
            handlers: vec![
                HandlerSummary {
                    name: "Research".into(),
                    description: "Run a literature search.".into(),
                    input_schema: Some("ResearchInput".into()),
                    output_schema: Some("ResearchOutput".into()),
                },
                HandlerSummary {
                    name: "Summarize".into(),
                    description: "Summarize prior text.".into(),
                    input_schema: None,
                    output_schema: None,
                },
            ],
            auth: Some(AuthScheme::Bearer),
            schemas,
        }
    }

    #[test]
    fn derive_produces_well_formed_card() {
        let s = summary();
        let card = derive_agent_card(&s, "https://research.example.com");
        card.verify().unwrap();
        assert_eq!(card.name, "Research");
        assert_eq!(card.endpoint, "https://research.example.com/agent");
        assert_eq!(card.capabilities.len(), 2);
        assert_eq!(
            card.capabilities[0].input_schema_url.as_deref(),
            Some("https://example.com/schemas/research-in.json")
        );
        assert!(matches!(card.auth, AuthScheme::Bearer));
    }

    #[test]
    fn trailing_slash_on_base_url_handled() {
        let card = derive_agent_card(&summary(), "https://x.example.com/");
        assert_eq!(card.endpoint, "https://x.example.com/agent");
    }

    #[test]
    fn missing_schema_is_dropped_to_none() {
        let mut s = summary();
        s.schemas.clear();
        let card = derive_agent_card(&s, "https://x.example.com");
        assert!(card.capabilities[0].input_schema_url.is_none());
    }

    #[test]
    fn auth_defaults_to_none_when_unset() {
        let mut s = summary();
        s.auth = None;
        let card = derive_agent_card(&s, "https://x.example.com");
        assert!(matches!(card.auth, AuthScheme::None));
    }

    #[test]
    fn render_well_known_returns_valid_json() {
        let card = derive_agent_card(&summary(), "https://example.com");
        let body = render_well_known(&card).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["name"].as_str(), Some("Research"));
    }

    #[test]
    fn well_known_path_constant_matches() {
        assert_eq!(WELL_KNOWN_PATH, ".well-known/agent-card.json");
    }
}
