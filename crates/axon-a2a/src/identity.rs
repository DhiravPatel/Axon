//! Ed25519 signed identity for agent cards (§54.2).
//!
//! Stage 22 closes the cross-org trust gap. Every served agent now has
//! a public/private Ed25519 keypair; published cards are wrapped in a
//! [`SignedAgentCard`] envelope containing:
//!
//!   * `card_json`: the bytes that were signed (the canonical JSON
//!     serialization of the underlying [`AgentCard`]).
//!   * `signature_hex`: the 64-byte Ed25519 signature, hex-encoded.
//!   * `signer_pubkey_hex`: the 32-byte verifying key, hex-encoded.
//!
//! Consumers discover a card via [`SignedAgentCard::verify`] against a
//! [`TrustStore`] — a set of public keys the program is willing to
//! accept. Anything else fails closed.
//!
//! v0 limits: Ed25519 with raw keys, no certificate chain, no key
//! rotation. PKI / delegation (`on_behalf_of`) lands in Stage 23.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::AgentCard;

/// Wrapper around a freshly-generated or restored signing key.
#[derive(Clone)]
pub struct KeyPair {
    inner: SigningKey,
}

impl KeyPair {
    /// Generate a fresh keypair using the OS RNG.
    pub fn generate() -> Self {
        let mut rng = rand_core::OsRng;
        Self {
            inner: SigningKey::generate(&mut rng),
        }
    }

    /// Restore from a 32-byte seed (the Ed25519 private scalar). Used
    /// to round-trip a saved keypair from disk / vault.
    pub fn from_seed_bytes(seed: &[u8; 32]) -> Self {
        Self {
            inner: SigningKey::from_bytes(seed),
        }
    }

    /// 32-byte seed (the private key material). Treat as a `Secret<T>` —
    /// the caller should never log or print this.
    pub fn seed_bytes(&self) -> [u8; 32] {
        self.inner.to_bytes()
    }

    /// 64-char hex of the seed. Same caveat as [`Self::seed_bytes`].
    pub fn seed_hex(&self) -> String {
        hex_encode(&self.seed_bytes())
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.inner.verifying_key()
    }

    pub fn pubkey_hex(&self) -> String {
        hex_encode(self.verifying_key().as_bytes())
    }

    /// Sign `data` with this keypair, returning a hex-encoded 64-byte
    /// signature.
    pub fn sign_hex(&self, data: &[u8]) -> String {
        hex_encode(&self.inner.sign(data).to_bytes())
    }

    /// Sign an [`AgentCard`]: serialize the card to canonical JSON,
    /// sign those bytes, and wrap in a [`SignedAgentCard`].
    pub fn sign_card(&self, card: &AgentCard) -> Result<SignedAgentCard, IdentityError> {
        let card_json = canonical_card_json(card)?;
        let signature_hex = self.sign_hex(card_json.as_bytes());
        Ok(SignedAgentCard {
            card_json,
            signature_hex,
            signer_pubkey_hex: self.pubkey_hex(),
        })
    }
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never leak the private key material into Debug output.
        f.debug_struct("KeyPair")
            .field("pubkey_hex", &self.pubkey_hex())
            .field("seed_bytes", &"<redacted>")
            .finish()
    }
}

/// Detached 64-byte Ed25519 signature in hex form. Wrapped in a struct
/// so the public surface can later add expiry/scope without breaking the
/// shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAgentCard {
    /// Canonical JSON serialization of the inner [`AgentCard`] — the
    /// exact bytes that were signed. Verifiers MUST re-hash these bytes
    /// rather than re-serializing the card themselves (different JSON
    /// libraries reorder keys).
    pub card_json: String,
    /// 64-byte Ed25519 signature over `card_json`, hex-encoded.
    pub signature_hex: String,
    /// 32-byte verifying key (the signer's pubkey), hex-encoded.
    pub signer_pubkey_hex: String,
}

impl SignedAgentCard {
    /// Verify the signature against `trust`. Returns the inner card on
    /// success.
    pub fn verify(&self, trust: &TrustStore) -> Result<AgentCard, IdentityError> {
        let signer = hex_decode(&self.signer_pubkey_hex)
            .map_err(|e| IdentityError::BadHex(format!("signer_pubkey: {e}")))?;
        if signer.len() != 32 {
            return Err(IdentityError::BadKey(format!(
                "signer_pubkey must be 32 bytes, got {}",
                signer.len()
            )));
        }
        let mut sk: [u8; 32] = [0u8; 32];
        sk.copy_from_slice(&signer);
        if !trust.allows(&sk) {
            return Err(IdentityError::Untrusted(self.signer_pubkey_hex.clone()));
        }
        let key = VerifyingKey::from_bytes(&sk)
            .map_err(|e| IdentityError::BadKey(e.to_string()))?;
        let sig_bytes = hex_decode(&self.signature_hex)
            .map_err(|e| IdentityError::BadHex(format!("signature: {e}")))?;
        if sig_bytes.len() != 64 {
            return Err(IdentityError::BadSignature(format!(
                "signature must be 64 bytes, got {}",
                sig_bytes.len()
            )));
        }
        let mut sig_arr: [u8; 64] = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);
        let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
        key.verify(self.card_json.as_bytes(), &sig)
            .map_err(|e| IdentityError::BadSignature(e.to_string()))?;
        // Signature checks out — parse the inner card.
        let card: AgentCard = serde_json::from_str(&self.card_json)
            .map_err(|e| IdentityError::BadCard(e.to_string()))?;
        card.verify()?;
        Ok(card)
    }

    /// Load a signed card from a JSON file and verify it.
    pub fn load_and_verify(
        path: impl AsRef<std::path::Path>,
        trust: &TrustStore,
    ) -> Result<AgentCard, IdentityError> {
        let bytes = std::fs::read(path).map_err(|e| IdentityError::Io(e.to_string()))?;
        let signed: SignedAgentCard =
            serde_json::from_slice(&bytes).map_err(|e| IdentityError::Parse(e.to_string()))?;
        signed.verify(trust)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, IdentityError> {
        serde_json::to_vec_pretty(self).map_err(|e| IdentityError::Parse(e.to_string()))
    }
}

/// Allowlist of public keys this program will trust as agent-card
/// signers. The default is empty — every program must explicitly add a
/// pubkey before any signed card will verify.
#[derive(Clone, Debug, Default)]
pub struct TrustStore {
    keys: Vec<[u8; 32]>,
}

impl TrustStore {
    pub fn new() -> Self {
        Self { keys: Vec::new() }
    }

    /// Add a verifying key from a 64-char hex string.
    pub fn add_hex(&mut self, hex: &str) -> Result<(), IdentityError> {
        let bytes = hex_decode(hex).map_err(|e| IdentityError::BadHex(e))?;
        if bytes.len() != 32 {
            return Err(IdentityError::BadKey(format!(
                "verifying key must be 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        if !self.keys.iter().any(|k| k == &arr) {
            self.keys.push(arr);
        }
        Ok(())
    }

    pub fn allows(&self, key: &[u8; 32]) -> bool {
        self.keys.iter().any(|k| k == key)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[derive(Debug)]
pub enum IdentityError {
    Io(String),
    Parse(String),
    BadHex(String),
    BadKey(String),
    BadSignature(String),
    BadCard(String),
    /// Signature checked out but the signer's pubkey isn't in the trust
    /// store. Returns the offending hex so the operator can decide
    /// whether to add it.
    Untrusted(String),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::Io(m) => write!(f, "identity I/O: {m}"),
            IdentityError::Parse(m) => write!(f, "identity parse: {m}"),
            IdentityError::BadHex(m) => write!(f, "identity bad hex: {m}"),
            IdentityError::BadKey(m) => write!(f, "identity bad key: {m}"),
            IdentityError::BadSignature(m) => write!(f, "identity signature invalid: {m}"),
            IdentityError::BadCard(m) => write!(f, "identity inner card invalid: {m}"),
            IdentityError::Untrusted(k) => write!(
                f,
                "identity: signer pubkey `{k}` is not in the trust store"
            ),
        }
    }
}
impl std::error::Error for IdentityError {}

impl From<crate::A2aError> for IdentityError {
    fn from(e: crate::A2aError) -> Self {
        IdentityError::BadCard(e.to_string())
    }
}

// ---- helpers ----------------------------------------------------------

fn canonical_card_json(card: &AgentCard) -> Result<String, IdentityError> {
    // `serde_json::to_string` on a struct with a fixed field order
    // produces a stable byte sequence — that's what we sign. Verifiers
    // re-hash the stored `card_json` rather than re-serializing, so this
    // function doesn't need to do anything cleverer than vanilla serde.
    serde_json::to_string(card).map_err(|e| IdentityError::Parse(e.to_string()))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err(format!("odd-length hex string ({})", s.len()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("bad hex digit `{}`", b as char)),
    }
}

// ---------------------------------------------------------------------------
// §54.2 Delegated identity
// ---------------------------------------------------------------------------

/// What the holder of this delegation is allowed to do on behalf of
/// `principal`. Scopes are free-form strings ("repo:read", "billing:refund")
/// — the receiving agent decides what set it accepts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delegation {
    /// Stable identifier of the user the delegation acts on behalf of
    /// (e.g. `"user:alice@acme.com"` or a UUID).
    pub principal: String,
    /// Audience: the agent or service this delegation is meant for.
    /// Verifiers MUST check that the audience matches what they expect
    /// — otherwise a delegation issued to agent X could be replayed
    /// against agent Y.
    pub audience: String,
    /// Scoped capability set.
    pub scopes: Vec<String>,
    /// Unix-epoch seconds at which the delegation stops being valid.
    pub expires_at_secs: i64,
    /// Replay-protection nonce. The receiving agent SHOULD remember
    /// recently-seen nonces and reject duplicates.
    #[serde(default)]
    pub nonce: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedDelegation {
    /// Canonical JSON of the inner [`Delegation`] — the exact bytes
    /// that were signed.
    pub delegation_json: String,
    pub signature_hex: String,
    pub signer_pubkey_hex: String,
}

impl KeyPair {
    pub fn sign_delegation(
        &self,
        delegation: &Delegation,
    ) -> Result<SignedDelegation, IdentityError> {
        let delegation_json = serde_json::to_string(delegation)
            .map_err(|e| IdentityError::Parse(e.to_string()))?;
        let signature_hex = self.sign_hex(delegation_json.as_bytes());
        Ok(SignedDelegation {
            delegation_json,
            signature_hex,
            signer_pubkey_hex: self.pubkey_hex(),
        })
    }
}

impl SignedDelegation {
    /// Verify the delegation against `trust` (the set of pubkeys whose
    /// delegations the verifier is willing to accept) for `audience`
    /// (the receiving agent's identifier) at `now_secs` (typically the
    /// current wall clock).
    ///
    /// Returns the inner [`Delegation`] on success. The audience check
    /// is part of the contract — passing the wrong audience produces a
    /// `BadCard` error so a delegation cannot be replayed cross-agent.
    pub fn verify(
        &self,
        trust: &TrustStore,
        expected_audience: &str,
        now_secs: i64,
    ) -> Result<Delegation, IdentityError> {
        let signer = hex_decode(&self.signer_pubkey_hex)
            .map_err(|e| IdentityError::BadHex(format!("signer_pubkey: {e}")))?;
        if signer.len() != 32 {
            return Err(IdentityError::BadKey(format!(
                "signer_pubkey must be 32 bytes, got {}",
                signer.len()
            )));
        }
        let mut sk = [0u8; 32];
        sk.copy_from_slice(&signer);
        if !trust.allows(&sk) {
            return Err(IdentityError::Untrusted(self.signer_pubkey_hex.clone()));
        }
        let key = VerifyingKey::from_bytes(&sk)
            .map_err(|e| IdentityError::BadKey(e.to_string()))?;
        let sig_bytes = hex_decode(&self.signature_hex)
            .map_err(|e| IdentityError::BadHex(format!("signature: {e}")))?;
        if sig_bytes.len() != 64 {
            return Err(IdentityError::BadSignature(format!(
                "signature must be 64 bytes, got {}",
                sig_bytes.len()
            )));
        }
        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);
        let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
        key.verify(self.delegation_json.as_bytes(), &sig)
            .map_err(|e| IdentityError::BadSignature(e.to_string()))?;
        let d: Delegation = serde_json::from_str(&self.delegation_json)
            .map_err(|e| IdentityError::BadCard(e.to_string()))?;
        if d.audience != expected_audience {
            return Err(IdentityError::BadCard(format!(
                "delegation audience `{}` doesn't match expected `{}`",
                d.audience, expected_audience
            )));
        }
        if now_secs >= d.expires_at_secs {
            return Err(IdentityError::BadCard(format!(
                "delegation expired at {} (now {})",
                d.expires_at_secs, now_secs
            )));
        }
        Ok(d)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, IdentityError> {
        serde_json::to_vec_pretty(self).map_err(|e| IdentityError::Parse(e.to_string()))
    }

    pub fn load_and_verify(
        path: impl AsRef<std::path::Path>,
        trust: &TrustStore,
        expected_audience: &str,
        now_secs: i64,
    ) -> Result<Delegation, IdentityError> {
        let bytes = std::fs::read(path).map_err(|e| IdentityError::Io(e.to_string()))?;
        let signed: SignedDelegation =
            serde_json::from_slice(&bytes).map_err(|e| IdentityError::Parse(e.to_string()))?;
        signed.verify(trust, expected_audience, now_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentCard, AuthScheme, Capability, CARD_FORMAT_VERSION};

    fn fixture_card() -> AgentCard {
        AgentCard {
            format_version: CARD_FORMAT_VERSION,
            agent_id: "researcher-1".into(),
            name: "Research".into(),
            version: "1.0.0".into(),
            description: "demo".into(),
            endpoint: "https://example.com/agent".into(),
            capabilities: vec![Capability {
                name: "Research".into(),
                input_schema_url: None,
                output_schema_url: None,
                description: String::new(),
            }],
            auth: AuthScheme::None,
            pricing: None,
            rate_limits: None,
            metadata: Default::default(),
        }
    }

    #[test]
    fn keypair_round_trips_via_seed() {
        let k = KeyPair::generate();
        let seed = k.seed_bytes();
        let restored = KeyPair::from_seed_bytes(&seed);
        assert_eq!(k.pubkey_hex(), restored.pubkey_hex());
    }

    #[test]
    fn signing_and_verifying_round_trip() {
        let k = KeyPair::generate();
        let mut trust = TrustStore::new();
        trust.add_hex(&k.pubkey_hex()).unwrap();
        let card = fixture_card();
        let signed = k.sign_card(&card).unwrap();
        let verified = signed.verify(&trust).unwrap();
        assert_eq!(verified, card);
    }

    #[test]
    fn untrusted_signer_is_rejected() {
        let signer = KeyPair::generate();
        let attacker = KeyPair::generate();
        let mut trust = TrustStore::new();
        // Trust only the attacker — signature from `signer` should be
        // rejected even though it's mathematically valid.
        trust.add_hex(&attacker.pubkey_hex()).unwrap();
        let signed = signer.sign_card(&fixture_card()).unwrap();
        let err = signed.verify(&trust).unwrap_err();
        assert!(matches!(err, IdentityError::Untrusted(_)));
    }

    #[test]
    fn tampered_card_json_fails_signature_check() {
        let k = KeyPair::generate();
        let mut trust = TrustStore::new();
        trust.add_hex(&k.pubkey_hex()).unwrap();
        let mut signed = k.sign_card(&fixture_card()).unwrap();
        // Flip one byte of the canonical JSON without re-signing.
        let mut bytes = signed.card_json.into_bytes();
        for b in &mut bytes {
            if *b == b'1' {
                *b = b'2';
                break;
            }
        }
        signed.card_json = String::from_utf8(bytes).unwrap();
        let err = signed.verify(&trust).unwrap_err();
        assert!(matches!(err, IdentityError::BadSignature(_)));
    }

    #[test]
    fn trust_store_dedups_repeated_adds() {
        let k = KeyPair::generate();
        let mut trust = TrustStore::new();
        trust.add_hex(&k.pubkey_hex()).unwrap();
        trust.add_hex(&k.pubkey_hex()).unwrap();
        assert_eq!(trust.len(), 1);
    }

    #[test]
    fn add_hex_rejects_short_keys() {
        let mut trust = TrustStore::new();
        let err = trust.add_hex("aabb").unwrap_err();
        assert!(matches!(err, IdentityError::BadKey(_)));
    }

    #[test]
    fn load_and_verify_round_trips_through_disk() {
        let k = KeyPair::generate();
        let mut trust = TrustStore::new();
        trust.add_hex(&k.pubkey_hex()).unwrap();
        let signed = k.sign_card(&fixture_card()).unwrap();
        let mut p = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!("signed-card-{}-{ts}.json", std::process::id()));
        std::fs::write(&p, signed.to_json().unwrap()).unwrap();
        let card = SignedAgentCard::load_and_verify(&p, &trust).unwrap();
        assert_eq!(card.agent_id, "researcher-1");
        let _ = std::fs::remove_file(&p);
    }

    // ---- Delegated identity (§54.2) -----------------------------------

    fn fixture_delegation(audience: &str, expires_in_secs: i64) -> Delegation {
        Delegation {
            principal: "user:alice@acme.com".into(),
            audience: audience.into(),
            scopes: vec!["repo:read".into(), "issues:write".into()],
            expires_at_secs: now_secs() + expires_in_secs,
            nonce: "nonce-xyz".into(),
        }
    }

    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    #[test]
    fn delegation_round_trip_verifies_for_intended_audience() {
        let user_key = KeyPair::generate();
        let mut trust = TrustStore::new();
        trust.add_hex(&user_key.pubkey_hex()).unwrap();
        let d = fixture_delegation("agent:research", 3600);
        let signed = user_key.sign_delegation(&d).unwrap();
        let back = signed.verify(&trust, "agent:research", now_secs()).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn delegation_for_different_audience_is_rejected() {
        let user_key = KeyPair::generate();
        let mut trust = TrustStore::new();
        trust.add_hex(&user_key.pubkey_hex()).unwrap();
        let d = fixture_delegation("agent:research", 3600);
        let signed = user_key.sign_delegation(&d).unwrap();
        // The delegation was for `agent:research`; replay it against
        // `agent:billing` and expect a clean rejection.
        let err = signed
            .verify(&trust, "agent:billing", now_secs())
            .unwrap_err();
        assert!(matches!(err, IdentityError::BadCard(_)));
    }

    #[test]
    fn expired_delegation_is_rejected() {
        let user_key = KeyPair::generate();
        let mut trust = TrustStore::new();
        trust.add_hex(&user_key.pubkey_hex()).unwrap();
        let d = fixture_delegation("agent:research", -10); // expired 10s ago
        let signed = user_key.sign_delegation(&d).unwrap();
        let err = signed
            .verify(&trust, "agent:research", now_secs())
            .unwrap_err();
        assert!(matches!(err, IdentityError::BadCard(_)));
    }

    #[test]
    fn delegation_signed_by_untrusted_key_is_rejected() {
        let user_key = KeyPair::generate();
        let attacker = KeyPair::generate();
        let mut trust = TrustStore::new();
        trust.add_hex(&attacker.pubkey_hex()).unwrap();
        let d = fixture_delegation("agent:research", 3600);
        let signed = user_key.sign_delegation(&d).unwrap();
        let err = signed
            .verify(&trust, "agent:research", now_secs())
            .unwrap_err();
        assert!(matches!(err, IdentityError::Untrusted(_)));
    }

    #[test]
    fn tampered_delegation_json_fails_signature() {
        let user_key = KeyPair::generate();
        let mut trust = TrustStore::new();
        trust.add_hex(&user_key.pubkey_hex()).unwrap();
        let d = fixture_delegation("agent:research", 3600);
        let mut signed = user_key.sign_delegation(&d).unwrap();
        // Add a scope after signing.
        signed.delegation_json =
            signed.delegation_json.replace("issues:write", "billing:refund");
        let err = signed
            .verify(&trust, "agent:research", now_secs())
            .unwrap_err();
        assert!(matches!(err, IdentityError::BadSignature(_)));
    }
}
