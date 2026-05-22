//! `axon-memory` — pluggable persistent stores for `Memory` values.
//!
//! Two v0 backends:
//!
//! * [`EphemeralStore`] — pure in-memory key/value, lost on process exit.
//! * [`FileStore`] — JSON-backed key/value with **atomic writes** (write to a
//!   temp file, fsync, then `rename`). A restarted agent reads the same
//!   bytes it wrote on its last call.
//!
//! Both implement the [`Store`] trait; downstream code (the runtime, tests,
//! the CLI) holds a `Box<dyn Store>` or `Arc<dyn Store>` and never cares
//! which backend is behind it.
//!
//! Values are stored as JSON. This is deliberate: it survives schema
//! changes ergonomically, plays nicely with `serde_json::Value` from the
//! tooling layer, and makes it possible to inspect a `.json` memory file
//! with `cat`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

mod errors;

pub use errors::MemoryError;

/// One stored entry: a `serde_json::Value` plus optional metadata.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub value: serde_json::Value,
    /// Optional retention tag (`"important"`, `"redacted"`, ...). The
    /// runtime uses this for `forget` filters and consolidation passes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Wall-clock timestamp of the last write — `serde_json::Number` so the
    /// JSON file survives 32-bit / 64-bit boundaries. v0 records nanoseconds
    /// since Unix epoch (i64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written_at_ns: Option<i64>,
}

impl Entry {
    pub fn new(value: serde_json::Value) -> Self {
        Self {
            value,
            tag: None,
            written_at_ns: None,
        }
    }

    pub fn tagged(value: serde_json::Value, tag: impl Into<String>) -> Self {
        Self {
            value,
            tag: Some(tag.into()),
            written_at_ns: None,
        }
    }
}

/// Snapshot of the underlying store. The file backend persists this; the
/// ephemeral backend just keeps it in RAM.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    /// Schema version. Bumped only when the on-disk layout changes — older
    /// files with an unknown version are refused with [`MemoryError::SchemaMismatch`].
    #[serde(default = "default_version")]
    pub version: u32,
    /// Sorted to keep the JSON output deterministic.
    #[serde(default)]
    pub entries: BTreeMap<String, Entry>,
}

fn default_version() -> u32 {
    SCHEMA_VERSION
}

/// On-disk schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Object-safe trait every store implements.
pub trait Store: Send + Sync + std::fmt::Debug {
    fn get(&self, key: &str) -> Result<Option<Entry>, MemoryError>;
    fn set(&self, key: &str, entry: Entry) -> Result<(), MemoryError>;
    fn remove(&self, key: &str) -> Result<bool, MemoryError>;
    fn keys(&self) -> Result<Vec<String>, MemoryError>;
    fn len(&self) -> Result<usize, MemoryError> {
        self.keys().map(|k| k.len())
    }
    fn is_empty(&self) -> Result<bool, MemoryError> {
        self.len().map(|n| n == 0)
    }
    fn snapshot(&self) -> Result<Snapshot, MemoryError>;
    /// Drop every entry whose tag matches `tag`. Used by retention passes.
    fn forget_tagged(&self, tag: &str) -> Result<usize, MemoryError>;
    /// Drop every entry that has a `written_at_ns` older than the threshold.
    /// Entries with no timestamp are preserved.
    fn forget_older_than(&self, cutoff_ns: i64) -> Result<usize, MemoryError>;
}

// ---------------------------------------------------------------------------
// Ephemeral backend
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct EphemeralStore {
    inner: Mutex<BTreeMap<String, Entry>>,
}

impl EphemeralStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_arc(self) -> Arc<dyn Store> {
        Arc::new(self) as Arc<dyn Store>
    }
}

impl Store for EphemeralStore {
    fn get(&self, key: &str) -> Result<Option<Entry>, MemoryError> {
        Ok(self.inner.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, entry: Entry) -> Result<(), MemoryError> {
        self.inner.lock().unwrap().insert(key.to_string(), entry);
        Ok(())
    }
    fn remove(&self, key: &str) -> Result<bool, MemoryError> {
        Ok(self.inner.lock().unwrap().remove(key).is_some())
    }
    fn keys(&self) -> Result<Vec<String>, MemoryError> {
        Ok(self.inner.lock().unwrap().keys().cloned().collect())
    }
    fn len(&self) -> Result<usize, MemoryError> {
        Ok(self.inner.lock().unwrap().len())
    }
    fn snapshot(&self) -> Result<Snapshot, MemoryError> {
        Ok(Snapshot {
            version: SCHEMA_VERSION,
            entries: self.inner.lock().unwrap().clone(),
        })
    }
    fn forget_tagged(&self, tag: &str) -> Result<usize, MemoryError> {
        let mut g = self.inner.lock().unwrap();
        let before = g.len();
        g.retain(|_, v| v.tag.as_deref() != Some(tag));
        Ok(before - g.len())
    }
    fn forget_older_than(&self, cutoff_ns: i64) -> Result<usize, MemoryError> {
        let mut g = self.inner.lock().unwrap();
        let before = g.len();
        g.retain(|_, v| v.written_at_ns.map(|t| t >= cutoff_ns).unwrap_or(true));
        Ok(before - g.len())
    }
}

// ---------------------------------------------------------------------------
// File-backed backend (atomic writes)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct FileStore {
    path: PathBuf,
    inner: Mutex<BTreeMap<String, Entry>>,
}

impl FileStore {
    /// Open a file-backed store. If `path` exists, its contents are loaded.
    /// If it doesn't, the file is *not* created until the first write.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MemoryError> {
        let path = path.as_ref().to_path_buf();
        let entries = if path.exists() {
            let bytes = std::fs::read(&path)
                .map_err(|e| MemoryError::Io(format!("read {}: {e}", path.display())))?;
            let snap: Snapshot = serde_json::from_slice(&bytes)
                .map_err(|e| MemoryError::Parse(format!("{}: {e}", path.display())))?;
            if snap.version != SCHEMA_VERSION {
                return Err(MemoryError::SchemaMismatch {
                    found: snap.version,
                    expected: SCHEMA_VERSION,
                });
            }
            snap.entries
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            path,
            inner: Mutex::new(entries),
        })
    }

    /// Path the store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn into_arc(self) -> Arc<dyn Store> {
        Arc::new(self) as Arc<dyn Store>
    }

    /// Persist current contents to disk atomically:
    ///   1. write to `<path>.tmp.<pid>`
    ///   2. fsync the temp file
    ///   3. rename onto the final path (atomic on POSIX, best-effort on Windows)
    fn flush(&self, entries: &BTreeMap<String, Entry>) -> Result<(), MemoryError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| MemoryError::Io(format!("mkdir {}: {e}", parent.display())))?;
            }
        }
        let snap = Snapshot {
            version: SCHEMA_VERSION,
            entries: entries.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&snap)
            .map_err(|e| MemoryError::Encode(e.to_string()))?;
        let tmp = self
            .path
            .with_extension(format!("tmp.{}", std::process::id()));
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)
                .map_err(|e| MemoryError::Io(format!("create {}: {e}", tmp.display())))?;
            f.write_all(&bytes)
                .map_err(|e| MemoryError::Io(format!("write {}: {e}", tmp.display())))?;
            f.sync_all().ok();
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| {
            MemoryError::Io(format!(
                "rename {} -> {}: {e}",
                tmp.display(),
                self.path.display()
            ))
        })?;
        Ok(())
    }
}

impl Store for FileStore {
    fn get(&self, key: &str) -> Result<Option<Entry>, MemoryError> {
        Ok(self.inner.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, entry: Entry) -> Result<(), MemoryError> {
        let snapshot = {
            let mut g = self.inner.lock().unwrap();
            g.insert(key.to_string(), entry);
            g.clone()
        };
        self.flush(&snapshot)
    }
    fn remove(&self, key: &str) -> Result<bool, MemoryError> {
        let (existed, snapshot) = {
            let mut g = self.inner.lock().unwrap();
            let existed = g.remove(key).is_some();
            (existed, g.clone())
        };
        if existed {
            self.flush(&snapshot)?;
        }
        Ok(existed)
    }
    fn keys(&self) -> Result<Vec<String>, MemoryError> {
        Ok(self.inner.lock().unwrap().keys().cloned().collect())
    }
    fn len(&self) -> Result<usize, MemoryError> {
        Ok(self.inner.lock().unwrap().len())
    }
    fn snapshot(&self) -> Result<Snapshot, MemoryError> {
        Ok(Snapshot {
            version: SCHEMA_VERSION,
            entries: self.inner.lock().unwrap().clone(),
        })
    }
    fn forget_tagged(&self, tag: &str) -> Result<usize, MemoryError> {
        let (dropped, snapshot) = {
            let mut g = self.inner.lock().unwrap();
            let before = g.len();
            g.retain(|_, v| v.tag.as_deref() != Some(tag));
            (before - g.len(), g.clone())
        };
        if dropped > 0 {
            self.flush(&snapshot)?;
        }
        Ok(dropped)
    }
    fn forget_older_than(&self, cutoff_ns: i64) -> Result<usize, MemoryError> {
        let (dropped, snapshot) = {
            let mut g = self.inner.lock().unwrap();
            let before = g.len();
            g.retain(|_, v| v.written_at_ns.map(|t| t >= cutoff_ns).unwrap_or(true));
            (before - g.len(), g.clone())
        };
        if dropped > 0 {
            self.flush(&snapshot)?;
        }
        Ok(dropped)
    }
}
