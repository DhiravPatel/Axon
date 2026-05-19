//! Filesystem-protected vault.
//!
//! Layout on disk:
//! ```json
//! {
//!   "version": 1,
//!   "secrets": { "ANTHROPIC_KEY": "sk-ant-...", "DB_PASSWORD": "..." }
//! }
//! ```
//!
//! On Unix the file is created with mode `0600` (owner-only read/write).
//! On load, the runtime verifies the mode and refuses to read a vault that's
//! world- or group-readable — that's the "filesystem permissions are the
//! confidentiality barrier" contract.
//!
//! Values come back wrapped in [`crate::Secret`] so they can't accidentally
//! leak through logs.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Secret;

pub const VAULT_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Vault {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

fn default_version() -> u32 {
    VAULT_VERSION
}

#[derive(Debug)]
pub enum VaultError {
    Io(String),
    Parse(String),
    Encode(String),
    /// On Unix, the file's mode allows read/write access by group or world.
    /// The vault refuses to load until the user fixes the permission.
    InsecurePermissions { path: PathBuf, mode: u32 },
    UnsupportedVersion { found: u32, expected: u32 },
    NotFound(String),
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VaultError::Io(m) => write!(f, "vault I/O: {m}"),
            VaultError::Parse(m) => write!(f, "vault parse: {m}"),
            VaultError::Encode(m) => write!(f, "vault encode: {m}"),
            VaultError::InsecurePermissions { path, mode } => write!(
                f,
                "vault `{}` has insecure permissions (mode {:o}). \
                Run `chmod 600 {}` and try again.",
                path.display(),
                mode,
                path.display()
            ),
            VaultError::UnsupportedVersion { found, expected } => write!(
                f,
                "vault: format version mismatch (found={found}, expected={expected})"
            ),
            VaultError::NotFound(name) => write!(f, "vault: secret `{name}` not found"),
        }
    }
}

impl std::error::Error for VaultError {}

impl Vault {
    pub fn new() -> Self {
        Self {
            version: VAULT_VERSION,
            secrets: BTreeMap::new(),
        }
    }

    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.secrets.insert(name.into(), value.into());
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.secrets.remove(name).is_some()
    }

    pub fn names(&self) -> Vec<String> {
        self.secrets.keys().cloned().collect()
    }

    pub fn get(&self, name: &str) -> Result<Secret<String>, VaultError> {
        self.secrets
            .get(name)
            .map(|v| Secret::new(v.clone()))
            .ok_or_else(|| VaultError::NotFound(name.to_string()))
    }

    // ---- OAuth-shaped accessors -------------------------------------

    /// Store an OAuth token. The token's JSON shape is written under
    /// the key `oauth:{name}` so it lives alongside plain secrets in the
    /// same vault file without a schema change.
    pub fn set_oauth(&mut self, name: &str, token: &crate::oauth::OauthToken) -> Result<(), VaultError> {
        let json = serde_json::to_string(token).map_err(|e| VaultError::Encode(e.to_string()))?;
        self.secrets.insert(format!("oauth:{name}"), json);
        Ok(())
    }

    /// Load an OAuth token previously stored with [`set_oauth`].
    pub fn get_oauth(&self, name: &str) -> Result<crate::oauth::OauthToken, VaultError> {
        let key = format!("oauth:{name}");
        let raw = self
            .secrets
            .get(&key)
            .ok_or_else(|| VaultError::NotFound(key.clone()))?;
        serde_json::from_str(raw).map_err(|e| VaultError::Parse(e.to_string()))
    }

    /// Load + refresh if expired (or within `slack_secs` of expiry).
    /// Persists the rotated token back to `path` so the next process
    /// boot picks up the new access token. Returns the live token.
    pub fn load_oauth_with_refresh(
        &mut self,
        name: &str,
        slack_secs: i64,
        path: impl AsRef<Path>,
    ) -> Result<crate::oauth::OauthToken, VaultError> {
        let mut token = self.get_oauth(name)?;
        if token.needs_refresh(slack_secs) {
            token
                .refresh()
                .map_err(|e| VaultError::Encode(format!("oauth refresh: {e}")))?;
            self.set_oauth(name, &token)?;
            self.save(path)?;
        }
        Ok(token)
    }

    /// Write the vault to `path`. On Unix the file is created with `0600`;
    /// on other platforms we rely on default permissions and document the
    /// gap (Windows ACLs come with §42 sandbox).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), VaultError> {
        let path = path.as_ref();
        if self.version != VAULT_VERSION {
            return Err(VaultError::UnsupportedVersion {
                found: self.version,
                expected: VAULT_VERSION,
            });
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    VaultError::Io(format!("mkdir {}: {e}", parent.display()))
                })?;
            }
        }
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| VaultError::Encode(e.to_string()))?;
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        write_with_owner_only(&tmp, &bytes)?;
        std::fs::rename(&tmp, path).map_err(|e| {
            VaultError::Io(format!(
                "rename {} -> {}: {e}",
                tmp.display(),
                path.display()
            ))
        })?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, VaultError> {
        let path = path.as_ref();
        check_owner_only(path)?;
        let bytes = std::fs::read(path)
            .map_err(|e| VaultError::Io(format!("read {}: {e}", path.display())))?;
        let v: Self = serde_json::from_slice(&bytes)
            .map_err(|e| VaultError::Parse(format!("{}: {e}", path.display())))?;
        if v.version != VAULT_VERSION {
            return Err(VaultError::UnsupportedVersion {
                found: v.version,
                expected: VAULT_VERSION,
            });
        }
        Ok(v)
    }
}

#[cfg(unix)]
fn write_with_owner_only(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| VaultError::Io(format!("create {}: {e}", path.display())))?;
    f.write_all(bytes)
        .map_err(|e| VaultError::Io(format!("write {}: {e}", path.display())))?;
    f.sync_all().ok();
    Ok(())
}

#[cfg(not(unix))]
fn write_with_owner_only(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    std::fs::write(path, bytes)
        .map_err(|e| VaultError::Io(format!("write {}: {e}", path.display())))
}

#[cfg(unix)]
fn check_owner_only(path: &Path) -> Result<(), VaultError> {
    use std::os::unix::fs::PermissionsExt;
    let md = std::fs::metadata(path)
        .map_err(|e| VaultError::Io(format!("stat {}: {e}", path.display())))?;
    let mode = md.permissions().mode() & 0o777;
    // Reject if anyone other than the owner has any access.
    if mode & 0o077 != 0 {
        return Err(VaultError::InsecurePermissions {
            path: path.to_path_buf(),
            mode,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_owner_only(_path: &Path) -> Result<(), VaultError> {
    // Windows / WASI: rely on the host's ACLs. Document this in the README.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let pid = std::process::id();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!("axon-vault-test-{name}-{pid}-{ts}.json"));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn round_trip_through_disk() {
        let path = tmp_path("round_trip");
        let mut v = Vault::new();
        v.set("ANTHROPIC_KEY", "sk-ant-...");
        v.set("DB_PASSWORD", "hunter2");
        v.save(&path).unwrap();
        let back = Vault::load(&path).unwrap();
        assert_eq!(back.names().len(), 2);
        // Exposed value comes through Secret<String>.
        assert_eq!(back.get("DB_PASSWORD").unwrap().expose_for_use(), "hunter2");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_secret_errors() {
        let v = Vault::new();
        assert!(matches!(v.get("nope").unwrap_err(), VaultError::NotFound(_)));
    }

    #[test]
    fn debug_format_does_not_leak_inner_value() {
        let mut v = Vault::new();
        v.set("API_KEY", "super-secret-value");
        let s = v.get("API_KEY").unwrap();
        let dbg = format!("{s:?}");
        let dsp = format!("{s}");
        assert!(!dbg.contains("super-secret-value"));
        assert!(!dsp.contains("super-secret-value"));
    }

    #[test]
    #[cfg(unix)]
    fn insecure_permissions_rejected_on_load() {
        use std::os::unix::fs::PermissionsExt;
        let path = tmp_path("insecure");
        let mut v = Vault::new();
        v.set("k", "v");
        v.save(&path).unwrap();
        // Make it world-readable behind our back.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = Vault::load(&path).unwrap_err();
        assert!(matches!(err, VaultError::InsecurePermissions { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[cfg(unix)]
    fn save_sets_owner_only_mode() {
        use std::os::unix::fs::PermissionsExt;
        let path = tmp_path("perms");
        let mut v = Vault::new();
        v.set("k", "v");
        v.save(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "got mode {mode:o}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_format_version_rejected() {
        let path = tmp_path("badver");
        std::fs::write(&path, r#"{"version": 99, "secrets": {}}"#).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let err = Vault::load(&path).unwrap_err();
        assert!(matches!(err, VaultError::UnsupportedVersion { .. }));
        let _ = std::fs::remove_file(&path);
    }
}
