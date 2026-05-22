use std::fmt;

#[derive(Debug)]
pub enum SkillError {
    Io(String),
    Parse(String),
    Encode(String),
    Invalid(String),
    HashMismatch { stored: String, computed: String },
    UnsupportedVersion { found: u32, expected: u32 },
}

impl fmt::Display for SkillError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkillError::Io(m) => write!(f, "skill I/O: {m}"),
            SkillError::Parse(m) => write!(f, "skill parse: {m}"),
            SkillError::Encode(m) => write!(f, "skill encode: {m}"),
            SkillError::Invalid(m) => write!(f, "skill: invalid: {m}"),
            SkillError::HashMismatch { stored, computed } => write!(
                f,
                "skill: content hash mismatch (stored={stored}, computed={computed})"
            ),
            SkillError::UnsupportedVersion { found, expected } => write!(
                f,
                "skill: format version mismatch (found={found}, expected={expected})"
            ),
        }
    }
}

impl std::error::Error for SkillError {}
