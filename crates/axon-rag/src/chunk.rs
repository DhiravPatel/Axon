//! Recursive text chunker.
//!
//! Splits on the strongest separator first (`\n\n` paragraph), falls through
//! to weaker ones (`\n` line, ` ` word, `""` char) only when a chunk is still
//! too large. Overlap is the number of *characters* duplicated between
//! adjacent chunks so retrieval doesn't lose context at boundaries.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Chunk {
    /// Source identifier (file path, URL, doc id). Free-form.
    pub source: String,
    /// Zero-based ordinal within the source.
    pub ordinal: u32,
    /// Verbatim text.
    pub text: String,
}

/// Object-safe chunker trait. New backends (token-aware, layout-aware,
/// markdown-heading-aware) implement this without touching call sites.
pub trait Chunker: Send + Sync {
    fn chunks(&self, source: &str, text: &str) -> Vec<Chunk>;
}

#[derive(Clone, Debug)]
pub struct RecursiveChunker {
    pub max_chars: usize,
    pub overlap: usize,
}

impl RecursiveChunker {
    pub fn new(max_chars: usize, overlap: usize) -> Self {
        assert!(max_chars > 0, "max_chars must be positive");
        assert!(overlap < max_chars, "overlap must be smaller than max_chars");
        Self { max_chars, overlap }
    }
}

impl Default for RecursiveChunker {
    fn default() -> Self {
        Self::new(800, 120)
    }
}

impl Chunker for RecursiveChunker {
    fn chunks(&self, source: &str, text: &str) -> Vec<Chunk> {
        let mut pieces: Vec<String> = Vec::new();
        recursive_split(text, self.max_chars, &mut pieces);

        // Apply overlap: keep the last `overlap` chars of the previous chunk
        // at the start of the next, so context bridges the seam.
        let mut out: Vec<Chunk> = Vec::with_capacity(pieces.len());
        let mut ordinal: u32 = 0;
        let mut tail: String = String::new();
        for p in pieces {
            let combined = if tail.is_empty() {
                p.clone()
            } else {
                format!("{tail}{p}")
            };
            out.push(Chunk {
                source: source.to_string(),
                ordinal,
                text: combined.clone(),
            });
            ordinal += 1;
            tail = if self.overlap == 0 {
                String::new()
            } else {
                tail_chars(&combined, self.overlap)
            };
        }
        out
    }
}

fn recursive_split(text: &str, limit: usize, out: &mut Vec<String>) {
    if char_count(text) <= limit {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            out.push(trimmed.to_string());
        }
        return;
    }
    // Order matters: stronger separators first.
    for sep in &["\n\n", "\n", ". ", " "] {
        if let Some(pieces) = split_if_present(text, sep) {
            for p in pieces {
                if char_count(&p) <= limit {
                    let trimmed = p.trim();
                    if !trimmed.is_empty() {
                        out.push(trimmed.to_string());
                    }
                } else {
                    recursive_split(&p, limit, out);
                }
            }
            return;
        }
    }
    // No separator worked → hard-cut by characters.
    let mut buf = String::new();
    for c in text.chars() {
        buf.push(c);
        if char_count(&buf) >= limit {
            out.push(buf.clone());
            buf.clear();
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
}

fn split_if_present(text: &str, sep: &str) -> Option<Vec<String>> {
    if !text.contains(sep) {
        return None;
    }
    Some(text.split(sep).map(|s| s.to_string()).collect())
}

fn char_count(s: &str) -> usize {
    s.chars().count()
}

fn tail_chars(s: &str, n: usize) -> String {
    let total = char_count(s);
    if n >= total {
        return s.to_string();
    }
    s.chars().skip(total - n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_paragraphs_first() {
        let c = RecursiveChunker::new(40, 0);
        let xs = c.chunks(
            "doc",
            "alpha beta\n\ngamma delta\n\nepsilon zeta eta theta",
        );
        assert_eq!(xs.len(), 3);
        assert_eq!(xs[0].text, "alpha beta");
        assert_eq!(xs[1].text, "gamma delta");
    }

    #[test]
    fn falls_through_to_sentences_when_paragraph_too_big() {
        let c = RecursiveChunker::new(20, 0);
        let xs = c.chunks("doc", "Hello world. This is fine. Final part.");
        assert!(xs.len() >= 2);
        for ch in &xs {
            assert!(ch.text.chars().count() <= 20);
        }
    }

    #[test]
    fn overlap_bridges_chunks() {
        let c = RecursiveChunker::new(20, 5);
        let xs = c.chunks("d", "alpha beta\n\ngamma delta\n\nepsilon");
        // Second chunk should begin with the tail of the first.
        let tail = &xs[0].text[xs[0].text.len() - 5..];
        assert!(
            xs[1].text.starts_with(tail),
            "expected `{}` to start with `{}`",
            xs[1].text,
            tail
        );
    }

    #[test]
    fn hard_cut_on_unsplittable_input() {
        let c = RecursiveChunker::new(5, 0);
        let xs = c.chunks("d", "abcdefghij");
        assert!(xs.iter().all(|x| x.text.chars().count() <= 5));
        assert!(xs.len() >= 2);
    }
}
