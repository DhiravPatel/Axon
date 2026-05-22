use std::fmt;

#[derive(Debug)]
pub enum SandboxError {
    Spawn(String),
    Wait(String),
    Io(String),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::Spawn(m) => write!(f, "sandbox spawn: {m}"),
            SandboxError::Wait(m) => write!(f, "sandbox wait: {m}"),
            SandboxError::Io(m) => write!(f, "sandbox I/O: {m}"),
        }
    }
}

impl std::error::Error for SandboxError {}
