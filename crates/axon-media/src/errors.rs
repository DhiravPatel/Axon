use std::fmt;

#[derive(Debug)]
pub enum MediaError {
    Io(String),
    NotEnoughBytes { need: usize, got: usize },
    BadMagic { kind: &'static str },
    Truncated { context: &'static str },
    Unsupported { detail: String },
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::Io(m) => write!(f, "media I/O: {m}"),
            MediaError::NotEnoughBytes { need, got } => {
                write!(f, "media: need {need} bytes, only {got} available")
            }
            MediaError::BadMagic { kind } => {
                write!(f, "media: not a valid {kind} file (signature mismatch)")
            }
            MediaError::Truncated { context } => {
                write!(f, "media: truncated input while parsing {context}")
            }
            MediaError::Unsupported { detail } => write!(f, "media: unsupported: {detail}"),
        }
    }
}

impl std::error::Error for MediaError {}
