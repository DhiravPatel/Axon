//! `axon-skill` — `.axskill` package format.
//!
//! v0 ships a **pure-Rust JSON archive**: a single `.axskill` file contains
//! one top-level JSON object with two keys:
//!
//! ```text
//! {
//!   "manifest": { name, version, entrypoint, capabilities, ... },
//!   "files":    { "src/lib.ax": "fn ...", "src/main.ax": "..." }
//! }
//! ```
//!
//! Pros over tar.gz for v0:
//!   * No native deps (no flate2/miniz_oxide).
//!   * Deterministic — sorted keys, no archive timestamps.
//!   * Human-inspectable with `cat | jq`.
//!   * Trivially round-trippable through `axon-memory`.
//!
//! Every archive carries a **content hash** (FNV-1a over a canonical
//! ordering of the file map) so tampering between pack and install is
//! detectable. Real Ed25519 signatures land in Stage 15 (secrets).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod errors;
pub mod use_skill;

pub use errors::SkillError;
pub use use_skill::{bind_skill, explain_missing, narrow_effects, SkillBinding};

pub const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub entrypoint: String,
    /// Capability names the skill needs to run (matches the runtime's
    /// `uses { ... }` row).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Optional list of declared dependencies (other skills, by name).
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Free-form authorship metadata.
    #[serde(default)]
    pub authors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub format_version: u32,
    pub manifest: Manifest,
    /// Path-relative-to-skill-root → UTF-8 file contents.
    pub files: BTreeMap<String, String>,
    /// Content hash of `files`, computed in [`Skill::compute_hash`].
    pub content_hash: String,
}

impl Skill {
    pub fn new(manifest: Manifest, files: BTreeMap<String, String>) -> Self {
        let content_hash = Self::compute_hash(&files);
        Self {
            format_version: FORMAT_VERSION,
            manifest,
            files,
            content_hash,
        }
    }

    /// Hash the file map using FNV-1a over a canonical "path\0body\0..."
    /// concatenation. Deterministic; no native deps.
    pub fn compute_hash(files: &BTreeMap<String, String>) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        let prime: u64 = 0x100000001b3;
        for (k, v) in files.iter() {
            for b in k.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(prime);
            }
            h ^= 0;
            h = h.wrapping_mul(prime);
            for b in v.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(prime);
            }
            h ^= 0;
            h = h.wrapping_mul(prime);
        }
        format!("h_{h:016x}")
    }

    /// Recompute the hash and confirm it matches `content_hash`.
    pub fn verify(&self) -> Result<(), SkillError> {
        let expected = Self::compute_hash(&self.files);
        if expected != self.content_hash {
            return Err(SkillError::HashMismatch {
                stored: self.content_hash.clone(),
                computed: expected,
            });
        }
        if self.format_version != FORMAT_VERSION {
            return Err(SkillError::UnsupportedVersion {
                found: self.format_version,
                expected: FORMAT_VERSION,
            });
        }
        if self.manifest.name.is_empty() {
            return Err(SkillError::Invalid("manifest.name is empty".into()));
        }
        if self.manifest.version.is_empty() {
            return Err(SkillError::Invalid("manifest.version is empty".into()));
        }
        if !self.files.contains_key(&self.manifest.entrypoint) {
            return Err(SkillError::Invalid(format!(
                "entrypoint `{}` is not in the file map",
                self.manifest.entrypoint
            )));
        }
        Ok(())
    }

    /// Serialize to JSON bytes (pretty-printed, sorted keys).
    pub fn to_json(&self) -> Result<Vec<u8>, SkillError> {
        serde_json::to_vec_pretty(self).map_err(|e| SkillError::Encode(e.to_string()))
    }

    /// Parse from JSON bytes and verify integrity.
    pub fn from_json(bytes: &[u8]) -> Result<Self, SkillError> {
        let s: Self = serde_json::from_slice(bytes)
            .map_err(|e| SkillError::Parse(e.to_string()))?;
        s.verify()?;
        Ok(s)
    }

    /// Pack a directory tree into a `Skill`. The directory must contain
    /// `manifest.json` at its root; all other files are slurped (UTF-8
    /// only) under their path relative to the root.
    pub fn pack(dir: impl AsRef<Path>) -> Result<Self, SkillError> {
        let dir = dir.as_ref();
        let manifest_path = dir.join("manifest.json");
        let manifest_bytes = std::fs::read(&manifest_path).map_err(|e| {
            SkillError::Io(format!("read {}: {e}", manifest_path.display()))
        })?;
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| SkillError::Parse(format!("manifest.json: {e}")))?;

        let mut files: BTreeMap<String, String> = BTreeMap::new();
        walk_collect(dir, dir, &mut files)?;
        // `manifest.json` itself is the *spec*, not a runtime file — exclude.
        files.remove("manifest.json");
        Ok(Self::new(manifest, files))
    }

    /// Write each file in this skill into `dest` (creating directories as
    /// needed) so callers can `axon run dest/<entrypoint>`. Verifies first.
    pub fn unpack_to(&self, dest: impl AsRef<Path>) -> Result<(), SkillError> {
        self.verify()?;
        let dest = dest.as_ref();
        std::fs::create_dir_all(dest)
            .map_err(|e| SkillError::Io(format!("mkdir {}: {e}", dest.display())))?;
        for (rel, body) in self.files.iter() {
            let p = dest.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    SkillError::Io(format!("mkdir {}: {e}", parent.display()))
                })?;
            }
            std::fs::write(&p, body)
                .map_err(|e| SkillError::Io(format!("write {}: {e}", p.display())))?;
        }
        Ok(())
    }
}

fn walk_collect(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, String>,
) -> Result<(), SkillError> {
    for entry in std::fs::read_dir(dir)
        .map_err(|e| SkillError::Io(format!("read_dir {}: {e}", dir.display())))?
    {
        let entry = entry
            .map_err(|e| SkillError::Io(format!("dirent {}: {e}", dir.display())))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .map_err(|e| SkillError::Io(format!("file_type {}: {e}", path.display())))?;
        if ft.is_dir() {
            walk_collect(root, &path, out)?;
        } else if ft.is_file() {
            let rel: PathBuf = path
                .strip_prefix(root)
                .map_err(|e| SkillError::Io(format!("strip_prefix: {e}")))?
                .to_path_buf();
            let rel_s = rel
                .to_str()
                .ok_or_else(|| {
                    SkillError::Invalid(format!("non-utf8 path: {}", rel.display()))
                })?
                .replace('\\', "/");
            let body = std::fs::read_to_string(&path).map_err(|e| {
                SkillError::Io(format!("read {}: {e}", path.display()))
            })?;
            out.insert(rel_s, body);
        }
    }
    Ok(())
}
