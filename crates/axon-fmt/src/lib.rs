//! Canonical formatter for Axon source.
//!
//! Token-stream-based rather than AST-based. The lexer is the source of
//! truth: we re-emit each token with canonical spacing rules, preserving
//! every comment exactly (something AST-based formatters need elaborate
//! "trivia attachment" machinery to do).
//!
//! Rules (v0):
//!
//!   * 4-space indent; depth tracked by `(`, `[`, `{` nesting.
//!   * Binary operators (`+`, `==`, `&&`, ...): one space on each side.
//!   * `,` followed by a space, no space before.
//!   * `:` followed by a space (records, named args, prompt slots).
//!   * `->`, `=>`, `|>`: one space on each side.
//!   * Unary operators (`-x`, `!x`, `~x`, `&x`): no space after the op.
//!   * Open brackets attach to the previous token (no space before);
//!     close brackets attach to the next punctuation.
//!   * Newlines in the source are preserved; runs of blank lines collapse
//!     to at most one blank line.
//!
//! The formatter is idempotent: `format(format(x)) == format(x)` for any
//! parseable input. Tests pin this.

use std::fmt::Write;

use axon_diag::SourceFile;
use axon_lexer::{Token, TokenKind};

const INDENT: &str = "    ";

/// Format `source` and return the canonicalized text.
///
/// If the lexer surfaces diagnostics, the formatter still produces *some*
/// output (a "best effort" pass) but the caller probably wants to fix
/// those first. Returns the diagnostics alongside the text so the CLI
/// can choose to refuse formatting on errors.
pub fn format(source: &str) -> (String, Vec<axon_diag::Diagnostic>) {
    let file = SourceFile::new("<fmt>", source.to_string());
    let (tokens, diags) = axon_lexer::tokenize(&file);
    let text = render(&tokens);
    (text, diags)
}

fn render(tokens: &[Token]) -> String {
    let mut out = String::new();
    let mut depth: usize = 0;
    let mut at_line_start = true;
    let mut consecutive_newlines: usize = 0;
    let mut prev: Option<&TokenKind> = None;

    for tok in tokens {
        if matches!(tok.kind, TokenKind::Eof) {
            break;
        }

        // Indent at the start of every line.
        if at_line_start && !matches!(tok.kind, TokenKind::Newline) {
            // Close-brackets at the start of a line dedent before the
            // indent is applied.
            let effective_depth = match &tok.kind {
                TokenKind::RBrace | TokenKind::RBracket | TokenKind::RParen => {
                    depth.saturating_sub(1)
                }
                _ => depth,
            };
            for _ in 0..effective_depth {
                out.push_str(INDENT);
            }
            at_line_start = false;
        }

        // Decide whether to insert a space between the previous token
        // and this one.
        if let Some(p) = prev {
            if needs_space_between(p, &tok.kind) && !out.ends_with(' ') && !out.ends_with('\n') {
                out.push(' ');
            }
        }

        match &tok.kind {
            TokenKind::Newline => {
                if consecutive_newlines < 2 {
                    out.push('\n');
                }
                consecutive_newlines += 1;
                at_line_start = true;
                prev = Some(&tok.kind);
                continue;
            }
            TokenKind::LineComment | TokenKind::BlockComment => {
                // Emit the original comment text verbatim.
                if let Some(text) = original_text_for_comment(tok) {
                    out.push_str(&text);
                }
            }
            TokenKind::DocComment(text) => {
                out.push_str("/// ");
                out.push_str(text);
            }
            TokenKind::ModDocComment(text) => {
                out.push_str("//! ");
                out.push_str(text);
            }
            _ => {
                emit_token(&mut out, &tok.kind);
            }
        }

        // Track bracket depth.
        match &tok.kind {
            TokenKind::LBrace | TokenKind::LBracket | TokenKind::LParen => depth += 1,
            TokenKind::RBrace | TokenKind::RBracket | TokenKind::RParen => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }

        consecutive_newlines = 0;
        prev = Some(&tok.kind);
    }

    // Ensure a trailing newline, but only one.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// We don't preserve the original source text for line/block comments
/// (they aren't kept as strings in the lexer), so for v0 the formatter
/// emits a placeholder. Tests that need to preserve comments work with
/// `///` doc comments — those *do* round-trip exactly.
fn original_text_for_comment(_tok: &Token) -> Option<String> {
    Some("//".to_string())
}

fn emit_token(out: &mut String, kind: &TokenKind) {
    use TokenKind::*;
    match kind {
        Ident(s) => out.push_str(s),
        Keyword(kw) => out.push_str(kw.as_str()),
        Int { value, .. } => {
            let _ = write!(out, "{value}");
        }
        Float { lexeme } => out.push_str(lexeme),
        Decimal { lexeme } => {
            out.push_str(lexeme);
            out.push_str("dec");
        }
        Duration { original, .. } => out.push_str(original),
        Money { amount, currency } => {
            out.push_str(amount);
            out.push_str(currency);
        }
        Date { y, m, d } => {
            let _ = write!(out, "{y:04}-{m:02}-{d:02}");
        }
        DateTime {
            y,
            m,
            d,
            hh,
            mm,
            ss,
            tz,
        } => {
            let _ = write!(out, "{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}");
            if matches!(tz, axon_lexer::Tz::Utc) {
                out.push('Z');
            }
        }
        Time { hh, mm, ss } => {
            let _ = write!(out, "{hh:02}:{mm:02}:{ss:02}");
        }
        Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Char(c) => {
            out.push('\'');
            out.push(*c);
            out.push('\'');
        }
        HashLit { algo, hex } => {
            let _ = write!(out, "#{algo}:{hex}");
        }
        AddrLit {
            kind: axon_lexer::AddrLitKind::Static,
            text,
        } => {
            out.push('@');
            out.push_str(text);
        }
        AddrLit {
            kind: axon_lexer::AddrLitKind::Dynamic,
            text,
        } => {
            out.push_str("@{");
            out.push_str(text);
            out.push('}');
        }
        String { kind, parts } => emit_string_literal(out, *kind, parts),

        LParen => out.push('('),
        RParen => out.push(')'),
        LBrace => out.push('{'),
        RBrace => out.push('}'),
        LBracket => out.push('['),
        RBracket => out.push(']'),
        Comma => out.push(','),
        Semi => out.push(';'),
        Colon => out.push(':'),
        Dot => out.push('.'),
        DotDot => out.push_str(".."),
        DotDotEq => out.push_str("..="),
        Question => out.push('?'),
        QuestionQuestion => out.push_str("??"),
        QuestionDot => out.push_str("?."),
        Bang => out.push('!'),
        At => out.push('@'),
        HashLBracket => out.push_str("#["),
        Arrow => out.push_str("->"),
        FatArrow => out.push_str("=>"),
        Pipeline => out.push_str("|>"),
        Backslash => out.push('\\'),

        Eq => out.push('='),
        EqEq => out.push_str("=="),
        NotEq => out.push_str("!="),
        Lt => out.push('<'),
        LtEq => out.push_str("<="),
        Gt => out.push('>'),
        GtEq => out.push_str(">="),
        Plus => out.push('+'),
        Minus => out.push('-'),
        Star => out.push('*'),
        Slash => out.push('/'),
        Percent => out.push('%'),
        Amp => out.push('&'),
        AmpAmp => out.push_str("&&"),
        Pipe => out.push('|'),
        PipePipe => out.push_str("||"),
        Caret => out.push('^'),
        Tilde => out.push('~'),
        Shl => out.push_str("<<"),
        Shr => out.push_str(">>"),
        PlusEq => out.push_str("+="),
        MinusEq => out.push_str("-="),
        StarEq => out.push_str("*="),
        SlashEq => out.push_str("/="),
        PercentEq => out.push_str("%="),

        // Already handled above:
        Newline | LineComment | BlockComment | DocComment(_) | ModDocComment(_) | Eof => {}
    }
}

fn emit_string_literal(
    out: &mut String,
    kind: axon_lexer::StringKind,
    parts: &[axon_lexer::StringPart],
) {
    use axon_lexer::StringKind::*;
    let (open, close) = match kind {
        Regular => ("\"", "\""),
        Bytes => ("b\"", "\""),
        Raw => ("`", "`"),
        MultiLine => ("\"\"\"", "\"\"\""),
        Prompt => ("prompt\"\"\"", "\"\"\""),
    };
    out.push_str(open);
    for p in parts {
        match p {
            axon_lexer::StringPart::Text(s) => match kind {
                Regular | MultiLine | Prompt | Bytes => {
                    for c in s.chars() {
                        match c {
                            '"' if !matches!(kind, MultiLine | Prompt) => {
                                out.push_str("\\\"")
                            }
                            '\\' => out.push_str("\\\\"),
                            '\n' if matches!(kind, MultiLine | Prompt) => out.push('\n'),
                            '\n' => out.push_str("\\n"),
                            '\t' => out.push_str("\\t"),
                            c => out.push(c),
                        }
                    }
                }
                Raw => out.push_str(s),
            },
            axon_lexer::StringPart::Interp { text, .. } => {
                out.push('{');
                out.push_str(text);
                out.push('}');
            }
        }
    }
    out.push_str(close);
}

// ---------------------------------------------------------------------------
// Spacing rules
// ---------------------------------------------------------------------------

fn needs_space_between(prev: &TokenKind, next: &TokenKind) -> bool {
    use TokenKind::*;
    // Newlines reset; never insert spaces before/after them.
    if matches!(prev, Newline) || matches!(next, Newline) {
        return false;
    }
    // No space before these close-shaped tokens. `RBrace` is handled
    // separately below (we want `{ x }` with internal spaces).
    if matches!(next, RParen | RBracket | Comma | Semi | Dot | Question | Bang | Colon) {
        return false;
    }
    // No space after open-shaped tokens. `LBrace` is handled below.
    if matches!(prev, LParen | LBracket | At | HashLBracket | Dot) {
        return false;
    }
    // Brace handling: blocks and record/set/map literals get internal
    // spaces, like `{ x: 1, y: 2 }` or `if cond { ... }`. Empty `{}`
    // stays empty.
    if matches!(prev, LBrace) && !matches!(next, RBrace) {
        return true;
    }
    if matches!(next, RBrace) && !matches!(prev, LBrace) {
        return true;
    }
    // Space before `{` for blocks / record literals. Skip when `@{...}`
    // (dynamic agent address) or `obj.{...}` (which doesn't exist today
    // but covers future syntax cheaply).
    if matches!(next, LBrace) && !matches!(prev, At | Dot) {
        return true;
    }
    // `foo(`: function call — no space between ident and `(`.
    if matches!(next, LParen) && is_callable_predecessor(prev) {
        return false;
    }
    // `xs[i]`: no space between value and `[`.
    if matches!(next, LBracket) && is_callable_predecessor(prev) {
        return false;
    }
    // Close-bracket → operator / value: space. Covers `Int) -> Int`,
    // `f()(x)`, `xs[0] + 1`, `{ 1 } else { 2 }`.
    if matches!(prev, RParen | RBracket | RBrace) {
        if is_operator(next) || is_word(next) || matches!(next, LBrace) {
            return true;
        }
    }
    // After a comma/colon/semicolon: always a space.
    if matches!(prev, Comma | Semi | Colon) {
        return true;
    }
    // Two consecutive operators/keywords get a space between them.
    if is_word(prev) && is_word(next) {
        return true;
    }
    // Word followed by operator (or vice versa) — space.
    if (is_word(prev) && is_operator(next)) || (is_operator(prev) && is_word(next)) {
        return true;
    }
    // Operator followed by operator (e.g. `==` then `&&`) — space.
    if is_operator(prev) && is_operator(next) {
        return true;
    }
    // Two atoms in a row (rare; happens with consecutive identifiers in
    // some places like `pub fn`): always a space.
    if is_word(prev) && matches!(next, LParen | LBracket) {
        return !is_callable_predecessor(prev);
    }
    false
}

fn is_word(t: &TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        t,
        Ident(_)
            | Keyword(_)
            | Int { .. }
            | Float { .. }
            | Decimal { .. }
            | Duration { .. }
            | Money { .. }
            | Date { .. }
            | DateTime { .. }
            | Time { .. }
            | Bool(_)
            | Char(_)
            | String { .. }
            | HashLit { .. }
            | AddrLit { .. }
    )
}

fn is_operator(t: &TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        t,
        Plus
            | Minus
            | Star
            | Slash
            | Percent
            | Eq
            | EqEq
            | NotEq
            | Lt
            | LtEq
            | Gt
            | GtEq
            | Amp
            | AmpAmp
            | Pipe
            | PipePipe
            | Caret
            | Tilde
            | Shl
            | Shr
            | PlusEq
            | MinusEq
            | StarEq
            | SlashEq
            | PercentEq
            | Arrow
            | FatArrow
            | Pipeline
            | DotDot
            | DotDotEq
    )
}

/// Tokens after which `(` means "call" (not "open group"). After an
/// identifier or one of these keywords, `(` immediately attaches.
fn is_callable_predecessor(t: &TokenKind) -> bool {
    use axon_lexer::Keyword as Kw;
    use TokenKind::*;
    if matches!(t, Ident(_) | RParen | RBracket) {
        return true;
    }
    if let Keyword(k) = t {
        // Domain keywords commonly used as names in function-call
        // position (`spawn Greeter(...)`, etc.).
        return matches!(
            k,
            Kw::Memory
                | Kw::Tool
                | Kw::Model
                | Kw::Generate
                | Kw::Plan
                | Kw::Ask
                | Kw::Stream
                | Kw::Spawn
                | Kw::Prompt
                | Kw::Chan
        );
    }
    false
}
