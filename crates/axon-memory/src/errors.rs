use std::fmt;

#[derive(Debug)]
pub enum MemoryError {
    Io(String),
    Parse(String),
    Encode(String),
    SchemaMismatch { found: u32, expected: u32 },
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryError::Io(m) => write!(f, "memory I/O: {m}"),
            MemoryError::Parse(m) => write!(f, "memory parse: {m}"),
            MemoryError::Encode(m) => write!(f, "memory encode: {m}"),
            MemoryError::SchemaMismatch { found, expected } => write!(
                f,
                "memory schema mismatch: file is v{found}, this binary expects v{expected}"
            ),
        }
    }
}

impl std::error::Error for MemoryError {}
