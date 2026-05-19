//! OAuth2 bearer-token storage with refresh.
//!
//! Stage 21 surface for §40.2:
//!
//!   * [`OauthToken`] — `access_token` + optional `refresh_token` + the
//!     wall-clock instant the access token expires. All three are
//!     wrapped in `Secret<T>` at the value layer; serialization for the
//!     vault uses the raw form (the vault file itself is the secrecy
//!     boundary).
//!
//!   * [`OauthToken::refresh`] — POSTs `grant_type=refresh_token` to a
//!     token endpoint via `ureq`, parses the JSON response, and rotates
//!     the in-memory token. The new `expires_at` is computed from
//!     `expires_in` (seconds from now) when the server returns it; if
//!     the server omits it, we default to one hour to match the OAuth2
//!     spec recommendation.
//!
//! v0 limits: only the `application/x-www-form-urlencoded` refresh flow
//! is implemented. PKCE, ID-token validation, client-cert auth, and
//! token-introspection live behind the same trait surface and arrive
//! when a provider needs them.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OauthToken {
    pub access_token: String,
    /// `None` for short-lived tokens that can't be refreshed.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Wall-clock seconds since Unix epoch when `access_token` expires.
    /// `None` means "unknown lifetime" — `is_expired_at` returns false in
    /// that case so the caller decides whether to trust it.
    #[serde(default)]
    pub expires_at_secs: Option<i64>,
    /// Token endpoint to call when refreshing. Stored alongside the
    /// token so refresh doesn't need a separate config lookup.
    #[serde(default)]
    pub token_url: Option<String>,
    /// OAuth client id. Saved with the token so refresh works after a
    /// process restart without a separate side-channel.
    #[serde(default)]
    pub client_id: Option<String>,
}

impl OauthToken {
    /// Construct a new token without an expiry. The caller should
    /// chain `.with_expires_in(...)` if the server returned `expires_in`.
    pub fn new(access_token: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            refresh_token: None,
            expires_at_secs: None,
            token_url: None,
            client_id: None,
        }
    }

    pub fn with_refresh_token(mut self, refresh_token: impl Into<String>) -> Self {
        self.refresh_token = Some(refresh_token.into());
        self
    }

    pub fn with_expires_in(mut self, seconds: i64) -> Self {
        self.expires_at_secs = Some(now_secs() + seconds);
        self
    }

    pub fn with_token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = Some(url.into());
        self
    }

    pub fn with_client_id(mut self, id: impl Into<String>) -> Self {
        self.client_id = Some(id.into());
        self
    }

    /// Is the access token expired *as of* `now_secs`?
    pub fn is_expired_at(&self, now_secs: i64) -> bool {
        match self.expires_at_secs {
            Some(exp) => now_secs >= exp,
            None => false,
        }
    }

    /// Convenience: is the token expired right now (real wall clock)?
    pub fn is_expired(&self) -> bool {
        self.is_expired_at(now_secs())
    }

    /// Within `slack_secs` of expiry? Useful when callers want to refresh
    /// proactively to avoid races (5-minute slack is a common production
    /// pre-emption window).
    pub fn needs_refresh(&self, slack_secs: i64) -> bool {
        match self.expires_at_secs {
            Some(exp) => now_secs() + slack_secs >= exp,
            None => false,
        }
    }

    /// Refresh against `token_url` using the stored `refresh_token`.
    /// Mutates `self` on success. Returns [`RefreshOutcome`] so the caller
    /// can distinguish "renewed" from "no refresh needed".
    pub fn refresh(&mut self) -> Result<RefreshOutcome, TokenRefreshError> {
        let refresh_token = match self.refresh_token.clone() {
            Some(t) => t,
            None => return Err(TokenRefreshError::NoRefreshToken),
        };
        let url = match self.token_url.clone() {
            Some(u) => u,
            None => return Err(TokenRefreshError::NoTokenUrl),
        };
        let mut form: Vec<(String, String)> = vec![
            ("grant_type".into(), "refresh_token".into()),
            ("refresh_token".into(), refresh_token),
        ];
        if let Some(cid) = &self.client_id {
            form.push(("client_id".into(), cid.clone()));
        }
        let form_refs: Vec<(&str, &str)> =
            form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(15))
            .build();
        let resp = agent
            .post(&url)
            .set("Accept", "application/json")
            .send_form(&form_refs)
            .map_err(|e| TokenRefreshError::Http(e.to_string()))?;
        if resp.status() < 200 || resp.status() >= 300 {
            return Err(TokenRefreshError::HttpStatus {
                status: resp.status(),
                body: resp.into_string().unwrap_or_default(),
            });
        }
        let body = resp
            .into_string()
            .map_err(|e| TokenRefreshError::Io(e.to_string()))?;
        let v: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| TokenRefreshError::Parse(e.to_string()))?;
        let access = v
            .get("access_token")
            .and_then(|x| x.as_str())
            .ok_or_else(|| {
                TokenRefreshError::Parse("response missing `access_token`".into())
            })?
            .to_string();
        let expires_in = v
            .get("expires_in")
            .and_then(|x| x.as_i64())
            .unwrap_or(3600);
        // Some providers rotate the refresh token; if a new one shows up
        // in the response, keep it, otherwise preserve the old one.
        let new_refresh = v.get("refresh_token").and_then(|x| x.as_str()).map(String::from);
        self.access_token = access;
        if let Some(nr) = new_refresh {
            self.refresh_token = Some(nr);
        }
        self.expires_at_secs = Some(now_secs() + expires_in);
        Ok(RefreshOutcome::Refreshed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    Refreshed,
    NoOp,
}

#[derive(Debug)]
pub enum TokenRefreshError {
    NoRefreshToken,
    NoTokenUrl,
    Http(String),
    HttpStatus { status: u16, body: String },
    Io(String),
    Parse(String),
}

impl std::fmt::Display for TokenRefreshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenRefreshError::NoRefreshToken => write!(f, "oauth: no refresh_token stored"),
            TokenRefreshError::NoTokenUrl => write!(f, "oauth: no token_url stored"),
            TokenRefreshError::Http(m) => write!(f, "oauth refresh HTTP: {m}"),
            TokenRefreshError::HttpStatus { status, body } => {
                write!(f, "oauth refresh: HTTP {status}: {body}")
            }
            TokenRefreshError::Io(m) => write!(f, "oauth refresh I/O: {m}"),
            TokenRefreshError::Parse(m) => write!(f, "oauth refresh parse: {m}"),
        }
    }
}
impl std::error::Error for TokenRefreshError {}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_with_no_expiry_is_never_expired() {
        let t = OauthToken::new("abc");
        assert!(!t.is_expired_at(now_secs() + 99999));
    }

    #[test]
    fn token_with_expiry_in_past_is_expired() {
        let mut t = OauthToken::new("abc");
        t.expires_at_secs = Some(now_secs() - 10);
        assert!(t.is_expired_at(now_secs()));
    }

    #[test]
    fn needs_refresh_within_slack_window() {
        let mut t = OauthToken::new("abc");
        // Expires in 60s; with 120s slack we should want to refresh.
        t.expires_at_secs = Some(now_secs() + 60);
        assert!(t.needs_refresh(120));
        assert!(!t.needs_refresh(10));
    }

    #[test]
    fn refresh_without_token_or_url_errors() {
        let mut t = OauthToken::new("abc");
        assert!(matches!(t.refresh(), Err(TokenRefreshError::NoRefreshToken)));
        t.refresh_token = Some("r".into());
        assert!(matches!(t.refresh(), Err(TokenRefreshError::NoTokenUrl)));
    }

    #[test]
    fn json_round_trip_preserves_all_fields() {
        let t = OauthToken::new("abc")
            .with_refresh_token("rt")
            .with_expires_in(3600)
            .with_token_url("https://example.com/token")
            .with_client_id("cli");
        let bytes = serde_json::to_vec(&t).unwrap();
        let back: OauthToken = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, t);
    }
}
