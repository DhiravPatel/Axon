//! Language Server Protocol implementation for Axon.
//!
//! Layout:
//!
//!   * [`analyze`] is the pure-data layer. Given source text it produces
//!     an [`Analysis`] with parse + type-check diagnostics, the AST, and
//!     the type-checker context. Easy to unit-test.
//!
//!   * [`position`] converts between LSP `Position { line, character }`
//!     (UTF-8 code units in v0) and Axon's byte offsets.
//!
//!   * [`query`] runs read-only queries on an analysis — "what item is at
//!     this position?", "where is `foo` defined?", "what completions
//!     apply here?". These power hover, go-to-definition, and completion.
//!
//!   * [`server`] is the LSP message loop. Drives JSON-RPC over stdio
//!     using `lsp-server`, dispatches requests to the analysis layer,
//!     publishes diagnostics back to the editor on every change.
//!
//! The pure layers are testable in isolation; the server layer is glued
//! together at the top.

pub mod analyze;
pub mod position;
pub mod query;
pub mod server;

pub use analyze::{analyze, Analysis};
pub use position::{offset_to_position, position_to_offset, span_to_range};
pub use server::run;
