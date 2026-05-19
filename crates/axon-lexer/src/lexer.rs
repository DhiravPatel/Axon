//! The Axon lexer.
//!
//! Input is UTF-8 [`SourceFile`] text. Output is a stream of [`Token`]s plus
//! any [`Diagnostic`]s recoverable lexing produced. Whitespace except newlines
//! is dropped silently; newlines are kept as `Newline` tokens because Axon is
//! newline-significant (§9.2).
//!
//! The lexer is single-pass, hand-written, and uses byte-level scanning except
//! when it actually needs the next Unicode scalar (identifier starts, currency
//! symbols, character literals).

use axon_diag::{Diagnostic, Severity, SourceFile, Span};
use unicode_normalization::UnicodeNormalization;

use crate::token::{
    symbol_to_currency, AddrLitKind, Keyword, NumBase, StringKind, StringPart, Token,
    TokenKind, Tz, KNOWN_CURRENCIES,
};

pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    file_id: u16,
    pub diagnostics: Vec<Diagnostic>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a SourceFile) -> Self {
        Self {
            src: source.text(),
            pos: 0,
            file_id: source.id(),
            diagnostics: Vec::new(),
        }
    }

    /// Drive the lexer to completion and return all tokens. An [`TokenKind::Eof`]
    /// token is always appended so the parser can rely on lookahead.
    pub fn tokenize(mut self) -> (Vec<Token>, Vec<Diagnostic>) {
        let mut out = Vec::new();
        while let Some(tok) = self.next_token() {
            out.push(tok);
        }
        out.push(Token {
            kind: TokenKind::Eof,
            span: Span::in_file(self.src.len(), self.src.len(), self.file_id),
        });
        (out, self.diagnostics)
    }

    // ---------------------------------------------------------------------
    // Cursor primitives
    // ---------------------------------------------------------------------

    fn peek_byte(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn peek_byte_at(&self, offset: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + offset).copied()
    }

    fn peek_char(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn bump_char(&mut self) -> Option<char> {
        let c = self.peek_char()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn bump_if(&mut self, expected: u8) -> bool {
        if self.peek_byte() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn starts_with(&self, lit: &str) -> bool {
        self.src[self.pos..].starts_with(lit)
    }

    fn span_from(&self, start: usize) -> Span {
        Span::in_file(start, self.pos, self.file_id)
    }

    fn span_at(&self, start: usize, end: usize) -> Span {
        Span::in_file(start, end, self.file_id)
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code: None,
            message: message.into(),
            primary: axon_diag::Label {
                span,
                message: None,
            },
            secondary: Vec::new(),
            notes: Vec::new(),
        });
    }

    // ---------------------------------------------------------------------
    // Top-level dispatch
    // ---------------------------------------------------------------------

    fn next_token(&mut self) -> Option<Token> {
        loop {
            // Skip horizontal whitespace.
            while let Some(b) = self.peek_byte() {
                if b == b' ' || b == b'\t' || b == b'\r' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let start = self.pos;
            let Some(b) = self.peek_byte() else {
                return None;
            };

            // Newline (§9.2).
            if b == b'\n' {
                self.pos += 1;
                return Some(Token {
                    kind: TokenKind::Newline,
                    span: self.span_from(start),
                });
            }

            // Comments (§9.1).
            if b == b'/' {
                if let Some(tok) = self.try_scan_comment(start) {
                    return Some(tok);
                }
            }

            // Identifier or keyword (and `prompt"""..."""`, `b"..."`).
            if is_ident_start(b) || (b >= 0x80 && self.peek_char().map_or(false, is_ident_start_char)) {
                return Some(self.scan_ident_or_keyword(start));
            }

            // ASCII digit → number / date / datetime.
            if b.is_ascii_digit() {
                return Some(self.scan_number(start));
            }

            // Currency-prefix money: `€`, `£`, `¥`, `₹`, `$` followed by digit.
            if let Some(c) = self.peek_char() {
                if symbol_to_currency(c).is_some() {
                    let symbol_len = c.len_utf8();
                    let next_is_digit = self
                        .src
                        .as_bytes()
                        .get(self.pos + symbol_len)
                        .map_or(false, |b| b.is_ascii_digit());
                    if next_is_digit {
                        return Some(self.scan_money_with_symbol(start, c));
                    }
                }
            }

            // String / char literals.
            if b == b'"' {
                // Could be a regular `"..."` or multi-line `"""..."""`.
                if self.starts_with("\"\"\"") {
                    return Some(self.scan_multiline_string(start, StringKind::MultiLine));
                }
                return Some(self.scan_string(start, StringKind::Regular));
            }
            if b == b'`' {
                return Some(self.scan_raw_string(start));
            }
            if b == b'\'' {
                return Some(self.scan_char(start));
            }

            // Time literal: HH:MM:SS where all three are exactly two digits.
            // (Already handled inside `scan_number` for digit starts, but a
            // bare leading colon does not begin a time.)

            // Hash literals and attribute `#[`.
            if b == b'#' {
                return Some(self.scan_hash(start));
            }

            // Agent address literal or stand-alone `@`.
            if b == b'@' {
                return Some(self.scan_at(start));
            }

            // Punctuation / operators.
            return Some(self.scan_punct(start));
        }
    }

    // ---------------------------------------------------------------------
    // Comments
    // ---------------------------------------------------------------------

    fn try_scan_comment(&mut self, start: usize) -> Option<Token> {
        match (self.peek_byte(), self.peek_byte_at(1)) {
            (Some(b'/'), Some(b'/')) => {
                self.pos += 2;
                let is_doc = self.peek_byte() == Some(b'/');
                let is_mod_doc = self.peek_byte() == Some(b'!');
                if is_doc || is_mod_doc {
                    self.pos += 1;
                }
                let text_start = self.pos;
                while let Some(b) = self.peek_byte() {
                    if b == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
                let text = self.src[text_start..self.pos].trim().to_string();
                let kind = if is_doc {
                    TokenKind::DocComment(text)
                } else if is_mod_doc {
                    TokenKind::ModDocComment(text)
                } else {
                    TokenKind::LineComment
                };
                Some(Token {
                    kind,
                    span: self.span_from(start),
                })
            }
            (Some(b'/'), Some(b'*')) => {
                self.pos += 2;
                let mut depth = 1usize;
                while depth > 0 {
                    match (self.peek_byte(), self.peek_byte_at(1)) {
                        (Some(b'/'), Some(b'*')) => {
                            self.pos += 2;
                            depth += 1;
                        }
                        (Some(b'*'), Some(b'/')) => {
                            self.pos += 2;
                            depth -= 1;
                        }
                        (Some(_), _) => {
                            // Advance one whole UTF-8 codepoint.
                            let _ = self.bump_char();
                        }
                        (None, _) => {
                            self.error(
                                "unterminated block comment",
                                self.span_from(start),
                            );
                            break;
                        }
                    }
                }
                Some(Token {
                    kind: TokenKind::BlockComment,
                    span: self.span_from(start),
                })
            }
            _ => None,
        }
    }

    // ---------------------------------------------------------------------
    // Identifiers, keywords, and the few identifier-prefixed literals
    // ---------------------------------------------------------------------

    fn scan_ident_or_keyword(&mut self, start: usize) -> Token {
        while let Some(c) = self.peek_char() {
            if is_ident_continue_char(c) {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
        let raw = &self.src[start..self.pos];

        // Identifier-prefixed literals.
        if raw == "b" && self.peek_byte() == Some(b'"') {
            return self.scan_string(start, StringKind::Bytes);
        }
        if raw == "prompt" {
            if self.starts_with("\"\"\"") {
                return self.scan_multiline_string(start, StringKind::Prompt);
            }
            if self.peek_byte() == Some(b'"') {
                return self.scan_string(start, StringKind::Prompt);
            }
            // Falls through and tokenizes `prompt` as a keyword.
        }
        if raw == "true" {
            return Token {
                kind: TokenKind::Bool(true),
                span: self.span_from(start),
            };
        }
        if raw == "false" {
            return Token {
                kind: TokenKind::Bool(false),
                span: self.span_from(start),
            };
        }

        let normalized: String = raw.nfc().collect();
        let kind = match Keyword::from_str(&normalized) {
            Some(kw) => TokenKind::Keyword(kw),
            None => TokenKind::Ident(normalized),
        };
        Token {
            kind,
            span: self.span_from(start),
        }
    }

    // ---------------------------------------------------------------------
    // Numeric, temporal, money literals
    // ---------------------------------------------------------------------

    fn scan_number(&mut self, start: usize) -> Token {
        // Try date / datetime first: pattern `DDDD-DD-DD[T...]`. Only if
        // exactly four leading digits and the next character is `-`.
        if self.looks_like_date(start) {
            return self.scan_date_or_datetime(start);
        }
        // Try time literal `DD:DD:DD`.
        if self.looks_like_time(start) {
            return self.scan_time(start);
        }

        // Hex / oct / bin prefix.
        if self.peek_byte() == Some(b'0') {
            match self.peek_byte_at(1) {
                Some(b'x') | Some(b'X') => return self.scan_int_radix(start, NumBase::Hex, 16),
                Some(b'o') | Some(b'O') => return self.scan_int_radix(start, NumBase::Oct, 8),
                Some(b'b') | Some(b'B') => return self.scan_int_radix(start, NumBase::Bin, 2),
                _ => {}
            }
        }

        // Decimal integer / float / decimal / duration / money.
        // Consume leading digits.
        let int_start = self.pos;
        self.eat_decimal_digits();
        let mut is_float = false;
        // Fractional part.
        if self.peek_byte() == Some(b'.')
            && self
                .peek_byte_at(1)
                .map_or(false, |b| b.is_ascii_digit())
        {
            is_float = true;
            self.pos += 1; // '.'
            self.eat_decimal_digits();
        }
        // Exponent.
        if matches!(self.peek_byte(), Some(b'e') | Some(b'E')) {
            // Only a real exponent if followed by optional sign and digit.
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek_byte(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if self.peek_byte().map_or(false, |b| b.is_ascii_digit()) {
                is_float = true;
                self.eat_decimal_digits();
            } else {
                self.pos = save;
            }
        }

        let num_text = &self.src[int_start..self.pos];

        // Trailing alphabetic suffix decides decimal/duration/money.
        let suffix_start = self.pos;
        while let Some(b) = self.peek_byte() {
            if b.is_ascii_alphabetic() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let suffix = &self.src[suffix_start..self.pos];

        if suffix.is_empty() {
            // Plain Int or Float.
            return self.finish_plain_number(start, num_text, is_float);
        }

        match suffix {
            "dec" => Token {
                kind: TokenKind::Decimal {
                    lexeme: num_text.to_string(),
                },
                span: self.span_from(start),
            },
            "ms" => self.finish_duration(start, num_text, 1_000_000, "ms"),
            "s" => self.finish_duration(start, num_text, 1_000_000_000, "s"),
            "m" => self.finish_duration(start, num_text, 60 * 1_000_000_000, "m"),
            "h" => self.finish_duration(start, num_text, 60 * 60 * 1_000_000_000, "h"),
            "d" => self.finish_duration(start, num_text, 24 * 60 * 60 * 1_000_000_000, "d"),
            cur if KNOWN_CURRENCIES.contains(&cur) => Token {
                kind: TokenKind::Money {
                    amount: num_text.to_string(),
                    currency: cur.to_string(),
                },
                span: self.span_from(start),
            },
            other => {
                self.error(
                    format!("unknown numeric suffix `{other}`"),
                    Span::in_file(suffix_start,  self.pos, self.file_id),
                );
                self.finish_plain_number(start, num_text, is_float)
            }
        }
    }

    fn finish_plain_number(&self, start: usize, text: &str, is_float: bool) -> Token {
        let span = Span::in_file(start,  self.pos, self.file_id);
        if is_float {
            return Token {
                kind: TokenKind::Float {
                    lexeme: text.to_string(),
                },
                span,
            };
        }
        let cleaned: String = text.chars().filter(|c| *c != '_').collect();
        let value = cleaned.parse::<i128>().unwrap_or(0);
        Token {
            kind: TokenKind::Int {
                value,
                base: NumBase::Dec,
            },
            span,
        }
    }

    fn finish_duration(
        &mut self,
        start: usize,
        text: &str,
        per_unit_nanos: i128,
        unit: &str,
    ) -> Token {
        let span = self.span_from(start);
        let cleaned: String = text.chars().filter(|c| *c != '_').collect();
        if let Ok(n) = cleaned.parse::<i128>() {
            return Token {
                kind: TokenKind::Duration {
                    nanos: n.saturating_mul(per_unit_nanos),
                    original: format!("{text}{unit}"),
                },
                span,
            };
        }
        // Fractional duration: parse as f64, convert to nanos. We accept the
        // float; precision loss for sub-millisecond fractions is accepted at
        // lex time and tracked in the original lexeme.
        if let Ok(f) = cleaned.parse::<f64>() {
            let nanos = (f * per_unit_nanos as f64) as i128;
            return Token {
                kind: TokenKind::Duration {
                    nanos,
                    original: format!("{text}{unit}"),
                },
                span,
            };
        }
        self.error(format!("invalid duration `{text}{unit}`"), span);
        Token {
            kind: TokenKind::Duration {
                nanos: 0,
                original: format!("{text}{unit}"),
            },
            span,
        }
    }

    fn scan_int_radix(&mut self, start: usize, base: NumBase, radix: u32) -> Token {
        self.pos += 2; // skip `0x` / `0o` / `0b`
        let digits_start = self.pos;
        while let Some(b) = self.peek_byte() {
            let c = b as char;
            if c == '_' || c.is_digit(radix) {
                self.pos += 1;
            } else {
                break;
            }
        }
        let raw = &self.src[digits_start..self.pos];
        let cleaned: String = raw.chars().filter(|c| *c != '_').collect();
        let span = self.span_from(start);
        let value = i128::from_str_radix(&cleaned, radix).unwrap_or_else(|_| {
            self.diagnostics.push(Diagnostic::error(
                "invalid numeric literal",
                span,
            ));
            0
        });
        Token {
            kind: TokenKind::Int { value, base },
            span,
        }
    }

    fn eat_decimal_digits(&mut self) {
        while let Some(b) = self.peek_byte() {
            if b.is_ascii_digit() || b == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn looks_like_date(&self, start: usize) -> bool {
        let bytes = self.src.as_bytes();
        // exactly four leading ASCII digits then '-'
        if bytes.len() < start + 5 {
            return false;
        }
        bytes[start..start + 4].iter().all(|b| b.is_ascii_digit()) && bytes[start + 4] == b'-'
    }

    fn looks_like_time(&self, start: usize) -> bool {
        // DD:DD:DD pattern.
        let bytes = self.src.as_bytes();
        if bytes.len() < start + 8 {
            return false;
        }
        let p = |i| bytes.get(start + i).copied();
        p(0).map_or(false, |b| b.is_ascii_digit())
            && p(1).map_or(false, |b| b.is_ascii_digit())
            && p(2) == Some(b':')
            && p(3).map_or(false, |b| b.is_ascii_digit())
            && p(4).map_or(false, |b| b.is_ascii_digit())
            && p(5) == Some(b':')
            && p(6).map_or(false, |b| b.is_ascii_digit())
            && p(7).map_or(false, |b| b.is_ascii_digit())
    }

    fn scan_date_or_datetime(&mut self, start: usize) -> Token {
        // DDDD-DD-DD has been verified by looks_like_date.
        let y = parse_u16(&self.src[self.pos..self.pos + 4]);
        self.pos += 4;
        self.pos += 1; // '-'
        let (m, m_ok) = parse_2_digit(&self.src[self.pos..]);
        self.pos += 2;
        if !m_ok || self.peek_byte() != Some(b'-') {
            self.error("invalid date literal", self.span_from(start));
        } else {
            self.pos += 1; // '-'
        }
        let (d, d_ok) = parse_2_digit(&self.src[self.pos..]);
        self.pos += 2;
        if !d_ok {
            self.error("invalid date literal", self.span_from(start));
        }

        // DateTime extension: `T HH:MM:SS [Z]`.
        if self.peek_byte() == Some(b'T') {
            self.pos += 1;
            let (hh, _) = parse_2_digit(&self.src[self.pos..]);
            self.pos += 2;
            if self.peek_byte() == Some(b':') {
                self.pos += 1;
            }
            let (mm, _) = parse_2_digit(&self.src[self.pos..]);
            self.pos += 2;
            if self.peek_byte() == Some(b':') {
                self.pos += 1;
            }
            let (ss, _) = parse_2_digit(&self.src[self.pos..]);
            self.pos += 2;
            let tz = if self.peek_byte() == Some(b'Z') {
                self.pos += 1;
                Tz::Utc
            } else {
                Tz::Local
            };
            return Token {
                kind: TokenKind::DateTime {
                    y,
                    m,
                    d,
                    hh,
                    mm,
                    ss,
                    tz,
                },
                span: self.span_from(start),
            };
        }

        Token {
            kind: TokenKind::Date { y, m, d },
            span: self.span_from(start),
        }
    }

    fn scan_time(&mut self, start: usize) -> Token {
        let (hh, _) = parse_2_digit(&self.src[self.pos..]);
        self.pos += 2;
        self.pos += 1; // ':'
        let (mm, _) = parse_2_digit(&self.src[self.pos..]);
        self.pos += 2;
        self.pos += 1; // ':'
        let (ss, _) = parse_2_digit(&self.src[self.pos..]);
        self.pos += 2;
        Token {
            kind: TokenKind::Time { hh, mm, ss },
            span: self.span_from(start),
        }
    }

    fn scan_money_with_symbol(&mut self, start: usize, symbol: char) -> Token {
        let symbol_currency = symbol_to_currency(symbol).unwrap();
        self.pos += symbol.len_utf8();
        let amount_start = self.pos;
        self.eat_decimal_digits();
        if self.peek_byte() == Some(b'.')
            && self
                .peek_byte_at(1)
                .map_or(false, |b| b.is_ascii_digit())
        {
            self.pos += 1;
            self.eat_decimal_digits();
        }
        let amount = self.src[amount_start..self.pos].to_string();

        // Optional redundant currency suffix (`₹150inr`). If present it must
        // either match the symbol or be empty.
        let suffix_start = self.pos;
        while let Some(b) = self.peek_byte() {
            if b.is_ascii_alphabetic() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let suffix = &self.src[suffix_start..self.pos];
        let currency = if suffix.is_empty() {
            symbol_currency.to_string()
        } else if suffix == symbol_currency {
            symbol_currency.to_string()
        } else if KNOWN_CURRENCIES.contains(&suffix) {
            self.error(
                format!(
                    "currency symbol `{symbol}` (= `{symbol_currency}`) conflicts with suffix `{suffix}`"
                ),
                self.span_from(start),
            );
            symbol_currency.to_string()
        } else {
            self.error(
                format!("unknown currency suffix `{suffix}`"),
                Span::in_file(suffix_start,  self.pos, self.file_id),
            );
            symbol_currency.to_string()
        };
        Token {
            kind: TokenKind::Money { amount, currency },
            span: self.span_from(start),
        }
    }

    // ---------------------------------------------------------------------
    // String / char / bytes / raw / multi-line
    // ---------------------------------------------------------------------

    fn scan_string(&mut self, start: usize, kind: StringKind) -> Token {
        // For `b"..."`, start currently points at the `b`; for `prompt"..."`
        // it points at the `p`. Both are followed by exactly one `"` at the
        // current position (we are called immediately after the `b`/`prompt`).
        if matches!(kind, StringKind::Bytes | StringKind::Prompt) {
            // skip the prefix that was already consumed by the caller — pos
            // is currently at the opening `"`.
        }
        debug_assert_eq!(self.peek_byte(), Some(b'"'));
        self.pos += 1;

        let mut parts: Vec<StringPart> = Vec::new();
        let mut buf = String::new();
        loop {
            let Some(b) = self.peek_byte() else {
                self.error("unterminated string literal", self.span_from(start));
                break;
            };
            match b {
                b'"' => {
                    self.pos += 1;
                    break;
                }
                b'\\' => {
                    self.pos += 1;
                    self.consume_escape_into(&mut buf, &kind);
                }
                b'{' if !matches!(kind, StringKind::Bytes) => {
                    // `{{` is a literal `{`.
                    if self.peek_byte_at(1) == Some(b'{') {
                        buf.push('{');
                        self.pos += 2;
                    } else {
                        // Flush buffered text.
                        if !buf.is_empty() {
                            parts.push(StringPart::Text(std::mem::take(&mut buf)));
                        }
                        self.pos += 1; // opening `{`
                        let interp_start = self.pos;
                        let mut depth = 1usize;
                        while depth > 0 {
                            match self.peek_byte() {
                                Some(b'{') => {
                                    depth += 1;
                                    self.pos += 1;
                                }
                                Some(b'}') => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                    self.pos += 1;
                                }
                                Some(_) => {
                                    let _ = self.bump_char();
                                }
                                None => {
                                    self.error(
                                        "unterminated interpolation in string",
                                        self.span_from(start),
                                    );
                                    break;
                                }
                            }
                        }
                        let interp_text = self.src[interp_start..self.pos].to_string();
                        let interp_span = Span::in_file(interp_start,  self.pos, self.file_id);
                        if self.peek_byte() == Some(b'}') {
                            self.pos += 1;
                        }
                        parts.push(StringPart::Interp {
                            text: interp_text,
                            span: interp_span,
                        });
                    }
                }
                b'}' if !matches!(kind, StringKind::Bytes) => {
                    if self.peek_byte_at(1) == Some(b'}') {
                        buf.push('}');
                        self.pos += 2;
                    } else {
                        self.error(
                            "unexpected `}` in string; use `}}` to write a literal `}`",
                            Span::in_file(self.pos,  self.pos + 1, self.file_id),
                        );
                        self.pos += 1;
                    }
                }
                b'\n' => {
                    self.error(
                        "string literal cannot contain a bare newline; use `\\n` or a multi-line string `\"\"\"...\"\"\"`",
                        Span::in_file(self.pos,  self.pos + 1, self.file_id),
                    );
                    self.pos += 1;
                }
                _ => {
                    if let Some(c) = self.bump_char() {
                        buf.push(c);
                    }
                }
            }
        }
        if !buf.is_empty() {
            parts.push(StringPart::Text(buf));
        }
        Token {
            kind: TokenKind::String { kind, parts },
            span: self.span_from(start),
        }
    }

    fn consume_escape_into(&mut self, buf: &mut String, kind: &StringKind) {
        let Some(b) = self.peek_byte() else {
            return;
        };
        let esc_start = self.pos - 1; // position of the backslash
        self.pos += 1;
        match b {
            b'n' => buf.push('\n'),
            b'r' => buf.push('\r'),
            b't' => buf.push('\t'),
            b'\\' => buf.push('\\'),
            b'\'' => buf.push('\''),
            b'"' => buf.push('"'),
            b'0' => buf.push('\0'),
            b'x' => {
                let mut hex = String::new();
                for _ in 0..2 {
                    if let Some(c) = self.peek_byte() {
                        if (c as char).is_ascii_hexdigit() {
                            hex.push(c as char);
                            self.pos += 1;
                        }
                    }
                }
                match u32::from_str_radix(&hex, 16) {
                    Ok(v) if v < 0x80 || matches!(kind, StringKind::Bytes) => {
                        buf.push(v as u8 as char);
                    }
                    Ok(_) => self.error(
                        "`\\x` escape may not exceed 0x7F outside byte strings; use `\\u{...}`",
                        Span::in_file(esc_start,  self.pos, self.file_id),
                    ),
                    Err(_) => {
                        self.error("invalid `\\x` escape", Span::in_file(esc_start,  self.pos, self.file_id))
                    }
                }
            }
            b'u' => {
                if self.peek_byte() != Some(b'{') {
                    self.error("expected `{` after `\\u`", Span::in_file(esc_start,  self.pos, self.file_id));
                    return;
                }
                self.pos += 1;
                let hex_start = self.pos;
                while let Some(b) = self.peek_byte() {
                    if (b as char).is_ascii_hexdigit() {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let hex = &self.src[hex_start..self.pos];
                if self.peek_byte() != Some(b'}') {
                    self.error("expected `}` to close `\\u{...}`", Span::in_file(esc_start,  self.pos, self.file_id));
                    return;
                }
                self.pos += 1;
                match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                    Some(c) => buf.push(c),
                    None => self.error(
                        "invalid Unicode scalar value",
                        Span::in_file(esc_start,  self.pos, self.file_id),
                    ),
                }
            }
            other => self.error(
                format!("unknown escape `\\{}`", other as char),
                Span::in_file(esc_start,  self.pos, self.file_id),
            ),
        }
    }

    fn scan_raw_string(&mut self, start: usize) -> Token {
        self.pos += 1; // `
        let text_start = self.pos;
        loop {
            match self.peek_byte() {
                Some(b'`') => {
                    let text = self.src[text_start..self.pos].to_string();
                    self.pos += 1;
                    return Token {
                        kind: TokenKind::String {
                            kind: StringKind::Raw,
                            parts: vec![StringPart::Text(text)],
                        },
                        span: self.span_from(start),
                    };
                }
                Some(_) => {
                    let _ = self.bump_char();
                }
                None => {
                    self.error("unterminated raw string", self.span_from(start));
                    return Token {
                        kind: TokenKind::String {
                            kind: StringKind::Raw,
                            parts: vec![StringPart::Text(
                                self.src[text_start..self.pos].to_string(),
                            )],
                        },
                        span: self.span_from(start),
                    };
                }
            }
        }
    }

    fn scan_multiline_string(&mut self, start: usize, kind: StringKind) -> Token {
        debug_assert!(self.starts_with("\"\"\""));
        self.pos += 3;
        let text_start = self.pos;
        loop {
            if self.starts_with("\"\"\"") {
                let raw = &self.src[text_start..self.pos];
                self.pos += 3;
                let dedented = dedent_multiline(raw);
                return Token {
                    kind: TokenKind::String {
                        kind,
                        parts: vec![StringPart::Text(dedented)],
                    },
                    span: self.span_from(start),
                };
            }
            if self.peek_byte().is_none() {
                self.error("unterminated multi-line string", self.span_from(start));
                let raw = &self.src[text_start..self.pos];
                return Token {
                    kind: TokenKind::String {
                        kind,
                        parts: vec![StringPart::Text(dedent_multiline(raw))],
                    },
                    span: self.span_from(start),
                };
            }
            let _ = self.bump_char();
        }
    }

    fn scan_char(&mut self, start: usize) -> Token {
        self.pos += 1; // '
        let mut buf = String::new();
        let kind = StringKind::Regular;
        if self.peek_byte() == Some(b'\\') {
            self.pos += 1;
            self.consume_escape_into(&mut buf, &kind);
        } else if let Some(c) = self.bump_char() {
            buf.push(c);
        }
        if self.peek_byte() == Some(b'\'') {
            self.pos += 1;
        } else {
            self.error("unterminated character literal", self.span_from(start));
        }
        let c = buf.chars().next().unwrap_or('\0');
        if buf.chars().count() != 1 {
            self.error(
                "character literal must contain exactly one scalar",
                self.span_from(start),
            );
        }
        Token {
            kind: TokenKind::Char(c),
            span: self.span_from(start),
        }
    }

    // ---------------------------------------------------------------------
    // `#` — hash literal or attribute open
    // ---------------------------------------------------------------------

    fn scan_hash(&mut self, start: usize) -> Token {
        debug_assert_eq!(self.peek_byte(), Some(b'#'));
        self.pos += 1;
        if self.peek_byte() == Some(b'[') {
            self.pos += 1;
            return Token {
                kind: TokenKind::HashLBracket,
                span: self.span_from(start),
            };
        }
        // `#algo:hex` — capture identifier-like algo, then `:`, then hex.
        let algo_start = self.pos;
        while let Some(b) = self.peek_byte() {
            if b.is_ascii_alphanumeric() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let algo = self.src[algo_start..self.pos].to_string();
        if self.peek_byte() != Some(b':') {
            self.error("expected `:` after hash algorithm name", self.span_from(start));
            return Token {
                kind: TokenKind::HashLit {
                    algo,
                    hex: String::new(),
                },
                span: self.span_from(start),
            };
        }
        self.pos += 1;
        let hex_start = self.pos;
        while let Some(b) = self.peek_byte() {
            if (b as char).is_ascii_hexdigit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let hex = self.src[hex_start..self.pos].to_string();
        if hex.is_empty() {
            self.error("hash literal has no hex payload", self.span_from(start));
        }
        Token {
            kind: TokenKind::HashLit { algo, hex },
            span: self.span_from(start),
        }
    }

    // ---------------------------------------------------------------------
    // `@` — address literal or stand-alone
    // ---------------------------------------------------------------------

    fn scan_at(&mut self, start: usize) -> Token {
        debug_assert_eq!(self.peek_byte(), Some(b'@'));
        self.pos += 1;
        // `@{expr}` — dynamic agent address. We keep this fused into one
        // token because the `{` here introduces an *expression*, not a block,
        // and we don't want the parser to misread it as a brace literal.
        if self.peek_byte() == Some(b'{') {
            self.pos += 1;
            let body_start = self.pos;
            let mut depth = 1;
            while depth > 0 {
                match self.peek_byte() {
                    Some(b'{') => {
                        depth += 1;
                        self.pos += 1;
                    }
                    Some(b'}') => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        self.pos += 1;
                    }
                    Some(_) => {
                        let _ = self.bump_char();
                    }
                    None => {
                        self.error(
                            "unterminated `@{...}` address literal",
                            self.span_from(start),
                        );
                        break;
                    }
                }
            }
            let text = self.src[body_start..self.pos].to_string();
            if self.peek_byte() == Some(b'}') {
                self.pos += 1;
            }
            return Token {
                kind: TokenKind::AddrLit {
                    kind: AddrLitKind::Dynamic,
                    text,
                },
                span: self.span_from(start),
            };
        }
        // `@ident` — could be an attribute (`@retry(...)`), a refinement
        // (`@range(0, 200)`), or a static agent address (`@alice`). The
        // disambiguation is contextual, so we always emit a bare `At` here
        // and let the parser decide based on the surrounding production.
        Token {
            kind: TokenKind::At,
            span: self.span_from(start),
        }
    }

    // ---------------------------------------------------------------------
    // Punctuation / operators
    // ---------------------------------------------------------------------

    fn scan_punct(&mut self, start: usize) -> Token {
        use TokenKind::*;
        let b = self.peek_byte().unwrap();
        self.pos += 1;
        let kind = match b {
            b'(' => LParen,
            b')' => RParen,
            b'{' => LBrace,
            b'}' => RBrace,
            b'[' => LBracket,
            b']' => RBracket,
            b',' => Comma,
            b';' => Semi,
            b':' => Colon,
            b'\\' => Backslash,
            b'~' => Tilde,
            b'^' => Caret,
            b'?' => Question,
            b'.' => {
                if self.bump_if(b'.') {
                    if self.bump_if(b'=') {
                        DotDotEq
                    } else {
                        DotDot
                    }
                } else {
                    Dot
                }
            }
            b'-' => {
                if self.bump_if(b'>') {
                    Arrow
                } else if self.bump_if(b'=') {
                    MinusEq
                } else {
                    Minus
                }
            }
            b'+' => {
                if self.bump_if(b'=') {
                    PlusEq
                } else {
                    Plus
                }
            }
            b'*' => {
                if self.bump_if(b'=') {
                    StarEq
                } else {
                    Star
                }
            }
            b'/' => {
                if self.bump_if(b'=') {
                    SlashEq
                } else {
                    Slash
                }
            }
            b'%' => {
                if self.bump_if(b'=') {
                    PercentEq
                } else {
                    Percent
                }
            }
            b'=' => {
                if self.bump_if(b'=') {
                    EqEq
                } else if self.bump_if(b'>') {
                    FatArrow
                } else {
                    Eq
                }
            }
            b'!' => {
                if self.bump_if(b'=') {
                    NotEq
                } else {
                    Bang
                }
            }
            b'<' => {
                if self.bump_if(b'=') {
                    LtEq
                } else if self.bump_if(b'<') {
                    Shl
                } else {
                    Lt
                }
            }
            b'>' => {
                if self.bump_if(b'=') {
                    GtEq
                } else if self.bump_if(b'>') {
                    Shr
                } else {
                    Gt
                }
            }
            b'&' => {
                if self.bump_if(b'&') {
                    AmpAmp
                } else {
                    Amp
                }
            }
            b'|' => {
                if self.bump_if(b'|') {
                    PipePipe
                } else if self.bump_if(b'>') {
                    Pipeline
                } else {
                    Pipe
                }
            }
            other => {
                let span = Span::in_file(start,  self.pos, self.file_id);
                self.error(
                    format!("unexpected character `{}`", other as char),
                    span,
                );
                // Skip and try to recover by emitting a Bang as a placeholder.
                Bang
            }
        };
        Token {
            kind,
            span: self.span_from(start),
        }
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

fn is_ident_start(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphabetic()
}

fn is_ident_start_char(c: char) -> bool {
    c == '_' || unicode_ident::is_xid_start(c)
}

fn is_ident_continue_char(c: char) -> bool {
    c == '_' || unicode_ident::is_xid_continue(c)
}

fn parse_u16(s: &str) -> u16 {
    s.parse().unwrap_or(0)
}

fn parse_2_digit(s: &str) -> (u8, bool) {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_digit() && bytes[1].is_ascii_digit() {
        (((bytes[0] - b'0') * 10 + (bytes[1] - b'0')) as u8, true)
    } else {
        (0, false)
    }
}

/// Strip the common leading whitespace from every line of a triple-quoted
/// string and remove the leading/trailing blank line, matching the spec's
/// "dedented to the closing fence" rule.
fn dedent_multiline(raw: &str) -> String {
    let mut lines: Vec<&str> = raw.lines().collect();
    // Trim a leading empty line.
    if lines.first().map_or(false, |l| l.trim().is_empty()) {
        lines.remove(0);
    }
    // Compute the minimum leading-whitespace width across non-blank lines.
    let mut min_indent = usize::MAX;
    for line in lines.iter() {
        if line.trim().is_empty() {
            continue;
        }
        let n = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        if n < min_indent {
            min_indent = n;
        }
    }
    if min_indent == usize::MAX {
        min_indent = 0;
    }
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = if line.len() >= min_indent {
            &line[min_indent..]
        } else {
            line
        };
        if i > 0 {
            out.push('\n');
        }
        out.push_str(trimmed);
    }
    out
}
