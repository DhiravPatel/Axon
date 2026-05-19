//! LSP positions ↔ Axon byte offsets.
//!
//! LSP 3.17 negotiates a `positionEncoding`; we advertise `utf-8` so
//! `Position.character` is a byte offset within the line. Editors that
//! still send UTF-16 positions will see slightly-off columns on
//! multi-byte characters until they upgrade — most modern clients
//! (VS Code, Helix, Zed, neovim) support UTF-8 since 2023.

use axon_diag::Span;
use lsp_types::{Position, Range};

/// Convert a byte offset to `(line, character)`. Both are 0-based for
/// LSP. Falls back to (0, 0) if the offset is past the end of the text.
pub fn offset_to_position(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (i, b) in text.as_bytes().iter().enumerate().take(offset) {
        if *b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    Position {
        line,
        character: (offset - line_start) as u32,
    }
}

/// Convert a `Position { line, character }` to a byte offset. Out-of-range
/// positions clamp to the end of the corresponding line.
pub fn position_to_offset(text: &str, pos: Position) -> usize {
    let mut line = 0u32;
    let mut offset = 0usize;
    for b in text.as_bytes() {
        if line == pos.line {
            break;
        }
        offset += 1;
        if *b == b'\n' {
            line += 1;
        }
    }
    if line != pos.line {
        return text.len();
    }
    let line_end = text.as_bytes()[offset..]
        .iter()
        .position(|b| *b == b'\n')
        .map(|p| offset + p)
        .unwrap_or(text.len());
    (offset + pos.character as usize).min(line_end)
}

/// Convert an Axon [`Span`] to an LSP [`Range`] within the same source.
pub fn span_to_range(text: &str, span: Span) -> Range {
    Range {
        start: offset_to_position(text, span.start as usize),
        end: offset_to_position(text, span.end as usize),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_single_line() {
        let text = "hello world";
        let p = offset_to_position(text, 6);
        assert_eq!(p, Position { line: 0, character: 6 });
        assert_eq!(position_to_offset(text, p), 6);
    }

    #[test]
    fn round_trip_multiline() {
        let text = "abc\ndefg\nhij";
        let off = 4 + 2; // 'f' on line 1
        let p = offset_to_position(text, off);
        assert_eq!(p, Position { line: 1, character: 2 });
        assert_eq!(position_to_offset(text, p), off);
    }

    #[test]
    fn out_of_range_clamps() {
        let text = "abc";
        let p = offset_to_position(text, 999);
        assert_eq!(p, Position { line: 0, character: 3 });
        let off = position_to_offset(text, Position { line: 99, character: 0 });
        assert_eq!(off, text.len());
    }

    #[test]
    fn position_past_eol_clamps_to_line_end() {
        let text = "abc\ndef";
        // Line 0 has 3 chars; asking for character 10 → end of line 0 = 3.
        let off = position_to_offset(text, Position { line: 0, character: 10 });
        assert_eq!(off, 3);
    }
}
