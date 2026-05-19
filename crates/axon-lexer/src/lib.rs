//! Lexer for the Axon programming language.
//!
//! See `token.rs` for the token taxonomy and `lexer.rs` for the scanning
//! routine. The public surface is intentionally tiny: callers create a
//! [`SourceFile`] in `axon-diag` and pass it to [`tokenize`].

pub mod lexer;
pub mod token;

pub use lexer::Lexer;
pub use token::{
    AddrLitKind, Keyword, NumBase, StringKind, StringPart, Token, TokenKind, Tz,
    KNOWN_CURRENCIES,
};

use axon_diag::{Diagnostic, SourceFile};

/// Convenience entry point: tokenize a full source file.
pub fn tokenize(source: &SourceFile) -> (Vec<Token>, Vec<Diagnostic>) {
    Lexer::new(source).tokenize()
}
