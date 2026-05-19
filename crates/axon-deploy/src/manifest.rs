//! `deploy.json` — sibling of `.axskill` at deploy time.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployManifest {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Name of the skill this manifest deploys. Should match the
    /// `manifest.name` in the `.axskill` it ships with.
    pub name: String,
    /// Top-level handler the server invokes for `POST /invoke`.
    pub entrypoint_handler: String,
    /// TCP port the server binds to.
    pub port: u16,
    /// Environment variables to set before running.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Names of health checks the runtime should plug in.
    #[serde(default)]
    pub health_checks: Vec<String>,
    /// Optional reference to a `.env` file path (relative to manifest).
    #[serde(default)]
    pub dotenv: Option<String>,
    /// Optional reference to a secrets vault path (relative to manifest).
    #[serde(default)]
    pub vault: Option<String>,
}

fn default_version() -> u32 {
    MANIFEST_VERSION
}

#[derive(Debug)]
pub enum ManifestError {
    Io(String),
    Parse(String),
    Encode(String),
    UnsupportedVersion { found: u32, expected: u32 },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Io(m) => write!(f, "manifest I/O: {m}"),
            ManifestError::Parse(m) => write!(f, "manifest parse: {m}"),
            ManifestError::Encode(m) => write!(f, "manifest encode: {m}"),
            ManifestError::UnsupportedVersion { found, expected } => write!(
                f,
                "manifest: version mismatch (found={found}, expected={expected})"
            ),
        }
    }
}
impl std::error::Error for ManifestError {}

impl DeployManifest {
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ManifestError> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| ManifestError::Encode(e.to_string()))?;
        std::fs::write(path, bytes).map_err(|e| ManifestError::Io(e.to_string()))?;
        Ok(())
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let bytes = std::fs::read(path).map_err(|e| ManifestError::Io(e.to_string()))?;
        let m: Self = serde_json::from_slice(&bytes).map_err(|e| ManifestError::Parse(e.to_string()))?;
        if m.version != MANIFEST_VERSION {
            return Err(ManifestError::UnsupportedVersion {
                found: m.version,
                expected: MANIFEST_VERSION,
            });
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_disk() {
        let mut m = DeployManifest {
            version: MANIFEST_VERSION,
            name: "demo".into(),
            entrypoint_handler: "handle".into(),
            port: 8080,
            env: Default::default(),
            health_checks: vec!["liveness".into()],
            dotenv: Some(".env".into()),
            vault: Some("vault.json".into()),
        };
        m.env.insert("LOG_LEVEL".into(), "info".into());

        let mut path = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        path.push(format!("axon-manifest-{}-{ts}.json", std::process::id()));

        m.save(&path).unwrap();
        let back = DeployManifest::load(&path).unwrap();
        assert_eq!(back, m);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_version_rejected() {
        let mut path = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        path.push(format!("axon-manifest-bad-{}-{ts}.json", std::process::id()));
        std::fs::write(
            &path,
            r#"{"version": 999, "name": "x", "entrypoint_handler": "h", "port": 0}"#,
        )
        .unwrap();
        let err = DeployManifest::load(&path).unwrap_err();
        assert!(matches!(err, ManifestError::UnsupportedVersion { .. }));
        let _ = std::fs::remove_file(&path);
    }
}
