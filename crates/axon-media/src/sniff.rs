//! Content-sniffing — return a coarse media kind and MIME from bytes.
//!
//! Matches the bytes-first WHATWG-style sniff: trust the file content, not
//! the file extension. Useful before deciding which parser to call.

use crate::MediaKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SniffResult {
    pub kind: MediaKind,
    pub mime: &'static str,
}

pub fn sniff(bytes: &[u8]) -> SniffResult {
    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return SniffResult {
            kind: MediaKind::Image,
            mime: "image/png",
        };
    }
    // JPEG: FF D8 FF
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return SniffResult {
            kind: MediaKind::Image,
            mime: "image/jpeg",
        };
    }
    // GIF89a / GIF87a
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return SniffResult {
            kind: MediaKind::Image,
            mime: "image/gif",
        };
    }
    // RIFF....WAVE  (12 bytes minimum)
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return SniffResult {
            kind: MediaKind::Audio,
            mime: "audio/wav",
        };
    }
    // PDF: %PDF-
    if bytes.starts_with(b"%PDF-") {
        return SniffResult {
            kind: MediaKind::Document,
            mime: "application/pdf",
        };
    }
    // BOM / printable → assume text
    if looks_like_text(bytes) {
        return SniffResult {
            kind: MediaKind::Document,
            mime: "text/plain",
        };
    }
    SniffResult {
        kind: MediaKind::Unknown,
        mime: "application/octet-stream",
    }
}

fn looks_like_text(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let probe = &bytes[..bytes.len().min(512)];
    let total = probe.len();
    let printable = probe
        .iter()
        .filter(|b| (**b >= 0x20 && **b < 0x7F) || matches!(**b, b'\n' | b'\r' | b'\t' | 0x0C))
        .count();
    (printable as f32 / total as f32) > 0.85
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_signature_classifies_as_image_png() {
        let bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        assert_eq!(sniff(&bytes).mime, "image/png");
    }

    #[test]
    fn jpeg_signature_classifies_as_image_jpeg() {
        let bytes = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];
        assert_eq!(sniff(&bytes).mime, "image/jpeg");
    }

    #[test]
    fn wav_signature_classifies_as_audio_wav() {
        let mut b = b"RIFF\x00\x00\x00\x00WAVE".to_vec();
        b.extend(&[0u8; 32]);
        assert_eq!(sniff(&b).mime, "audio/wav");
    }

    #[test]
    fn pdf_signature_classifies_as_document_pdf() {
        let bytes = b"%PDF-1.4\nblah";
        assert_eq!(sniff(bytes).mime, "application/pdf");
    }

    #[test]
    fn ascii_text_classifies_as_text_plain() {
        assert_eq!(sniff(b"hello, world\n").mime, "text/plain");
    }

    #[test]
    fn random_bytes_classify_as_unknown() {
        let bytes = [0u8, 1, 2, 0xFE, 0xFD, 0xFC, 0x00, 0xFF];
        assert_eq!(sniff(&bytes).mime, "application/octet-stream");
    }
}
