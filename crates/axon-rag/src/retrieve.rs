//! Top-k retrieval with hybrid scoring.

use serde::{Deserialize, Serialize};

use crate::embed::Embedder;
use crate::index::{Index, Passage};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hit {
    pub passage: Passage,
    pub vector_score: f32,
    pub lexical_score: f32,
    pub score: f32,
}

#[derive(Clone, Debug)]
pub struct Retriever<'a, E: Embedder + ?Sized> {
    pub index: &'a Index,
    pub embedder: &'a E,
    /// Weight on the dense (cosine) component. `1.0 - alpha` goes to BM25.
    pub alpha: f32,
}

impl<'a, E: Embedder + ?Sized> Retriever<'a, E> {
    pub fn new(index: &'a Index, embedder: &'a E) -> Self {
        Self {
            index,
            embedder,
            alpha: 0.7,
        }
    }

    pub fn with_alpha(mut self, alpha: f32) -> Self {
        assert!((0.0..=1.0).contains(&alpha), "alpha must be in [0, 1]");
        self.alpha = alpha;
        self
    }

    pub fn retrieve(&self, query: &str, k: usize) -> Vec<Hit> {
        if self.index.is_empty() || k == 0 {
            return Vec::new();
        }
        let q_vec = self.embedder.embed(query);
        let vec_scores = self.index.vector_scores(&q_vec);
        let lex_scores = self.index.lexical_scores(query);

        // Normalize lexical scores to [0, 1] so the blended hybrid score is
        // comparable to cosine. Vector scores from L2-unit embeddings are
        // already roughly in [-1, 1]; we map to [0, 1] for fairness.
        let max_lex = lex_scores.iter().copied().fold(0.0f32, f32::max).max(1e-6);

        let mut hits: Vec<Hit> = self
            .index
            .passages
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let v = ((vec_scores[i] + 1.0) * 0.5).clamp(0.0, 1.0);
                let l = (lex_scores[i] / max_lex).clamp(0.0, 1.0);
                let s = self.alpha * v + (1.0 - self.alpha) * l;
                Hit {
                    passage: p.clone(),
                    vector_score: vec_scores[i],
                    lexical_score: lex_scores[i],
                    score: s,
                }
            })
            .collect();
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(k);
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use crate::embed::HashEmbedder;

    fn corpus() -> (Index, HashEmbedder) {
        let e = HashEmbedder::new(512);
        let mut idx = Index::new(512);
        for (i, text) in [
            "Ferret are small carnivorous mammal related to weasel and mink.",
            "Ferret sleep up to eighteen hours a day and are most active at dawn.",
            "The stock market closed lower today, with tech leading declines.",
            "Yields on ten year treasuries climbed three basis points after the auction.",
            "Domestic ferret can be trained to use a litter box and respond to names.",
        ]
        .into_iter()
        .enumerate()
        {
            idx.add(
                &e,
                Chunk {
                    source: "kb".into(),
                    ordinal: i as u32,
                    text: text.into(),
                },
                Default::default(),
            );
        }
        (idx, e)
    }

    #[test]
    fn retrieves_topical_passage_top_1() {
        let (idx, e) = corpus();
        let r = Retriever::new(&idx, &e);
        let hits = r.retrieve("how long does a ferret sleep", 1);
        assert_eq!(hits.len(), 1);
        let text = &hits[0].passage.chunk.text;
        assert!(
            text.contains("Ferret") || text.contains("ferret"),
            "got: {text}"
        );
    }

    #[test]
    fn finance_query_skips_ferret_passages() {
        let (idx, e) = corpus();
        let r = Retriever::new(&idx, &e);
        let hits = r.retrieve("treasuries yield auction basis", 1);
        assert_eq!(hits.len(), 1);
        let top = &hits[0].passage.chunk.text;
        assert!(
            top.contains("treasuries") || top.contains("yield"),
            "expected a finance passage, got: {top}"
        );
    }

    #[test]
    fn k_zero_returns_empty() {
        let (idx, e) = corpus();
        let r = Retriever::new(&idx, &e);
        assert!(r.retrieve("anything", 0).is_empty());
    }

    #[test]
    fn alpha_zero_is_pure_lexical() {
        let (idx, e) = corpus();
        let r = Retriever::new(&idx, &e).with_alpha(0.0);
        let hits = r.retrieve("treasuries", 3);
        // Pure lexical: the treasury passage should be on top.
        assert!(hits[0].passage.chunk.text.contains("treasur"));
    }

    #[test]
    fn alpha_one_is_pure_vector() {
        let (idx, e) = corpus();
        let r = Retriever::new(&idx, &e).with_alpha(1.0);
        let hits = r.retrieve("financial markets and bond yields", 3);
        // The retrieved chunk's score field should equal the normalized
        // vector score (no lexical contribution).
        let normalized = ((hits[0].vector_score + 1.0) * 0.5).clamp(0.0, 1.0);
        assert!((hits[0].score - normalized).abs() < 1e-4);
    }
}
