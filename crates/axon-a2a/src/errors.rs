use std::fmt;

#[derive(Debug)]
pub enum A2aError {
    Io(String),
    Parse(String),
    Encode(String),
    Invalid(String),
    UnsupportedVersion { found: u32, expected: u32 },
}

impl fmt::Display for A2aError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            A2aError::Io(m) => write!(f, "a2a I/O: {m}"),
            A2aError::Parse(m) => write!(f, "a2a parse: {m}"),
            A2aError::Encode(m) => write!(f, "a2a encode: {m}"),
            A2aError::Invalid(m) => write!(f, "a2a: invalid: {m}"),
            A2aError::UnsupportedVersion { found, expected } => write!(
                f,
                "a2a: card format version mismatch (found={found}, expected={expected})"
            ),
        }
    }
}

impl std::error::Error for A2aError {}
