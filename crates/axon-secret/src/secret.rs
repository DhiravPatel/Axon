//! `Secret<T>` — redaction-aware wrapper.

use std::fmt;

use serde::{Deserialize, Serialize, Serializer};

/// Wraps a value of type `T` so it can travel through the system without
/// being accidentally logged, printed, or serialized in clear.
///
/// * `Debug` and `Display` emit `<redacted>`.
/// * `Serialize` emits the string literal `"<redacted>"` — round-tripping
///   through JSON is intentionally lossy. Use the `Vault` for persistence.
/// * `Deserialize` accepts any value as long as it parses into `T`.
///
/// `PartialEq`/`Eq` are constant-time-ish: they always compare lengths and
/// then bytes, but Rust's `==` on `String` short-circuits on the first
/// difference. For real timing-safe comparison use `expose_for_use` and a
/// dedicated CT-eq library.
#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub struct Secret<T> {
    inner: T,
}

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self { inner: value }
    }

    /// Audit-trail accessor: the name makes it obvious in code review when
    /// a secret is being read out. Returns a borrow so the caller can use
    /// the value transiently without taking ownership.
    pub fn expose_for_use(&self) -> &T {
        &self.inner
    }

    /// Consume the wrapper and return the inner value. Use sparingly — at
    /// this point the value is no longer redaction-protected.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<T> Serialize for Secret<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("<redacted>")
    }
}

impl<T: PartialEq> PartialEq for Secret<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}
impl<T: Eq> Eq for Secret<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_debug_redact() {
        let s = Secret::new("hunter2".to_string());
        assert_eq!(format!("{s}"), "<redacted>");
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
    }

    #[test]
    fn serialize_writes_redacted_marker() {
        let s = Secret::new("hunter2".to_string());
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"<redacted>\"");
    }

    #[test]
    fn expose_for_use_returns_the_real_value() {
        let s = Secret::new(42i64);
        assert_eq!(*s.expose_for_use(), 42);
    }

    #[test]
    fn equality_compares_inner() {
        let a = Secret::new("x".to_string());
        let b = Secret::new("x".to_string());
        let c = Secret::new("y".to_string());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn formatted_string_concatenation_does_not_leak() {
        let s = Secret::new("hunter2".to_string());
        let composed = format!("api_key={s}, debug={s:?}");
        assert!(!composed.contains("hunter2"));
    }
}
