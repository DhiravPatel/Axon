//! `axon-rag` — retrieval-augmented-generation primitives.
//!
//! Stage 12 ships the pure-Rust pieces that don't need a network:
//!
//!   * [`chunk`] — recursive text splitter (paragraph → sentence → word).
//!   * [`HashEmbedder`] — deterministic, feature-hashed embeddings; perfect
//!     for tests and for offline indexing. A real `RemoteEmbedder` trait
//!     impl can ship in Stage 12.1 without changing call sites.
//!   * [`Index`] — in-memory vector index with cosine-similarity search +
//!     BM25 over the chunked text. Hybrid scoring blends both.
//!   * [`Retriever`] — top-k search with optional re-ranking + filtering.
//!
//! Everything is built so the same code path is used by the CLI's `rag_*`
//! native bindings and by the unit tests below.

pub mod bm25;
pub mod chunk;
pub mod embed;
pub mod grounding;
pub mod index;
pub mod retrieve;

pub use bm25::Bm25;
pub use chunk::{Chunk, Chunker, RecursiveChunker};
pub use embed::{Embedder, HashEmbedder};
pub use grounding::{
    assess_grounding, Citation, CitationCheck, CitationPassage, ClaimAssessment,
    GroundingConfig, GroundingReport,
};
pub use index::{Index, Passage};
pub use retrieve::{Hit, Retriever};
