//! Parser for the Axon programming language.
//!
//! The parser is a hand-written recursive-descent + Pratt-precedence
//! expression parser. It consumes the token stream from `axon-lexer` and
//! emits an `axon-ast` [`Program`]. Strict fidelity to §43 of the spec is
//! the goal; less-detailed sub-grammars (policy rules, select arms, network
//! topology edges) are currently captured as raw source text so the surface
//! shape parses while the inner grammar matures.
//!
//! The entry point is [`parse`] (`fn parse(file: &SourceFile) -> (Program, Vec<Diagnostic>)`).

mod parser;

pub use parser::parse;
