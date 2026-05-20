//! Grounding & citations (§50.2 / §50.3).
//!
//! After a model generates an answer that's supposed to be backed by
//! retrieved passages, we still have to *verify* that every claim in
//! the answer is entailed by those passages. Hallucination is what
//! happens when this check is skipped.
//!
//! The library ships three pieces:
//!
//!   * [`CitationPassage`] — a citable unit: id, text, source, optional URL.
//!   * [`Citation`] — a span of the answer marked as attributed to one
//!     or more passage IDs.
//!   * [`GroundingReport`] — output of `assess_grounding`: a per-claim
//!     map from claim text to the passages that support it (if any), a
//!     per-citation validity check, and an aggregate score in [0, 1].
//!
//! The match is **lexical** for v0 — claim is grounded if every
//! ≥5-character word in it appears in the supporting passages, modulo
//! a small stop-word list. A semantic verifier (embedding similarity,
//! NLI model) can drop in behind `Verifier` without changing call
//! sites.
//!
//! Note: this `CitationPassage` is the *grounding* view of a passage
//! (id + text + source + url). The richer `Passage` in [`crate::index`]
//! also carries chunk + embedding for retrieval — they're deliberately
//! distinct so this module doesn't drag the vector machinery in.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationPassage {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    /// Substring of the answer that this citation covers.
    pub span: String,
    /// IDs of passages that back the span.
    pub passage_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimAssessment {
    pub claim: String,
    /// IDs of passages that support this claim. Empty = ungrounded.
    pub supporting_passage_ids: Vec<String>,
    /// Per-word overlap fraction with the union of supporting passages.
    pub overlap_score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CitationCheck {
    pub citation: Citation,
    /// True when every named passage exists in the supplied set AND the
    /// span's content words appear in at least one of them.
    pub ok: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroundingReport {
    pub claims: Vec<ClaimAssessment>,
    pub citations: Vec<CitationCheck>,
    /// Fraction of claims with at least one supporting passage.
    pub grounded_fraction: f64,
    /// Fraction of citations whose `ok` is true.
    pub citation_validity: f64,
    /// True when both fractions are >= the configured threshold.
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroundingConfig {
    /// Minimum overlap fraction (per claim) to call a claim grounded.
    pub min_overlap: f64,
    /// Required grounded-fraction across all claims.
    pub grounded_threshold: f64,
    /// Required citation-validity fraction.
    pub citation_threshold: f64,
}

impl Default for GroundingConfig {
    fn default() -> Self {
        Self {
            min_overlap: 0.6,
            grounded_threshold: 0.8,
            citation_threshold: 1.0,
        }
    }
}

/// Decompose `answer` into claims (sentence-split), score each against
/// `passages`, and verify each `citation`.
pub fn assess_grounding(
    answer: &str,
    passages: &[CitationPassage],
    citations: &[Citation],
    cfg: &GroundingConfig,
) -> GroundingReport {
    let passages_by_id: BTreeMap<&str, &CitationPassage> =
        passages.iter().map(|p| (p.id.as_str(), p)).collect();
    let claims = split_into_claims(answer);

    let mut claim_results: Vec<ClaimAssessment> = Vec::new();
    let mut grounded_count = 0usize;
    for c in &claims {
        let (supporting, score) = best_supporters(c, passages, cfg.min_overlap);
        if !supporting.is_empty() {
            grounded_count += 1;
        }
        claim_results.push(ClaimAssessment {
            claim: c.clone(),
            supporting_passage_ids: supporting,
            overlap_score: score,
        });
    }

    let mut citation_results: Vec<CitationCheck> = Vec::new();
    let mut valid_citations = 0usize;
    for cit in citations {
        let missing: Vec<&str> = cit
            .passage_ids
            .iter()
            .filter(|pid| !passages_by_id.contains_key(pid.as_str()))
            .map(|s| s.as_str())
            .collect();
        if !missing.is_empty() {
            citation_results.push(CitationCheck {
                citation: cit.clone(),
                ok: false,
                message: format!("unknown passage IDs: {}", missing.join(", ")),
            });
            continue;
        }
        let supporters: Vec<&CitationPassage> = cit
            .passage_ids
            .iter()
            .filter_map(|pid| passages_by_id.get(pid.as_str()).copied())
            .collect();
        let overlap = content_overlap(&cit.span, &supporters);
        let ok = overlap >= cfg.min_overlap;
        if ok {
            valid_citations += 1;
        }
        citation_results.push(CitationCheck {
            citation: cit.clone(),
            ok,
            message: if ok {
                String::new()
            } else {
                format!(
                    "span overlap {:.2} below threshold {:.2}",
                    overlap, cfg.min_overlap
                )
            },
        });
    }

    let grounded_fraction = if claims.is_empty() {
        1.0
    } else {
        grounded_count as f64 / claims.len() as f64
    };
    let citation_validity = if citations.is_empty() {
        1.0
    } else {
        valid_citations as f64 / citations.len() as f64
    };
    let passed = grounded_fraction >= cfg.grounded_threshold
        && citation_validity >= cfg.citation_threshold;

    GroundingReport {
        claims: claim_results,
        citations: citation_results,
        grounded_fraction,
        citation_validity,
        passed,
    }
}

fn best_supporters(
    claim: &str,
    passages: &[CitationPassage],
    min_overlap: f64,
) -> (Vec<String>, f64) {
    let mut best_overlap = 0.0f64;
    let mut hits: Vec<String> = Vec::new();
    for p in passages {
        let o = content_overlap(claim, &[p]);
        if o >= min_overlap {
            hits.push(p.id.clone());
            if o > best_overlap {
                best_overlap = o;
            }
        }
    }
    (hits, best_overlap)
}

fn content_overlap(text: &str, sources: &[&CitationPassage]) -> f64 {
    let claim_words = content_words(text);
    if claim_words.is_empty() {
        return 1.0;
    }
    let mut combined = String::new();
    for s in sources {
        combined.push(' ');
        combined.push_str(&s.text);
    }
    let lower = combined.to_lowercase();
    let mut hits = 0usize;
    for w in &claim_words {
        if lower.contains(w.as_str()) {
            hits += 1;
        }
    }
    hits as f64 / claim_words.len() as f64
}

fn split_into_claims(answer: &str) -> Vec<String> {
    answer
        .split(|c: char| matches!(c, '.' | '?' | '!' | '\n'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn content_words(text: &str) -> Vec<String> {
    let stop: BTreeSet<&str> = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "and",
        "or", "but", "if", "then", "else", "for", "in", "on", "at", "to", "of",
        "from", "by", "with", "as", "that", "this", "these", "those", "it", "its",
        "we", "you", "they", "i", "he", "she", "them", "his", "her", "their",
        "do", "does", "did", "have", "has", "had", "will", "would", "should",
        "can", "could", "may", "might", "must", "not", "no", "yes",
    ]
    .iter()
    .copied()
    .collect();
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 5)
        .filter(|w| !stop.contains(w))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage(id: &str, text: &str) -> CitationPassage {
        CitationPassage {
            id: id.into(),
            text: text.into(),
            source: String::new(),
            url: String::new(),
        }
    }

    #[test]
    fn fully_grounded_answer_passes() {
        let passages = vec![
            passage(
                "p1",
                "The 2024 European Union AI Act was originally adopted to regulate high-risk AI systems.",
            ),
            passage(
                "p2",
                "Amendments in 2025 tightened transparency requirements for foundation models.",
            ),
        ];
        let answer = "The 2024 European Union AI Act regulated high-risk systems. Amendments in 2025 tightened transparency.";
        let cfg = GroundingConfig::default();
        let r = assess_grounding(answer, &passages, &[], &cfg);
        assert!(r.grounded_fraction >= 0.99, "got {}", r.grounded_fraction);
        assert!(r.passed);
    }

    #[test]
    fn ungrounded_claims_drop_the_score() {
        let passages = vec![passage("p1", "Cats nap during the day.")];
        let answer = "Cats nap during the day. Dolphins compose symphonies under water.";
        let cfg = GroundingConfig::default();
        let r = assess_grounding(answer, &passages, &[], &cfg);
        assert!((r.grounded_fraction - 0.5).abs() < 1e-9);
        assert!(!r.passed);
    }

    #[test]
    fn citation_to_unknown_passage_is_invalid() {
        let passages = vec![passage("p1", "Hello world")];
        let cits = vec![Citation {
            span: "Hello".into(),
            passage_ids: vec!["nonexistent".into()],
        }];
        let cfg = GroundingConfig::default();
        let r = assess_grounding("Hello world.", &passages, &cits, &cfg);
        assert!(r.citations[0].message.contains("unknown passage"));
        assert!(!r.citations[0].ok);
    }

    #[test]
    fn citation_span_overlap_must_meet_threshold() {
        let passages = vec![passage(
            "p1",
            "Mountains rise above the surrounding plains.",
        )];
        let bad = vec![Citation {
            span: "rapid global inflation reshaped markets".into(),
            passage_ids: vec!["p1".into()],
        }];
        let cfg = GroundingConfig::default();
        let r = assess_grounding("rapid global inflation reshaped markets.", &passages, &bad, &cfg);
        assert!(!r.citations[0].ok);
        assert!(r.citations[0].message.contains("below threshold"));
    }

    #[test]
    fn stop_words_dont_pollute_overlap() {
        // Claim is just stop words — overlap defaults to 1.0 so we
        // don't penalize "the cat is the cat" against an empty passage.
        let passages = vec![passage("p1", "")];
        let r = assess_grounding("The is the.", &passages, &[], &GroundingConfig::default());
        // Only one claim; all words are stop words → claim has no content.
        // Empty content_words → overlap returns 1.0 → claim grounded.
        assert!(r.grounded_fraction >= 0.99);
    }

    #[test]
    fn config_thresholds_round_trip_through_json() {
        let cfg = GroundingConfig {
            min_overlap: 0.7,
            grounded_threshold: 0.85,
            citation_threshold: 0.95,
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: GroundingConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, cfg);
    }
}
