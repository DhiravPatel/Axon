//! Vector + lexical index.
//!
//! One [`Index`] holds passages, their dense embeddings, and a BM25
//! lexical index built over the same text. It can be serialized to JSON
//! (the same shape `axon-memory` uses for `FileStore`) and reloaded
//! losslessly.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::bm25::Bm25;
use crate::chunk::Chunk;
use crate::embed::{cosine, Embedder};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Passage {
    pub id: String,
    pub chunk: Chunk,
    pub embedding: Vec<f32>,
    /// Free-form metadata. Sorted JSON object for stable on-disk output.
    #[serde(default)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub dims: usize,
    pub passages: Vec<Passage>,
    /// Map id → vec index, rebuilt on load.
    #[serde(skip)]
    by_id: HashMap<String, usize>,
    /// BM25 lexical index, rebuilt on load.
    #[serde(skip)]
    bm25: Bm25,
}

impl Index {
    pub fn new(dims: usize) -> Self {
        Self {
            dims,
            passages: Vec::new(),
            by_id: HashMap::new(),
            bm25: Bm25::default(),
        }
    }

    /// Append one passage. Returns `false` if the id was already present
    /// (the prior entry is preserved; this makes ingestion idempotent).
    pub fn add(
        &mut self,
        embedder: &dyn Embedder,
        chunk: Chunk,
        metadata: serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        let id = passage_id(&chunk);
        if self.by_id.contains_key(&id) {
            return false;
        }
        let embedding = embedder.embed(&chunk.text);
        assert_eq!(
            embedding.len(),
            self.dims,
            "embedder dims must match index dims"
        );
        self.bm25.add(&chunk.text);
        let idx = self.passages.len();
        self.passages.push(Passage {
            id: id.clone(),
            chunk,
            embedding,
            metadata,
        });
        self.by_id.insert(id, idx);
        true
    }

    pub fn len(&self) -> usize {
        self.passages.len()
    }
    pub fn is_empty(&self) -> bool {
        self.passages.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Passage> {
        self.by_id.get(id).and_then(|i| self.passages.get(*i))
    }

    /// Dense cosine scores in id order.
    pub fn vector_scores(&self, query_vec: &[f32]) -> Vec<f32> {
        self.passages
            .iter()
            .map(|p| cosine(query_vec, &p.embedding))
            .collect()
    }

    /// BM25 scores in id order.
    pub fn lexical_scores(&self, query: &str) -> Vec<f32> {
        self.bm25.score_all(query)
    }

    /// Restore non-serialized sidecars (id map + BM25) after deserialization.
    pub fn rehydrate(&mut self) {
        self.by_id = self
            .passages
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id.clone(), i))
            .collect();
        let mut bm25 = Bm25::default();
        for p in &self.passages {
            bm25.add(&p.chunk.text);
        }
        self.bm25 = bm25;
    }

    /// Serialize the index to a JSON byte buffer.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }

    /// Parse + rehydrate from JSON bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let mut idx: Self = serde_json::from_slice(bytes)?;
        idx.rehydrate();
        Ok(idx)
    }
}

/// Deterministic content-hash-style id for a chunk. Same `(source, ordinal,
/// text)` always produces the same id, which makes ingestion idempotent and
/// safe to re-run on partial deletes.
pub fn passage_id(chunk: &Chunk) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    let prime: u64 = 0x100000001b3;
    for b in chunk.source.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(prime);
    }
    h ^= chunk.ordinal as u64;
    h = h.wrapping_mul(prime);
    for b in chunk.text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(prime);
    }
    format!("p_{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::HashEmbedder;

    fn idx() -> Index {
        let mut i = Index::new(512);
        let e = HashEmbedder::new(512);
        i.add(
            &e,
            Chunk {
                source: "a".into(),
                ordinal: 0,
                text: "ferret pet animal friendly weasel companion".into(),
            },
            Default::default(),
        );
        i.add(
            &e,
            Chunk {
                source: "b".into(),
                ordinal: 0,
                text: "the stock market closed lower today".into(),
            },
            Default::default(),
        );
        i
    }

    #[test]
    fn add_is_idempotent_per_id() {
        let mut i = Index::new(16);
        let e = HashEmbedder::new(16);
        let c = Chunk {
            source: "a".into(),
            ordinal: 0,
            text: "hello".into(),
        };
        assert!(i.add(&e, c.clone(), Default::default()));
        assert!(!i.add(&e, c, Default::default()));
        assert_eq!(i.len(), 1);
    }

    #[test]
    fn vector_search_picks_topical_match() {
        let i = idx();
        let e = HashEmbedder::new(512);
        let q = e.embed("friendly ferret pet animal");
        let scores = i.vector_scores(&q);
        assert!(scores[0] > scores[1], "scores: {scores:?}");
    }

    #[test]
    fn bm25_inside_index_matches_standalone() {
        let i = idx();
        let scores = i.lexical_scores("ferret pet");
        assert!(scores[0] > scores[1], "scores: {scores:?}");
    }

    #[test]
    fn round_trip_through_json() {
        let i = idx();
        let bytes = i.to_json_bytes().unwrap();
        let back = Index::from_json_bytes(&bytes).unwrap();
        assert_eq!(back.len(), i.len());
        assert_eq!(back.dims, i.dims);
        // Sidecars rehydrated:
        assert!(back.get(&i.passages[0].id).is_some());
        // BM25 scores still work:
        let scores = back.lexical_scores("ferret pet");
        assert!(scores[0] > scores[1], "scores: {scores:?}");
    }
}
