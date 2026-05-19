//! Integration tests for the Axon lexer.
//!
//! Each test pins behavior against the §9 lexical-structure spec. When you
//! change the lexer, run `cargo test -p axon-lexer` — failing tests usually
//! indicate either a regression or a deliberate spec amendment that needs to
//! land in this file *and* the README at the same time.

use axon_diag::SourceFile;
use axon_lexer::{
    tokenize, AddrLitKind, Keyword as Kw, NumBase, StringKind, StringPart, TokenKind, Tz,
};

fn kinds(src: &str) -> Vec<TokenKind> {
    let file = SourceFile::new("t.ax", src);
    let (toks, diags) = tokenize(&file);
    assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    toks.into_iter()
        .map(|t| t.kind)
        .filter(|k| !matches!(k, TokenKind::Newline | TokenKind::Eof))
        .collect()
}

fn kinds_with_diags(src: &str) -> (Vec<TokenKind>, Vec<axon_diag::Diagnostic>) {
    let file = SourceFile::new("t.ax", src);
    let (toks, diags) = tokenize(&file);
    (
        toks.into_iter()
            .map(|t| t.kind)
            .filter(|k| !matches!(k, TokenKind::Newline | TokenKind::Eof))
            .collect(),
        diags,
    )
}

#[test]
fn keywords_are_all_recognized() {
    // Spans every keyword listed in §9.3.
    let src = "agent actor and as ask async await break case chan const continue \
               defer do effect else enum export extern fn for gen generate graph \
               if impl import in is let match memory model mut nil not on or plan \
               prompt pub recover replay return schema select self spawn state \
               stream struct supervisor trait try tool type use uses var when \
               while with yield";
    let k = kinds(src);
    for kind in &k {
        match kind {
            TokenKind::Keyword(_) => {}
            other => panic!("expected only keywords, got {other:?}"),
        }
    }
}

#[test]
fn identifiers_are_nfc_normalized() {
    // Two visually identical strings — one composed, one decomposed.
    let composed = "café";
    let decomposed = "cafe\u{0301}";
    let k1 = kinds(composed);
    let k2 = kinds(decomposed);
    assert_eq!(k1, k2);
    assert!(matches!(&k1[0], TokenKind::Ident(s) if s == composed));
}

#[test]
fn line_and_block_and_doc_comments() {
    let src = "// line\n/* block /* nested */ tail */ /// doc\n//! mod doc\nfoo";
    let file = SourceFile::new("t.ax", src);
    let (toks, diags) = tokenize(&file);
    assert!(diags.is_empty(), "{diags:?}");
    let nontrivia: Vec<_> = toks
        .iter()
        .map(|t| &t.kind)
        .filter(|k| !matches!(k, TokenKind::Newline | TokenKind::Eof))
        .collect();
    use TokenKind::*;
    assert!(matches!(nontrivia[0], LineComment));
    assert!(matches!(nontrivia[1], BlockComment));
    assert!(matches!(nontrivia[2], DocComment(s) if s == "doc"));
    assert!(matches!(nontrivia[3], ModDocComment(s) if s == "mod doc"));
    assert!(matches!(nontrivia[4], Ident(s) if s == "foo"));
}

#[test]
fn integer_bases_and_underscores() {
    let k = kinds("42 0xFF 0o17 0b1010 1_000_000");
    use TokenKind::*;
    assert!(matches!(k[0], Int { value: 42, base: NumBase::Dec }));
    assert!(matches!(k[1], Int { value: 255, base: NumBase::Hex }));
    assert!(matches!(k[2], Int { value: 15, base: NumBase::Oct }));
    assert!(matches!(k[3], Int { value: 10, base: NumBase::Bin }));
    assert!(matches!(k[4], Int { value: 1_000_000, base: NumBase::Dec }));
}

#[test]
fn floats_and_decimal() {
    let k = kinds("3.14 6.022e23 1_0.5 1.99dec 0.1dec");
    use TokenKind::*;
    assert!(matches!(&k[0], Float { lexeme } if lexeme == "3.14"));
    assert!(matches!(&k[1], Float { lexeme } if lexeme == "6.022e23"));
    assert!(matches!(&k[2], Float { lexeme } if lexeme == "1_0.5"));
    assert!(matches!(&k[3], Decimal { lexeme } if lexeme == "1.99"));
    assert!(matches!(&k[4], Decimal { lexeme } if lexeme == "0.1"));
}

#[test]
fn duration_all_units_normalize_to_nanoseconds() {
    let k = kinds("500ms 30s 5m 2h 1d");
    use TokenKind::*;
    let mut nanos = Vec::new();
    for t in k {
        match t {
            Duration { nanos: n, .. } => nanos.push(n),
            other => panic!("expected Duration, got {other:?}"),
        }
    }
    assert_eq!(nanos[0], 500 * 1_000_000);
    assert_eq!(nanos[1], 30 * 1_000_000_000);
    assert_eq!(nanos[2], 5 * 60 * 1_000_000_000);
    assert_eq!(nanos[3], 2 * 60 * 60 * 1_000_000_000);
    assert_eq!(nanos[4], 24 * 60 * 60 * 1_000_000_000);
}

#[test]
fn money_suffix_and_symbol_forms() {
    let k = kinds("1.50usd €2.00 ₹150inr 1000jpy");
    use TokenKind::*;
    assert!(
        matches!(&k[0], Money { amount, currency } if amount == "1.50" && currency == "usd"),
        "k[0] = {:?}",
        k[0]
    );
    assert!(
        matches!(&k[1], Money { amount, currency } if amount == "2.00" && currency == "eur"),
        "k[1] = {:?}",
        k[1]
    );
    assert!(
        matches!(&k[2], Money { amount, currency } if amount == "150" && currency == "inr"),
        "k[2] = {:?}",
        k[2]
    );
    assert!(
        matches!(&k[3], Money { amount, currency } if amount == "1000" && currency == "jpy"),
        "k[3] = {:?}",
        k[3]
    );
}

#[test]
fn unknown_numeric_suffix_is_a_lex_error() {
    let (_k, diags) = kinds_with_diags("42xyz");
    assert!(diags.iter().any(|d| d.message.contains("unknown numeric suffix")));
}

#[test]
fn date_datetime_time() {
    let k = kinds("2026-01-10 2026-01-10T14:30:00Z 14:30:00");
    use TokenKind::*;
    assert!(matches!(
        &k[0],
        Date { y: 2026, m: 1, d: 10 }
    ));
    assert!(matches!(
        &k[1],
        DateTime { y: 2026, m: 1, d: 10, hh: 14, mm: 30, ss: 0, tz: Tz::Utc }
    ));
    assert!(matches!(&k[2], Time { hh: 14, mm: 30, ss: 0 }));
}

#[test]
fn strings_with_escapes_and_interpolation() {
    let k = kinds(r#""hello" "esc \" quote" "interp {name} and {a + b}""#);
    use TokenKind::*;
    let TokenKind::String { parts, kind } = &k[0] else {
        panic!()
    };
    assert!(matches!(kind, StringKind::Regular));
    assert!(matches!(&parts[0], StringPart::Text(s) if s == "hello"));

    let TokenKind::String { parts, .. } = &k[1] else {
        panic!()
    };
    assert!(matches!(&parts[0], StringPart::Text(s) if s == "esc \" quote"));

    let TokenKind::String { parts, .. } = &k[2] else {
        panic!()
    };
    // Expect alternating text/interp/text/interp.
    assert!(matches!(&parts[0], StringPart::Text(s) if s == "interp "));
    assert!(matches!(&parts[1], StringPart::Interp { text, .. } if text == "name"));
    assert!(matches!(&parts[2], StringPart::Text(s) if s == " and "));
    assert!(matches!(&parts[3], StringPart::Interp { text, .. } if text == "a + b"));
}

#[test]
fn raw_string_does_not_interpret_escapes_or_interp() {
    let k = kinds("`raw \\n {no} {{interp}}`");
    use TokenKind::*;
    let TokenKind::String { kind, parts } = &k[0] else {
        panic!()
    };
    assert!(matches!(kind, StringKind::Raw));
    assert!(matches!(&parts[0], StringPart::Text(s) if s == "raw \\n {no} {{interp}}"));
}

#[test]
fn multiline_string_dedents_to_closing_fence() {
    let src = "\"\"\"\n    line one\n    line two\n    \"\"\"";
    let k = kinds(src);
    let TokenKind::String { kind, parts } = &k[0] else {
        panic!()
    };
    assert!(matches!(kind, StringKind::MultiLine));
    assert!(matches!(&parts[0], StringPart::Text(s) if s == "line one\nline two\n"));
}

#[test]
fn prompt_literal_is_distinct_string_kind() {
    let k = kinds("prompt\"\"\"You are helpful. user: {msg}\"\"\"");
    use TokenKind::*;
    let TokenKind::String { kind, parts: _ } = &k[0] else {
        panic!()
    };
    assert!(matches!(kind, StringKind::Prompt));
}

#[test]
fn byte_string_is_distinct_kind() {
    let k = kinds(r#"b"raw bytes \x00\xff""#);
    use TokenKind::*;
    let TokenKind::String { kind, parts } = &k[0] else {
        panic!()
    };
    assert!(matches!(kind, StringKind::Bytes));
    assert!(matches!(&parts[0], StringPart::Text(s) if s == "raw bytes \x00\u{ff}"));
}

#[test]
fn unicode_escape_in_string() {
    let k = kinds(r#""smile \u{1F600}""#);
    let TokenKind::String { parts, .. } = &k[0] else {
        panic!()
    };
    assert!(matches!(&parts[0], StringPart::Text(s) if s == "smile \u{1F600}"));
}

#[test]
fn char_literals_with_escape_and_unicode() {
    let k = kinds(r#"'A' '\n' '\u{1F600}'"#);
    use TokenKind::*;
    assert!(matches!(k[0], Char('A')));
    assert!(matches!(k[1], Char('\n')));
    assert!(matches!(k[2], Char('\u{1F600}')));
}

#[test]
fn hash_literal_and_attribute_open() {
    let k = kinds("#sha256:ab12cd34 #[foo]");
    use TokenKind::*;
    assert!(matches!(&k[0], HashLit { algo, hex } if algo == "sha256" && hex == "ab12cd34"));
    assert!(matches!(k[1], HashLBracket));
    assert!(matches!(&k[2], Ident(s) if s == "foo"));
    assert!(matches!(k[3], RBracket));
}

#[test]
fn agent_address_literals() {
    // `@{...}` (dynamic) stays fused because the `{` here introduces an
    // expression, not a brace literal. `@alice` and friends are emitted as
    // two tokens (`At` + `Ident`) so the parser can distinguish attributes
    // from address literals contextually.
    let k = kinds("@alice @{dyn_handle(x)} @retry");
    use TokenKind::*;
    assert!(matches!(k[0], At));
    assert!(matches!(&k[1], Ident(s) if s == "alice"));
    assert!(matches!(&k[2], AddrLit { kind: AddrLitKind::Dynamic, text } if text == "dyn_handle(x)"));
    assert!(matches!(k[3], At));
    assert!(matches!(&k[4], Ident(s) if s == "retry"));
}

#[test]
fn bare_at_before_punct() {
    let k = kinds("@ (");
    use TokenKind::*;
    assert!(matches!(k[0], At));
    assert!(matches!(k[1], LParen));
}

#[test]
fn punctuation_and_multi_char_operators() {
    let k = kinds("-> => |> .. ..= == != <= >= << >> && || += -= *= /= %= &");
    use TokenKind::*;
    let expected = [
        Arrow, FatArrow, Pipeline, DotDot, DotDotEq, EqEq, NotEq, LtEq, GtEq, Shl, Shr,
        AmpAmp, PipePipe, PlusEq, MinusEq, StarEq, SlashEq, PercentEq, Amp,
    ];
    assert_eq!(k.len(), expected.len(), "got {k:?}");
    for (i, e) in expected.iter().enumerate() {
        assert_eq!(
            std::mem::discriminant(&k[i]),
            std::mem::discriminant(e),
            "mismatch at {i}: got {:?}, expected {e:?}",
            k[i]
        );
    }
}

#[test]
fn newlines_are_emitted_as_tokens() {
    let src = "let x = 1\nlet y = 2\n";
    let file = SourceFile::new("t.ax", src);
    let (toks, _) = tokenize(&file);
    let newlines = toks
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Newline))
        .count();
    assert_eq!(newlines, 2);
}

#[test]
fn keyword_lookup_lowercase_only() {
    // Capitalized "Let" is just an identifier — keywords are §9.3 exact-match.
    let k = kinds("let Let");
    assert!(matches!(k[0], TokenKind::Keyword(Kw::Let)));
    assert!(matches!(&k[1], TokenKind::Ident(s) if s == "Let"));
}

#[test]
fn unterminated_string_produces_diagnostic() {
    let (_k, diags) = kinds_with_diags("\"oops");
    assert!(diags.iter().any(|d| d.message.contains("unterminated")));
}

#[test]
fn readme_researcher_snippet_lexes_without_diagnostics() {
    // First few lines of the README's marquee example — a smoke test that
    // the lexer handles real Axon code.
    let src = r#"agent Researcher(model: Model, tools: { search: Tool }, mem: Memory) {
    on ask(question: Tainted<String>) -> Answer uses { LLM, Net, Memory } {
        let ctx = mem.recall(question.text, k = 6) await
        return plan with self.model {
            system: "Answer only from sources found via the search tool."
            user:   question
            tools:  [self.tools.search]
            output: Answer
            budget: budget(usd = 0.05, tokens = 20_000)
        } await
    }
}"#;
    let file = SourceFile::new("researcher.ax", src);
    let (_, diags) = tokenize(&file);
    assert!(diags.is_empty(), "unexpected diagnostics: {diags:#?}");
}
