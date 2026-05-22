//! BM25 lexical scorer.
//!
//! Classical Okapi BM25 with `k1 = 1.5` and `b = 0.75` (the textbook
//! defaults). Tokenization mirrors [`HashEmbedder`] so the lexical and
//! vector pipelines see the same words.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bm25 {
    /// Document-frequency for each term.
    df: HashMap<String, u32>,
    /// Per-document term frequency tables, in `add()` order.
    docs: Vec<HashMap<String, u32>>,
    /// Per-document length (in tokens).
    doc_lens: Vec<u32>,
    /// Sum of `doc_lens` so we can compute the average.
    total_len: u64,
    pub k1: f32,
    pub b: f32,
}

impl Default for Bm25 {
    fn default() -> Self {
        Self::new(1.5, 0.75)
    }
}

impl Bm25 {
    pub fn new(k1: f32, b: f32) -> Self {
        Self {
            df: HashMap::new(),
            docs: Vec::new(),
            doc_lens: Vec::new(),
            total_len: 0,
            k1,
            b,
        }
    }

    pub fn doc_count(&self) -> usize {
        self.docs.len()
    }

    /// Index a document. Returns the doc's id (just its index).
    pub fn add(&mut self, text: &str) -> usize {
        let toks = tokenize(text);
        let mut tf: HashMap<String, u32> = HashMap::new();
        for t in &toks {
            *tf.entry(t.clone()).or_insert(0) += 1;
        }
        for term in tf.keys() {
            *self.df.entry(term.clone()).or_insert(0) += 1;
        }
        let len = toks.len() as u32;
        self.docs.push(tf);
        self.doc_lens.push(len);
        self.total_len += len as u64;
        self.docs.len() - 1
    }

    fn idf(&self, term: &str) -> f32 {
        let n = self.docs.len() as f32;
        let df = *self.df.get(term).unwrap_or(&0) as f32;
        // Add the +0.5 smoothing from the BM25+ variant so missing terms
        // don't return negative IDF.
        ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
    }

    fn avg_len(&self) -> f32 {
        if self.docs.is_empty() {
            return 0.0;
        }
        (self.total_len as f32) / (self.docs.len() as f32)
    }

    pub fn score(&self, query: &str, doc_id: usize) -> f32 {
        if doc_id >= self.docs.len() {
            return 0.0;
        }
        let tf = &self.docs[doc_id];
        let len = self.doc_lens[doc_id] as f32;
        let avg = self.avg_len().max(1.0);
        let mut total = 0.0;
        for term in tokenize(query) {
            let f = *tf.get(&term).unwrap_or(&0) as f32;
            if f == 0.0 {
                continue;
            }
            let idf = self.idf(&term);
            let num = f * (self.k1 + 1.0);
            let den = f + self.k1 * (1.0 - self.b + self.b * len / avg);
            total += idf * num / den;
        }
        total
    }

    /// Score every document and return their raw BM25 values in id order.
    pub fn score_all(&self, query: &str) -> Vec<f32> {
        (0..self.docs.len()).map(|i| self.score(query, i)).collect()
    }
}

pub(crate) fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Bm25 {
        let mut idx = Bm25::default();
        idx.add("the quick brown fox jumps over the lazy dog");
        idx.add("a fast brown fox leapt over a sleepy hound");
        idx.add("the stock market closed lower today");
        idx
    }

    #[test]
    fn matching_doc_outscores_unrelated() {
        let idx = fixture();
        let s_match = idx.score("brown fox", 0);
        let s_unrelated = idx.score("brown fox", 2);
        assert!(s_match > s_unrelated, "match {s_match}, unrelated {s_unrelated}");
        assert_eq!(s_unrelated, 0.0);
    }

    #[test]
    fn rare_term_outweighs_common_term() {
        let idx = fixture();
        // "fox" appears in 2/3 docs; "stock" in 1/3 — the latter has higher IDF.
        let s_fox = idx.score("fox", 0);
        let s_stock = idx.score("stock", 2);
        assert!(s_stock > s_fox, "stock {s_stock} should beat fox {s_fox}");
    }

    #[test]
    fn empty_query_scores_zero() {
        let idx = fixture();
        assert_eq!(idx.score("", 0), 0.0);
    }
}
