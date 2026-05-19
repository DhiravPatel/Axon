//! Plain-text Document type.
//!
//! v0 supports text documents with optional pagination by form-feed
//! (`\x0C`) — the convention used by `pdftotext` and many OCR tools.
//! PDF binary parsing is out of scope; the runtime should pre-extract
//! text into a `.txt` file or stream and load it here.

use serde::{Deserialize, Serialize};

use crate::errors::MediaError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub mime: String,
    pub pages: Vec<String>,
    pub byte_len: u64,
}

impl Document {
    /// Load a UTF-8 text file (or PDF-extracted text with form-feed page
    /// separators). Non-text MIME types are refused — the caller is
    /// expected to pipe through `doc.parse` (Stage 12.1+) for those.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Self, MediaError> {
        let bytes = std::fs::read(&path)
            .map_err(|e| MediaError::Io(format!("read {}: {e}", path.as_ref().display())))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MediaError> {
        let text = std::str::from_utf8(bytes).map_err(|_| MediaError::Unsupported {
            detail: "Document::from_bytes expects UTF-8 text".to_string(),
        })?;
        let pages: Vec<String> = if text.contains('\x0C') {
            text.split('\x0C').map(|p| p.to_string()).collect()
        } else {
            vec![text.to_string()]
        };
        Ok(Document {
            mime: "text/plain".to_string(),
            pages,
            byte_len: bytes.len() as u64,
        })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn full_text(&self) -> String {
        self.pages.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_page_when_no_form_feed() {
        let d = Document::from_bytes(b"Hello\nWorld").unwrap();
        assert_eq!(d.page_count(), 1);
        assert_eq!(d.pages[0], "Hello\nWorld");
    }

    #[test]
    fn form_feed_splits_into_pages() {
        let d = Document::from_bytes(b"page one\x0Cpage two\x0Cpage three").unwrap();
        assert_eq!(d.page_count(), 3);
        assert_eq!(d.pages[1], "page two");
    }

    #[test]
    fn full_text_joins_pages_with_blank_line() {
        let d = Document::from_bytes(b"a\x0Cb").unwrap();
        assert_eq!(d.full_text(), "a\n\nb");
    }

    #[test]
    fn non_utf8_input_is_rejected() {
        let err = Document::from_bytes(&[0xFF, 0xFE, 0xFD]).unwrap_err();
        assert!(matches!(err, MediaError::Unsupported { .. }));
    }
}
