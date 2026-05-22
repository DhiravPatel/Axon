use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::entry::CostEntry;

pub const LEDGER_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default = "default_version")]
    pub version: u32,
    pub entries: Vec<CostEntry>,
}

fn default_version() -> u32 {
    LEDGER_VERSION
}

#[derive(Debug)]
pub enum LedgerError {
    Io(String),
    Parse(String),
    Encode(String),
    UnsupportedVersion { found: u32, expected: u32 },
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LedgerError::Io(m) => write!(f, "ledger I/O: {m}"),
            LedgerError::Parse(m) => write!(f, "ledger parse: {m}"),
            LedgerError::Encode(m) => write!(f, "ledger encode: {m}"),
            LedgerError::UnsupportedVersion { found, expected } => write!(
                f,
                "ledger: version mismatch (found={found}, expected={expected})"
            ),
        }
    }
}

impl std::error::Error for LedgerError {}

impl Ledger {
    pub fn new() -> Self {
        Self {
            version: LEDGER_VERSION,
            entries: Vec::new(),
        }
    }

    pub fn record(&mut self, entry: CostEntry) {
        self.entries.push(entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), LedgerError> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| LedgerError::Encode(e.to_string()))?;
        std::fs::write(path, bytes).map_err(|e| LedgerError::Io(e.to_string()))?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, LedgerError> {
        let bytes = std::fs::read(path.as_ref()).map_err(|e| LedgerError::Io(e.to_string()))?;
        let l: Self = serde_json::from_slice(&bytes).map_err(|e| LedgerError::Parse(e.to_string()))?;
        if l.version != LEDGER_VERSION {
            return Err(LedgerError::UnsupportedVersion {
                found: l.version,
                expected: LEDGER_VERSION,
            });
        }
        Ok(l)
    }
}
