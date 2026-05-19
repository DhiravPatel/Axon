//! `axon-a2a` — agent-to-agent discovery & remote calls.
//!
//! v0 ships the **agent-card** half of §54:
//!
//!   * [`AgentCard`] — a typed schema for the JSON document an agent
//!     publishes at `/.well-known/agent-card.json`.
//!   * [`load_card_from_path`] / [`fetch_card`] — local-file and
//!     HTTP-GET discovery.
//!   * [`AgentCard::verify`] — structural validation (required fields,
//!     well-formed URLs, capability/endpoint cross-references).
//!
//! Cryptographic signing of cards lands with §40 secrets. Remote
//! protocol negotiation and on-behalf-of identity ride on top of cards
//! once Stage 15 brings secrets in.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

mod errors;
pub mod identity;

pub use errors::A2aError;
pub use identity::{
    Delegation, IdentityError, KeyPair, Signature, SignedAgentCard, SignedDelegation, TrustStore,
};

pub const CARD_FORMAT_VERSION: u32 = 1;

/// Discovery path appended to a base URL — same as Webfinger/OIDC convention.
pub const WELL_KNOWN_PATH: &str = ".well-known/agent-card.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCard {
    pub format_version: u32,
    pub agent_id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub endpoint: String,
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub auth: AuthScheme,
    #[serde(default)]
    pub pricing: Option<Pricing>,
    #[serde(default)]
    pub rate_limits: Option<RateLimits>,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub name: String,
    pub input_schema_url: Option<String>,
    pub output_schema_url: Option<String>,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scheme", rename_all = "snake_case")]
pub enum AuthScheme {
    None,
    ApiKey { header: String },
    Bearer,
    OAuth2 { authorize_url: String, scopes: Vec<String> },
}

impl Default for AuthScheme {
    fn default() -> Self {
        AuthScheme::None
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pricing {
    /// Cents per call, integer to avoid float in money fields.
    pub per_call_cents: u32,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimits {
    pub requests_per_minute: u32,
    pub burst: u32,
}

impl AgentCard {
    /// Structural validation: required fields are non-empty, URLs look
    /// like URLs, version field is non-empty, capabilities have unique
    /// names.
    pub fn verify(&self) -> Result<(), A2aError> {
        if self.format_version != CARD_FORMAT_VERSION {
            return Err(A2aError::UnsupportedVersion {
                found: self.format_version,
                expected: CARD_FORMAT_VERSION,
            });
        }
        for (label, val) in [
            ("agent_id", &self.agent_id),
            ("name", &self.name),
            ("version", &self.version),
            ("endpoint", &self.endpoint),
        ] {
            if val.is_empty() {
                return Err(A2aError::Invalid(format!("{label} is empty")));
            }
        }
        if !looks_like_url(&self.endpoint) {
            return Err(A2aError::Invalid(format!(
                "endpoint `{}` is not a URL",
                self.endpoint
            )));
        }
        let mut seen: Vec<&str> = Vec::with_capacity(self.capabilities.len());
        for cap in &self.capabilities {
            if cap.name.is_empty() {
                return Err(A2aError::Invalid("capability with empty name".into()));
            }
            if seen.iter().any(|n| *n == cap.name) {
                return Err(A2aError::Invalid(format!(
                    "duplicate capability `{}`",
                    cap.name
                )));
            }
            seen.push(&cap.name);
            for (label, url) in [
                ("input_schema_url", &cap.input_schema_url),
                ("output_schema_url", &cap.output_schema_url),
            ] {
                if let Some(u) = url {
                    if !looks_like_url(u) {
                        return Err(A2aError::Invalid(format!(
                            "capability `{}` {label}=`{u}` is not a URL",
                            cap.name
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, A2aError> {
        serde_json::to_vec_pretty(self).map_err(|e| A2aError::Encode(e.to_string()))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, A2aError> {
        let c: Self = serde_json::from_slice(bytes)
            .map_err(|e| A2aError::Parse(e.to_string()))?;
        c.verify()?;
        Ok(c)
    }

    pub fn capability(&self, name: &str) -> Option<&Capability> {
        self.capabilities.iter().find(|c| c.name == name)
    }
}

/// Load an agent card from a local file path. Used in tests, fixture-based
/// CI, and offline development.
pub fn load_card_from_path(path: impl AsRef<std::path::Path>) -> Result<AgentCard, A2aError> {
    let bytes = std::fs::read(&path)
        .map_err(|e| A2aError::Io(format!("read {}: {e}", path.as_ref().display())))?;
    AgentCard::from_json(&bytes)
}

/// Fetch an agent card from a base URL via HTTPS. Appends [`WELL_KNOWN_PATH`].
/// Uses `ureq` (already a dep for the Anthropic client) — no async runtime
/// required. A 10-second total timeout protects callers from slow peers.
pub fn fetch_card(base_url: &str) -> Result<AgentCard, A2aError> {
    if !looks_like_url(base_url) {
        return Err(A2aError::Invalid(format!(
            "base_url `{base_url}` is not a URL"
        )));
    }
    let url = if base_url.ends_with('/') {
        format!("{base_url}{WELL_KNOWN_PATH}")
    } else {
        format!("{base_url}/{WELL_KNOWN_PATH}")
    };
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();
    let resp = agent
        .get(&url)
        .call()
        .map_err(|e| A2aError::Io(format!("GET {url}: {e}")))?;
    if resp.status() != 200 {
        return Err(A2aError::Invalid(format!(
            "GET {url}: HTTP {}",
            resp.status()
        )));
    }
    let bytes = resp
        .into_string()
        .map_err(|e| A2aError::Io(format!("read body: {e}")))?
        .into_bytes();
    AgentCard::from_json(&bytes)
}

fn looks_like_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> AgentCard {
        AgentCard {
            format_version: CARD_FORMAT_VERSION,
            agent_id: "research-agent-1".into(),
            name: "Research Agent".into(),
            version: "0.1.0".into(),
            description: "Research and summarize docs.".into(),
            endpoint: "https://research.example.com/agent".into(),
            capabilities: vec![Capability {
                name: "Research".into(),
                input_schema_url: Some(
                    "https://research.example.com/schemas/research-input.json".into(),
                ),
                output_schema_url: Some(
                    "https://research.example.com/schemas/research-output.json".into(),
                ),
                description: "Run a literature search and return a structured summary.".into(),
            }],
            auth: AuthScheme::Bearer,
            pricing: Some(Pricing {
                per_call_cents: 5,
                currency: "USD".into(),
            }),
            rate_limits: Some(RateLimits {
                requests_per_minute: 60,
                burst: 10,
            }),
            metadata: Default::default(),
        }
    }

    #[test]
    fn valid_card_passes_verification() {
        fixture().verify().unwrap();
    }

    #[test]
    fn empty_required_field_rejected() {
        let mut c = fixture();
        c.agent_id = String::new();
        assert!(matches!(c.verify().unwrap_err(), A2aError::Invalid(_)));
    }

    #[test]
    fn non_http_endpoint_rejected() {
        let mut c = fixture();
        c.endpoint = "research.example.com".into();
        assert!(matches!(c.verify().unwrap_err(), A2aError::Invalid(_)));
    }

    #[test]
    fn duplicate_capability_rejected() {
        let mut c = fixture();
        c.capabilities.push(c.capabilities[0].clone());
        assert!(matches!(c.verify().unwrap_err(), A2aError::Invalid(_)));
    }

    #[test]
    fn json_round_trip_preserves_card() {
        let c = fixture();
        let bytes = c.to_json().unwrap();
        let back = AgentCard::from_json(&bytes).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn unknown_format_version_rejected() {
        let mut c = fixture();
        c.format_version = 999;
        assert!(matches!(
            c.verify().unwrap_err(),
            A2aError::UnsupportedVersion { .. }
        ));
    }

    #[test]
    fn lookup_by_capability_name() {
        let c = fixture();
        assert!(c.capability("Research").is_some());
        assert!(c.capability("Unknown").is_none());
    }

    #[test]
    fn load_from_local_path_round_trips() {
        let c = fixture();
        let mut tmp = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        tmp.push(format!("agent-card-{}-{}.json", std::process::id(), ts));
        std::fs::write(&tmp, c.to_json().unwrap()).unwrap();
        let loaded = load_card_from_path(&tmp).unwrap();
        assert_eq!(loaded, c);
        let _ = std::fs::remove_file(&tmp);
    }
}
