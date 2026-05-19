//! The pure analysis pipeline.
//!
//! Given source text, runs parse + type-check and surfaces every
//! diagnostic in a form the LSP server can translate to
//! `Diagnostic`-protocol messages. The result also exposes the parsed
//! `Program` and the type-checker `Ctx` so downstream queries (hover,
//! go-to-definition, completion) don't have to re-parse.

use axon_ast::Program;
use axon_diag::SourceFile;
use axon_tyck::Ctx;

/// Output of one analysis pass over a document.
pub struct Analysis {
    pub source: SourceFile,
    pub program: Program,
    pub ctx: Ctx,
    pub diagnostics: Vec<axon_diag::Diagnostic>,
}

/// Run the full parse + type-check pipeline against the given text.
/// The returned `SourceFile` has `file_id == 0` (a default LSP server
/// holds one document per URI, so we don't need a registry).
pub fn analyze(uri: &str, text: &str) -> Analysis {
    let source = SourceFile::new(uri.to_string(), text.to_string());
    let (program, parse_diags) = axon_parser::parse(&source);
    let (ctx, tyck_diags) = axon_tyck::check(&source, &program);
    let mut diagnostics = parse_diags;
    diagnostics.extend(tyck_diags);
    Analysis {
        source,
        program,
        ctx,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_clean_program_produces_no_diagnostics() {
        let a = analyze("file://t.ax", "fn f() -> Int { 1 }");
        assert!(a.diagnostics.is_empty(), "got: {:#?}", a.diagnostics);
    }

    #[test]
    fn analyze_type_error_surfaces() {
        let a = analyze("file://t.ax", "fn f() -> Int { \"hi\" }");
        assert!(!a.diagnostics.is_empty());
        assert!(a
            .diagnostics
            .iter()
            .any(|d| d.code == Some("E0211")));
    }

    #[test]
    fn analyze_parse_error_surfaces() {
        let a = analyze("file://t.ax", "fn f( {");
        assert!(!a.diagnostics.is_empty());
    }
}
