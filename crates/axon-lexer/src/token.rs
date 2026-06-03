//! Token kinds produced by the Axon lexer.
//!
//! Token shapes track §9 ("Lexical structure") and the terminals of §43
//! ("Formal grammar"). Numeric/temporal/money literals carry their parsed
//! payloads — getting these wrong (floating-point money, naïve durations)
//! is the classic production defect Axon's lexer is explicit about avoiding.

use axon_diag::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    // ---- Trivia ----------------------------------------------------------
    /// A logical newline. Per §9.2 newlines are statement terminators except
    /// when the line ends with an unclosed bracket, a binary operator,
    /// `|>`, or a `\` continuation. The lexer always emits these; the parser
    /// applies the bracket-tolerance rule.
    Newline,
    LineComment,
    BlockComment,
    DocComment(String),
    ModDocComment(String),

    // ---- Names & keywords ------------------------------------------------
    Ident(String),
    Keyword(Keyword),

    // ---- Numeric & temporal literals ------------------------------------
    Int {
        value: i128,
        base: NumBase,
    },
    Float {
        lexeme: String,
    },
    /// `1.99dec`, `0.1dec` — arbitrary-precision decimal, stored as the
    /// significand string with the decimal point position implied.
    Decimal {
        lexeme: String,
    },
    /// `30s`, `5m`, `2h`, `1d`, `500ms`. Stored normalized to nanoseconds.
    Duration {
        nanos: i128,
        original: String,
    },
    /// `1.50usd`, `€2.00`, `₹150inr`. Amount stored as the original lexeme
    /// of the digits/decimal so we never round in the lexer.
    Money {
        amount: String,
        currency: String,
    },
    Date {
        y: u16,
        m: u8,
        d: u8,
    },
    DateTime {
        y: u16,
        m: u8,
        d: u8,
        hh: u8,
        mm: u8,
        ss: u8,
        tz: Tz,
    },
    Time {
        hh: u8,
        mm: u8,
        ss: u8,
    },
    /// `#sha256:ab12...`
    HashLit {
        algo: String,
        hex: String,
    },
    /// `@alice` or `@{dyn}` (we capture the literal text inside the braces).
    AddrLit {
        kind: AddrLitKind,
        text: String,
    },

    // ---- Other literals --------------------------------------------------
    Bool(bool),
    Char(char),
    /// A string-shaped literal. The payload distinguishes regular, raw,
    /// multi-line, bytes, and prompt forms; interpolation segments live in
    /// `parts` and are stored as raw source text for the parser to re-parse.
    String {
        kind: StringKind,
        parts: Vec<StringPart>,
    },

    // ---- Punctuation -----------------------------------------------------
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Dot,
    DotDot,
    DotDotEq,
    Question,
    /// `??` — null-coalescing operator.
    QuestionQuestion,
    /// `?.` — nil-safe field/method access.
    QuestionDot,
    Bang,
    /// `@` standalone (e.g. before an attribute name like `@retry`). Note
    /// that `@alice` and `@{...}` are recognized as `AddrLit` instead.
    At,
    /// `#[` — the start of an outer attribute. `#sha256:...` is `HashLit`.
    HashLBracket,
    Arrow,    // ->
    FatArrow, // =>
    Pipeline, // |>
    Backslash,

    // ---- Operators -------------------------------------------------------
    Eq,
    EqEq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    AmpAmp,
    Pipe,
    PipePipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,

    Eof,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum NumBase {
    Dec,
    Hex,
    Oct,
    Bin,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tz {
    Utc,
    Local,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AddrLitKind {
    /// `@alice` — static handle (an identifier follows the `@`).
    Static,
    /// `@{expr}` — dynamic handle. The captured `text` is the raw expression.
    Dynamic,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum StringKind {
    /// `"..."`
    Regular,
    /// `b"..."`
    Bytes,
    /// `` `...` ``
    Raw,
    /// `"""..."""` — dedented to the closing fence.
    MultiLine,
    /// `prompt"""..."""`
    Prompt,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StringPart {
    /// Already-unescaped literal text.
    Text(String),
    /// `{expr}` — raw source text of the interpolated expression. The parser
    /// re-parses this through the same lexer at parse time. The span points
    /// at the inside of the braces.
    Interp { text: String, span: Span },
}

// ---------------------------------------------------------------------------
// Keywords (§9.3)
// ---------------------------------------------------------------------------

macro_rules! keywords {
    ($($variant:ident => $kw:literal,)*) => {
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
        pub enum Keyword {
            $($variant,)*
        }

        impl Keyword {
            pub fn from_str(s: &str) -> Option<Keyword> {
                match s {
                    $($kw => Some(Keyword::$variant),)*
                    _ => None,
                }
            }

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Keyword::$variant => $kw,)*
                }
            }
        }
    };
}

keywords! {
    Agent => "agent",
    Actor => "actor",
    And => "and",
    As => "as",
    Ask => "ask",
    Async => "async",
    Await => "await",
    Break => "break",
    Case => "case",
    Chan => "chan",
    Const => "const",
    Continue => "continue",
    Defer => "defer",
    Do => "do",
    Effect => "effect",
    Else => "else",
    Enum => "enum",
    Export => "export",
    Extern => "extern",
    Fn => "fn",
    For => "for",
    Gen => "gen",
    Generate => "generate",
    Graph => "graph",
    If => "if",
    Impl => "impl",
    Import => "import",
    In => "in",
    Is => "is",
    Let => "let",
    Match => "match",
    Memory => "memory",
    Model => "model",
    Mut => "mut",
    Nil => "nil",
    Not => "not",
    On => "on",
    Or => "or",
    Parallel => "parallel",
    Plan => "plan",
    Prompt => "prompt",
    Pub => "pub",
    Recover => "recover",
    Replay => "replay",
    Return => "return",
    Schema => "schema",
    Select => "select",
    SelfKw => "self",
    Spawn => "spawn",
    State => "state",
    Stream => "stream",
    Struct => "struct",
    Supervisor => "supervisor",
    Trait => "trait",
    Try => "try",
    Tool => "tool",
    Type => "type",
    Use => "use",
    Uses => "uses",
    Var => "var",
    When => "when",
    While => "while",
    With => "with",
    Yield => "yield",
}

/// Recognized currency codes for money literals. ISO-4217 subset; not exhaustive
/// — the lexer rejects unrecognized 3-letter suffixes so that typos like `1.50usf`
/// fail at lex time rather than silently becoming a Money value with a junk code.
pub const KNOWN_CURRENCIES: &[&str] = &[
    "usd", "eur", "gbp", "jpy", "inr", "cad", "aud", "chf", "cny", "krw", "sek",
    "nok", "dkk", "nzd", "sgd", "hkd", "mxn", "brl", "zar", "rub", "try", "aed",
];

/// Currency *symbols* that may appear before a numeric literal.
pub fn symbol_to_currency(c: char) -> Option<&'static str> {
    match c {
        '$' => Some("usd"),
        '€' => Some("eur"),
        '£' => Some("gbp"),
        '¥' => Some("jpy"),
        '₹' => Some("inr"),
        _ => None,
    }
}
