use std::fmt;

#[derive(Debug)]
pub enum FfiError {
    Spawn(String),
    Io(String),
    Wait(String),
    Encode(String),
    Parse(String),
    Closed,
    Timeout,
}

impl fmt::Display for FfiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FfiError::Spawn(m) => write!(f, "ffi spawn: {m}"),
            FfiError::Io(m) => write!(f, "ffi I/O: {m}"),
            FfiError::Wait(m) => write!(f, "ffi wait: {m}"),
            FfiError::Encode(m) => write!(f, "ffi encode: {m}"),
            FfiError::Parse(m) => write!(f, "ffi parse: {m}"),
            FfiError::Closed => write!(f, "ffi: subprocess closed stdout before responding"),
            FfiError::Timeout => write!(f, "ffi: timeout waiting for response"),
        }
    }
}

impl std::error::Error for FfiError {}
