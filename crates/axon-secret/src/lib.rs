//! `axon-secret` — redaction-aware secrets and a filesystem-backed vault.
//!
//! Two v0 surfaces:
//!
//!   * [`Secret<T>`] — a `T` wrapper whose `Debug`, `Display`, and `Serialize`
//!     impls render `<redacted>` instead of the underlying value. The only
//!     way to read it is `expose_for_use(...)` — a clearly-named accessor
//!     so audit trails can flag who called it.
//!
//!   * [`Vault`] — a JSON file (`{ "secrets": { name: value, ... } }`)
//!     protected by Unix file permissions (mode `0600` enforced on save and
//!     verified on load). Confidentiality at rest relies on filesystem
//!     permissions, which is consistent with how OpenSSH and most CLI tools
//!     ship private keys today; encrypted-at-rest is a v1 enhancement.
//!
//! Both pieces participate in the same `Secret<T>` redaction so casual
//! debugging never leaks a value.

pub mod oauth;
pub mod secret;
pub mod vault;

pub use oauth::{OauthToken, RefreshOutcome, TokenRefreshError};
pub use secret::Secret;
pub use vault::{Vault, VaultError};
