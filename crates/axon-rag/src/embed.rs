//! Embedding backends.
//!
//! `HashEmbedder` is a feature-hashing embedder: each unique whitespace
//! token is hashed (FNV-1a) into `dims` buckets, signed by hash bit 0, then
//! L2-normalized. It captures lexical overlap well, runs without a network,
//! and produces byte-identical embeddings on every call — perfect for
//! deterministic tests and replay-mode runs.
//!
//! When network-bound embedders (Anthropic, OpenAI, local ONNX) ship, they
//! implement the same `Embedder` trait without affecting the rest of the
//! pipeline.

use serde::{Deserialize, Serialize};

/// Object-safe embedding interface. Implementors choose dims & semantics.
pub trait Embedder: Send + Sync {
    fn dims(&self) -> usize;
    fn embed(&self, text: &str) -> Vec<f32>;
    /// Default batch impl falls back to per-call; real backends override
    /// to coalesce HTTP requests.
    fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }
}

/// Feature-hashing embedder. `dims` is the embedding dimensionality (the
/// hash output is `id % dims`). 256 is a reasonable balance for tests; 1536
/// matches OpenAI's `text-embedding-3-small` for shape compatibility.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HashEmbedder {
    pub dims: usize,
    /// Whether to lowercase tokens before hashing.
    #[serde(default = "default_lower")]
    pub lowercase: bool,
}

fn default_lower() -> bool {
    true
}

impl HashEmbedder {
    pub fn new(dims: usize) -> Self {
        assert!(dims > 0, "dims must be positive");
        Self {
            dims,
            lowercase: true,
        }
    }

    fn tokenize<'a>(&self, text: &'a str) -> impl Iterator<Item = std::borrow::Cow<'a, str>> + 'a {
        let lowercase = self.lowercase;
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(move |t| {
                if lowercase {
                    std::borrow::Cow::Owned(t.to_lowercase())
                } else {
                    std::borrow::Cow::Borrowed(t)
                }
            })
    }
}

impl Embedder for HashEmbedder {
    fn dims(&self) -> usize {
        self.dims
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dims];
        for tok in self.tokenize(text) {
            let h = fnv1a(tok.as_bytes());
            let bucket = (h as usize) % self.dims;
            let sign: f32 = if (h & 1) == 0 { 1.0 } else { -1.0 };
            v[bucket] += sign;
        }
        l2_normalize(&mut v);
        v
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum::<f32>()
    // Both sides are L2-normalized, so the inner product *is* cosine.
}

fn l2_normalize(v: &mut [f32]) {
    let mag = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag > 0.0 {
        for x in v.iter_mut() {
            *x /= mag;
        }
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut h = FNV_OFFSET;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_are_deterministic() {
        let e = HashEmbedder::new(64);
        let a = e.embed("hello world");
        let b = e.embed("hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn embeds_are_l2_unit() {
        let e = HashEmbedder::new(128);
        let v = e.embed("the quick brown fox");
        let mag: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((mag - 1.0).abs() < 1e-6, "magnitude {mag}");
    }

    #[test]
    fn semantic_overlap_beats_unrelated() {
        let e = HashEmbedder::new(256);
        let a = e.embed("ferrets are friendly weasel-like pets");
        let b = e.embed("ferrets make great companion animals");
        let c = e.embed("the stock market closed lower today");
        // Cosine works for our feature-hash space too — overlap on common
        // tokens dominates the score even with sign-flipping.
        let ab = cosine(&a, &b);
        let ac = cosine(&a, &c);
        assert!(
            ab > ac,
            "overlapping texts should score higher: ab={ab} ac={ac}"
        );
    }

    #[test]
    fn empty_text_produces_zero_vector_without_panic() {
        let e = HashEmbedder::new(8);
        let v = e.embed("");
        assert_eq!(v.len(), 8);
        assert!(v.iter().all(|x| *x == 0.0));
    }
}
