//! The Axon parser proper.
//!
//! Design notes:
//!
//! * **Newline handling.** Axon is newline-significant (§9.2). The parser
//!   tracks bracket depth: inside `(`, `[`, or `{` the lexer's `Newline`
//!   tokens are skipped as whitespace. At the top of a block they act as
//!   statement separators unless the previous token ends with something that
//!   implies continuation (an open bracket, a binary operator, `|>`, or `\`).
//!
//! * **Brace-literal vs block.** A bare `{` after an expression-introducing
//!   keyword (`if`, `match`, `for`, `while`, function/agent/etc. bodies)
//!   opens a [`Block`]. A `{` met *as an atom in expression position* opens
//!   a brace literal (set / map / record).
//!
//! * **Error recovery.** Errors are accumulated in a `Vec<Diagnostic>`; the
//!   parser tries to resync to the next statement / item boundary using a
//!   simple skip-until-`}` or skip-until-newline heuristic. The AST may end
//!   up with placeholder nodes (e.g. `Pattern::Wildcard`) so downstream
//!   passes can still walk it.

use axon_ast::*;
use axon_diag::{Diagnostic, SourceFile, Span};
use axon_lexer::{
    self as lex, AddrLitKind, Keyword as Kw, StringKind as LexStringKind, StringPart as LexStringPart,
    Token, TokenKind, Tz,
};

/// Parse a source file. Returns the AST and any diagnostics; the AST is
/// always returned (potentially with placeholder nodes) so tools can still
/// operate on partial input.
pub fn parse(source: &SourceFile) -> (Program, Vec<Diagnostic>) {
    let (tokens, mut diags) = lex::tokenize(source);
    let mut p = Parser::new(tokens, source.text(), source.id());
    let program = p.parse_program();
    diags.extend(p.diagnostics);
    (program, diags)
}

// ===========================================================================
// Parser state
// ===========================================================================

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    src: &'a str,
    file_id: u16,
    /// Depth of `(` and `[`. Inside these, newlines are non-significant.
    paren_depth: usize,
    diagnostics: Vec<Diagnostic>,
    /// `pos` at the last call to `error()`. Used to suppress cascade
    /// errors: when the parser keeps reporting from the same position
    /// without advancing, only the first message is kept.
    last_error_pos: Option<usize>,
}

impl<'a> Parser<'a> {
    fn new(tokens: Vec<Token>, src: &'a str, file_id: u16) -> Self {
        Self {
            tokens,
            pos: 0,
            src,
            file_id,
            paren_depth: 0,
            diagnostics: Vec::new(),
            last_error_pos: None,
        }
    }

    fn mk_span(&self, start: usize, end: usize) -> Span {
        Span::in_file(start, end, self.file_id)
    }

    // ---- Token-stream primitives ----------------------------------------

    fn peek(&self) -> &TokenKind {
        // Skip trivia: comments and (when inside brackets) newlines.
        let mut i = self.pos;
        loop {
            match self.tokens.get(i).map(|t| &t.kind) {
                Some(TokenKind::LineComment)
                | Some(TokenKind::BlockComment)
                | Some(TokenKind::DocComment(_))
                | Some(TokenKind::ModDocComment(_)) => i += 1,
                Some(TokenKind::Newline) if self.paren_depth > 0 => i += 1,
                Some(k) => return k,
                None => return &TokenKind::Eof,
            }
        }
    }

    /// Peek including newlines (for statement-terminator decisions).
    fn peek_raw(&self) -> &TokenKind {
        let mut i = self.pos;
        loop {
            match self.tokens.get(i).map(|t| &t.kind) {
                Some(TokenKind::LineComment)
                | Some(TokenKind::BlockComment)
                | Some(TokenKind::DocComment(_))
                | Some(TokenKind::ModDocComment(_)) => i += 1,
                Some(k) => return k,
                None => return &TokenKind::Eof,
            }
        }
    }

    fn peek_span(&self) -> Span {
        let mut i = self.pos;
        loop {
            match self.tokens.get(i).map(|t| &t.kind) {
                Some(TokenKind::LineComment)
                | Some(TokenKind::BlockComment)
                | Some(TokenKind::DocComment(_))
                | Some(TokenKind::ModDocComment(_)) => i += 1,
                Some(TokenKind::Newline) if self.paren_depth > 0 => i += 1,
                Some(_) => return self.tokens[i].span,
                None => return Span::DUMMY,
            }
        }
    }

    fn bump(&mut self) -> Token {
        // Advance past trivia, then take one token. Updates bracket depth
        // so the next `peek` can ignore newlines correctly.
        loop {
            match self.tokens.get(self.pos).map(|t| &t.kind) {
                Some(TokenKind::LineComment)
                | Some(TokenKind::BlockComment)
                | Some(TokenKind::DocComment(_))
                | Some(TokenKind::ModDocComment(_)) => self.pos += 1,
                Some(TokenKind::Newline) if self.paren_depth > 0 => self.pos += 1,
                _ => break,
            }
        }
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token {
            kind: TokenKind::Eof,
            span: Span::DUMMY,
        });
        self.pos += 1;
        match &tok.kind {
            TokenKind::LParen | TokenKind::LBracket => self.paren_depth += 1,
            TokenKind::RParen | TokenKind::RBracket => {
                if self.paren_depth > 0 {
                    self.paren_depth -= 1;
                }
            }
            _ => {}
        }
        tok
    }

    fn eat_kw(&mut self, kw: Kw) -> bool {
        if matches!(self.peek(), TokenKind::Keyword(k) if *k == kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_kw(&mut self, kw: Kw) -> bool {
        if self.eat_kw(kw) {
            true
        } else {
            let span = self.peek_span();
            self.error(format!("expected keyword `{}`", kw.as_str()), span);
            false
        }
    }

    fn expect(&mut self, k: &TokenKind, what: &str) -> Option<Span> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(k) {
            Some(self.bump().span)
        } else {
            let span = self.peek_span();
            self.error(format!("expected {what}"), span);
            None
        }
    }

    /// Consume one or more `Newline`s in statement-separator position.
    fn eat_newlines(&mut self) {
        while matches!(self.tokens.get(self.pos).map(|t| &t.kind), Some(TokenKind::Newline))
            || matches!(
                self.tokens.get(self.pos).map(|t| &t.kind),
                Some(TokenKind::LineComment)
                    | Some(TokenKind::BlockComment)
                    | Some(TokenKind::DocComment(_))
                    | Some(TokenKind::ModDocComment(_))
            )
        {
            self.pos += 1;
        }
    }

    /// Record a parse error. Suppresses cascade duplicates two ways:
    ///
    ///   * Exact-message + exact-span repeats are dropped (same as
    ///     before).
    ///   * Any error that fires while the parser is *still at the
    ///     same token position* as the previous error is dropped —
    ///     this is the classic "one bad token, ten cascade lines"
    ///     pattern. The first error fires; the rest are silenced
    ///     until the parser advances at least one token.
    fn error(&mut self, message: impl Into<String>, span: Span) {
        let msg = message.into();
        if let Some(last) = self.diagnostics.last() {
            if last.primary.span == span && last.message == msg {
                return;
            }
        }
        if let Some(last_pos) = self.last_error_pos {
            if last_pos == self.pos {
                return;
            }
        }
        self.last_error_pos = Some(self.pos);
        self.diagnostics.push(Diagnostic::error(msg, span));
    }

    /// Whole-program error recovery: advance to the start of the next
    /// top-level item. We scan for one of the keyword starts (`fn`,
    /// `agent`, `tool`, …) or contextual idents (`test`, `eval`,
    /// `policy`). Brace depth is deliberately *not* tracked — when
    /// the body that broke had unbalanced braces, depth-tracking would
    /// walk to EOF; the rule "any subsequent item-start keyword wins"
    /// is the right recovery heuristic for real code.
    fn recover_to_next_item(&mut self) {
        // Advance at least one token so we don't immediately re-match
        // the same broken position on the next loop iteration.
        if !self.at_eof() {
            self.bump();
        }
        while !self.at_eof() && !self.at_item_start_token() {
            self.bump();
        }
    }

    fn at_item_start_token(&self) -> bool {
        match self.peek() {
            TokenKind::Keyword(k) => matches!(
                k,
                Kw::Use
                    | Kw::Fn
                    | Kw::Type
                    | Kw::Schema
                    | Kw::Agent
                    | Kw::Actor
                    | Kw::Supervisor
                    | Kw::Graph
                    | Kw::Model
                    | Kw::Tool
                    | Kw::Memory
                    | Kw::Prompt
                    | Kw::Const
                    | Kw::Trait
                    | Kw::Impl
                    | Kw::Pub
            ),
            TokenKind::Ident(s) => matches!(s.as_str(), "test" | "eval" | "policy"),
            _ => false,
        }
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    /// Stage 37 — contextual keywords. The five reserved item-starter
    /// keywords `agent`/`tool`/`model`/`memory`/`prompt` may also appear
    /// in identifier positions (variable bindings, expression atoms,
    /// record field labels). This predicate is the gate: in places where
    /// either an `Ident` or one of these soft keywords is acceptable as a
    /// name, the parser consults this and treats the soft keyword as the
    /// corresponding identifier text.
    ///
    /// Why these five specifically: every one of them shadows a high-
    /// frequency author vocabulary word (`let prompt = ...`, `let model = ...`).
    /// The Stage 36 DX survey ranked this as PAPERCUT P2 — the single
    /// highest-frequency new-author error.
    ///
    /// Why NOT every keyword: we want `let if = 1` to stay an error. The
    /// boundary is "keywords that name domain concepts the user already
    /// has variables for" vs. "control-flow / type-system reserved words."
    fn is_soft_keyword(kw: axon_lexer::Keyword) -> bool {
        use axon_lexer::Keyword as Kw;
        matches!(
            kw,
            Kw::Agent | Kw::Tool | Kw::Model | Kw::Memory | Kw::Prompt
        )
    }

    // ---- Programs & items ----------------------------------------------

    fn parse_program(&mut self) -> Program {
        let start = self.peek_span();
        let mut items = Vec::new();
        self.eat_newlines();
        while !self.at_eof() {
            let item_start = self.pos;
            let diag_count_before = self.diagnostics.len();
            match self.parse_item() {
                Some(item) => {
                    // The item parsed (possibly with internal errors).
                    // If new diagnostics fired, resync to the next
                    // item boundary so a half-consumed body doesn't
                    // cascade into the next item's parse.
                    if self.diagnostics.len() > diag_count_before {
                        self.recover_to_next_item();
                    }
                    items.push(item);
                }
                None => {
                    // Couldn't recognize an item header.
                    if self.pos == item_start {
                        let span = self.peek_span();
                        self.error("expected a top-level item", span);
                        if !self.at_eof() {
                            self.bump();
                        }
                    }
                    // After failing to parse one, walk forward to the
                    // next plausible item start so we don't crawl
                    // token-by-token through a syntactically broken
                    // region.
                    self.recover_to_next_item();
                }
            }
            self.eat_newlines();
        }
        Program {
            items,
            span: Span::in_file(start.start as usize, self.peek_span().end as usize, self.file_id),
        }
    }

    fn parse_item(&mut self) -> Option<Item> {
        let attrs = self.parse_attributes();
        let vis = if self.eat_kw(Kw::Pub) {
            Visibility::Public
        } else {
            Visibility::Private
        };
        let is_async = self.eat_kw(Kw::Async);
        match self.peek().clone() {
            TokenKind::Keyword(Kw::Use) => Some(Item::Use(self.parse_use_decl())),
            TokenKind::Keyword(Kw::Fn) => Some(Item::Fn(self.parse_fn_decl(vis, attrs, is_async))),
            TokenKind::Keyword(Kw::Type) => Some(Item::Type(self.parse_type_decl(vis))),
            TokenKind::Keyword(Kw::Schema) => Some(Item::Schema(self.parse_schema_decl(vis))),
            TokenKind::Keyword(Kw::Agent) => Some(Item::Agent(self.parse_agent_decl())),
            TokenKind::Keyword(Kw::Actor) => Some(Item::Actor(self.parse_actor_decl())),
            TokenKind::Keyword(Kw::Supervisor) => {
                Some(Item::Supervisor(self.parse_supervisor_decl()))
            }
            TokenKind::Keyword(Kw::Graph) => Some(Item::Graph(self.parse_graph_decl())),
            TokenKind::Keyword(Kw::Model) => Some(Item::Model(self.parse_model_decl())),
            TokenKind::Keyword(Kw::Tool) => Some(Item::Tool(self.parse_tool_decl())),
            TokenKind::Keyword(Kw::Memory) => Some(Item::Memory(self.parse_memory_decl())),
            TokenKind::Keyword(Kw::Prompt) => Some(Item::Prompt(self.parse_prompt_decl())),
            TokenKind::Keyword(Kw::Const) => Some(Item::Const(self.parse_const_decl(vis))),
            TokenKind::Keyword(Kw::Trait) => Some(Item::Trait(self.parse_trait_decl(vis))),
            TokenKind::Keyword(Kw::Impl) => Some(Item::Impl(self.parse_impl_block())),
            // `test "..." { ... }` and `eval "..." { ... }` use contextual
            // identifiers (not keywords per §9.3) so they don't get reserved
            // as a name in user code.
            TokenKind::Ident(s) if s == "test" => Some(Item::Test(self.parse_test_decl(false))),
            TokenKind::Ident(s) if s == "eval" => Some(Item::Eval(self.parse_eval_decl())),
            TokenKind::Ident(s) if s == "policy" => {
                Some(Item::Policy(self.parse_policy_decl()))
            }
            _ => None,
        }
    }

    fn parse_test_decl(&mut self, _is_eval: bool) -> axon_ast::TestDecl {
        let start = self.peek_span();
        self.bump(); // consume `test`
        // The test's label is a string literal — either single-line or
        // multi-line. We pull the text out and use it as the test name.
        let name = self.expect_string_literal();
        let body = self.parse_block();
        axon_ast::TestDecl {
            name,
            body,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_eval_decl(&mut self) -> axon_ast::EvalDecl {
        let start = self.peek_span();
        self.bump(); // consume `eval`
        let name = self.expect_string_literal();
        let body = self.parse_block();
        axon_ast::EvalDecl {
            name,
            body,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    /// `policy NAME { allow tool …; deny net …; budget …; rate …; audit … }`
    /// (§30). Parsed into structured [`PolicyClause`]s; the runtime
    /// compiles them into an enforceable policy block.
    fn parse_policy_decl(&mut self) -> axon_ast::PolicyDecl {
        use axon_ast::{PolicyAction, PolicyClause};
        let start = self.peek_span();
        self.bump(); // consume `policy`
        let name = self.parse_ident();
        self.expect(&TokenKind::LBrace, "`{` to start policy body");
        self.eat_newlines();
        let mut clauses = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            // Each clause is keyword-led by a contextual identifier.
            let lead = match self.peek().clone() {
                TokenKind::Ident(s) => s,
                _ => {
                    let sp = self.peek_span();
                    self.error("expected a policy clause (allow/deny/budget/rate/audit)", sp);
                    // Skip to next line to recover.
                    let _ = self.consume_raw_until_newline();
                    self.eat_newlines();
                    continue;
                }
            };
            match lead.as_str() {
                "allow" | "deny" => {
                    self.bump(); // action
                    let action = if lead == "allow" {
                        PolicyAction::Allow
                    } else {
                        PolicyAction::Deny
                    };
                    let effect = self.parse_ident().name;
                    let mut patterns = vec![self.parse_policy_pattern()];
                    while matches!(self.peek(), TokenKind::Comma) {
                        self.bump();
                        patterns.push(self.parse_policy_pattern());
                    }
                    // `when` is a reserved keyword (Kw::When), not an ident.
                    let when = if self.eat_kw(Kw::When) {
                        Some(self.consume_raw_until_newline())
                    } else {
                        None
                    };
                    clauses.push(PolicyClause::Rule {
                        action,
                        effect,
                        patterns,
                        when,
                    });
                }
                "budget" => {
                    self.bump();
                    let scope = self.parse_ident().name;
                    let (usd_cents, tokens) = self.parse_policy_budget_body();
                    clauses.push(PolicyClause::Budget {
                        scope,
                        usd_cents,
                        tokens,
                    });
                }
                "rate" => {
                    self.bump();
                    let scope = self.parse_ident().name;
                    let (max_calls, window_secs) = self.parse_policy_rate_body();
                    clauses.push(PolicyClause::Rate {
                        scope,
                        max_calls,
                        window_secs,
                    });
                }
                "audit" => {
                    self.bump();
                    let mut kinds = vec![self.parse_ident().name];
                    while matches!(self.peek(), TokenKind::Comma) {
                        self.bump();
                        kinds.push(self.parse_ident().name);
                    }
                    clauses.push(PolicyClause::Audit(kinds));
                }
                other => {
                    let sp = self.peek_span();
                    self.error(
                        &format!("unknown policy clause `{other}` (expected allow/deny/budget/rate/audit)"),
                        sp,
                    );
                    let _ = self.consume_raw_until_newline();
                }
            }
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close policy");
        axon_ast::PolicyDecl {
            name,
            clauses,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    /// A policy target pattern: a dotted ident (`kb.search`), a string
    /// glob (`"kb.internal"`, `"*"`), or a bare `*`.
    fn parse_policy_pattern(&mut self) -> String {
        match self.peek().clone() {
            TokenKind::Ident(_) => {
                let path = self.parse_path();
                path.segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".")
            }
            TokenKind::Star => {
                self.bump();
                "*".to_string()
            }
            _ => self.expect_string_literal(),
        }
    }

    /// `{ usd = 0.50, tokens = 60_000 }` → (usd_cents, tokens).
    fn parse_policy_budget_body(&mut self) -> (Option<i64>, Option<i64>) {
        let mut usd_cents = None;
        let mut tokens = None;
        self.expect(&TokenKind::LBrace, "`{` to start budget body");
        self.eat_newlines();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            let key = self.parse_ident().name;
            self.expect(&TokenKind::Eq, "`=` in budget entry");
            match key.as_str() {
                "usd" => usd_cents = Some(self.parse_money_or_number_as_cents()),
                "tokens" => tokens = Some(self.parse_int_value()),
                _ => {
                    // Unknown budget key — skip its value.
                    let _ = self.consume_raw_until_newline();
                }
            }
            if matches!(self.peek(), TokenKind::Comma) {
                self.bump();
            }
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close budget body");
        (usd_cents, tokens)
    }

    /// `{ 30 per 1m }` → (max_calls, window_secs).
    fn parse_policy_rate_body(&mut self) -> (u32, u64) {
        self.expect(&TokenKind::LBrace, "`{` to start rate body");
        self.eat_newlines();
        let max_calls = self.parse_int_value().max(0) as u32;
        let _ = self.bump_if_ident("per");
        let window_secs = match self.peek().clone() {
            TokenKind::Duration { nanos, .. } => {
                self.bump();
                (nanos / 1_000_000_000) as u64
            }
            _ => {
                let v = self.parse_int_value().max(0) as u64;
                v
            }
        };
        self.eat_newlines();
        self.expect(&TokenKind::RBrace, "`}` to close rate body");
        (max_calls, window_secs.max(1))
    }

    fn parse_int_value(&mut self) -> i64 {
        match self.peek().clone() {
            TokenKind::Int { value, .. } => {
                self.bump();
                value as i64
            }
            _ => {
                let sp = self.peek_span();
                self.error("expected an integer", sp);
                0
            }
        }
    }

    /// Read a money literal (`0.50usd`) or a plain number and return the
    /// value in cents.
    fn parse_money_or_number_as_cents(&mut self) -> i64 {
        match self.peek().clone() {
            TokenKind::Money { amount, .. } => {
                self.bump();
                money_str_to_cents(&amount)
            }
            TokenKind::Float { lexeme } => {
                self.bump();
                let f: f64 = lexeme.parse().unwrap_or(0.0);
                (f * 100.0).round() as i64
            }
            TokenKind::Int { value, .. } => {
                self.bump();
                (value as i64) * 100
            }
            _ => {
                let sp = self.peek_span();
                self.error("expected a money or numeric budget value", sp);
                0
            }
        }
    }

    fn parse_attributes(&mut self) -> Vec<Attribute> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                TokenKind::At => {
                    let start = self.peek_span();
                    self.bump();
                    let name = self.parse_path();
                    let args = if matches!(self.peek(), TokenKind::LParen) {
                        self.parse_paren_arg_exprs()
                    } else {
                        Vec::new()
                    };
                    out.push(Attribute {
                        style: AttrStyle::At,
                        name,
                        args,
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    });
                    self.eat_newlines();
                }
                TokenKind::HashLBracket => {
                    let start = self.peek_span();
                    self.bump();
                    let name = self.parse_path();
                    let args = if matches!(self.peek(), TokenKind::LParen) {
                        self.parse_paren_arg_exprs()
                    } else {
                        Vec::new()
                    };
                    self.expect(&TokenKind::RBracket, "`]` to close attribute");
                    out.push(Attribute {
                        style: AttrStyle::Outer,
                        name,
                        args,
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    });
                    self.eat_newlines();
                }
                _ => break,
            }
        }
        out
    }

    fn prev_end(&self) -> usize {
        let mut i = self.pos;
        while i > 0 {
            i -= 1;
            if !matches!(
                self.tokens.get(i).map(|t| &t.kind),
                Some(TokenKind::Newline)
                    | Some(TokenKind::LineComment)
                    | Some(TokenKind::BlockComment)
                    | Some(TokenKind::DocComment(_))
                    | Some(TokenKind::ModDocComment(_))
            ) {
                return self.tokens[i].span.end as usize;
            }
        }
        0
    }

    // ---- Common parsers -------------------------------------------------

    /// Parse a name in a *binding* position.
    ///
    /// The spec lists many domain keywords (`model`, `memory`, `ask`, `plan`,
    /// `tool`, `prompt`, etc.). The README routinely uses these as parameter
    /// names, message-handler names, and field names — they only function as
    /// keywords in item-prefix or expression-start position. In binding
    /// positions we accept either an `Ident` token or any `Keyword`, the
    /// latter rewritten to the keyword's lexeme.
    fn parse_ident(&mut self) -> Ident {
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                let span = self.bump().span;
                Ident { name, span }
            }
            TokenKind::Keyword(kw) => {
                let span = self.bump().span;
                Ident {
                    name: kw.as_str().to_string(),
                    span,
                }
            }
            other => {
                let span = self.peek_span();
                self.error(format!("expected a name, got {other:?}"), span);
                Ident {
                    name: String::new(),
                    span,
                }
            }
        }
    }

    fn parse_ident_or_kw(&mut self) -> Ident {
        self.parse_ident()
    }

    fn parse_path(&mut self) -> Path {
        let first = self.parse_ident();
        let start = first.span;
        let mut segments = vec![first];
        while matches!(self.peek(), TokenKind::Dot) {
            // Path continuation: keep going as long as a name token (ident
            // or keyword-as-name) follows the dot. Bail otherwise so the
            // postfix `.` machinery can claim it.
            let save = self.pos;
            self.bump();
            if matches!(self.peek(), TokenKind::Ident(_) | TokenKind::Keyword(_)) {
                segments.push(self.parse_ident());
            } else {
                self.pos = save;
                break;
            }
        }
        let end = segments.last().unwrap().span.end as usize;
        Path {
            segments,
            span: Span::in_file(start.start as usize, end, self.file_id),
        }
    }

    fn parse_use_decl(&mut self) -> UseDecl {
        let start = self.peek_span();
        self.bump(); // `use`
        let path = self.parse_path();
        let items = if matches!(self.peek(), TokenKind::Dot) {
            // peek ahead for `.{`
            let save = self.pos;
            self.bump();
            if matches!(self.peek(), TokenKind::LBrace) {
                self.bump();
                let mut idents = Vec::new();
                if !matches!(self.peek(), TokenKind::RBrace) {
                    idents.push(self.parse_ident());
                    while matches!(self.peek(), TokenKind::Comma) {
                        self.bump();
                        if matches!(self.peek(), TokenKind::RBrace) {
                            break;
                        }
                        idents.push(self.parse_ident());
                    }
                }
                self.expect(&TokenKind::RBrace, "`}` to close use list");
                Some(idents)
            } else {
                self.pos = save;
                None
            }
        } else {
            None
        };
        let alias = if self.eat_kw(Kw::As) {
            Some(self.parse_ident())
        } else {
            None
        };
        UseDecl {
            path,
            items,
            alias,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_generics(&mut self) -> Generics {
        if !matches!(self.peek(), TokenKind::Lt) {
            return Generics::default();
        }
        let start = self.peek_span();
        self.bump();
        self.paren_depth += 1; // treat as bracket for newline purposes
        let mut params = Vec::new();
        loop {
            if matches!(self.peek(), TokenKind::Gt) {
                break;
            }
            params.push(self.parse_generic_param());
            if !matches!(self.peek(), TokenKind::Comma) {
                break;
            }
            self.bump();
        }
        self.paren_depth = self.paren_depth.saturating_sub(1);
        self.expect(&TokenKind::Gt, "`>` to close generics");
        Generics {
            params,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_generic_param(&mut self) -> GenericParam {
        let start = self.peek_span();
        match self.peek().clone() {
            TokenKind::Plus => {
                self.bump();
                let name = self.parse_ident();
                GenericParam::Covariant {
                    name,
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                }
            }
            TokenKind::Minus => {
                self.bump();
                let name = self.parse_ident();
                GenericParam::Contravariant {
                    name,
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                }
            }
            _ => {
                let name = self.parse_ident();
                let mut bounds = Vec::new();
                if matches!(self.peek(), TokenKind::Colon) {
                    self.bump();
                    bounds.push(self.parse_path());
                    while matches!(self.peek(), TokenKind::Plus) {
                        self.bump();
                        bounds.push(self.parse_path());
                    }
                }
                let is_effect_var = name.name.chars().next().map_or(false, |c| c.is_lowercase())
                    && bounds.is_empty();
                if is_effect_var {
                    GenericParam::Effect {
                        name,
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    }
                } else {
                    GenericParam::Type {
                        name,
                        bounds,
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    }
                }
            }
        }
    }

    fn parse_effect_row(&mut self) -> EffectRow {
        let start = self.peek_span();
        self.expect(&TokenKind::LBrace, "`{` to open effect row");
        self.paren_depth += 1;
        let mut effects = Vec::new();
        if !matches!(self.peek(), TokenKind::RBrace) {
            effects.push(self.parse_effect_atom());
            while matches!(self.peek(), TokenKind::Comma) {
                self.bump();
                if matches!(self.peek(), TokenKind::RBrace) {
                    break;
                }
                effects.push(self.parse_effect_atom());
            }
        }
        self.paren_depth = self.paren_depth.saturating_sub(1);
        self.expect(&TokenKind::RBrace, "`}` to close effect row");
        EffectRow {
            effects,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_effect_atom(&mut self) -> EffectAtom {
        let path = self.parse_path();
        EffectAtom {
            span: path.span,
            path,
        }
    }

    fn parse_params(&mut self) -> Vec<Param> {
        self.expect(&TokenKind::LParen, "`(` to start parameter list");
        let mut params = Vec::new();
        if !matches!(self.peek(), TokenKind::RParen) {
            params.push(self.parse_param());
            while matches!(self.peek(), TokenKind::Comma) {
                self.bump();
                if matches!(self.peek(), TokenKind::RParen) {
                    break;
                }
                params.push(self.parse_param());
            }
        }
        self.expect(&TokenKind::RParen, "`)` to close parameter list");
        params
    }

    fn parse_param(&mut self) -> Param {
        let start = self.peek_span();
        let name = self.parse_ident();
        self.expect(&TokenKind::Colon, "`:` after parameter name");
        // variadic: `name: ...type`
        let variadic = if matches!(self.peek(), TokenKind::DotDot) || matches!(self.peek(), TokenKind::DotDotEq) {
            // Treat `..` here as variadic marker per EBNF `"..." type`.
            // (Spec uses literal `...`; we map both `..` and `...` shapes.)
            self.bump();
            // If a third dot was lexed as DotDotEq accidentally, accept.
            true
        } else {
            false
        };
        let ty = self.parse_type();
        let default = if matches!(self.peek(), TokenKind::Eq) {
            self.bump();
            Some(self.parse_expr())
        } else {
            None
        };
        Param {
            name,
            ty,
            default,
            variadic,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    // ---- Functions -----------------------------------------------------

    fn parse_fn_decl(
        &mut self,
        vis: Visibility,
        attrs: Vec<Attribute>,
        is_async: bool,
    ) -> FnDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Fn);
        let name = self.parse_ident();
        let generics = self.parse_generics();
        let params = self.parse_params();
        let return_type = if matches!(self.peek(), TokenKind::Arrow) {
            self.bump();
            Some(self.parse_type_no_outer_effects())
        } else {
            None
        };
        let effect_row = if self.eat_kw(Kw::Uses) {
            Some(self.parse_effect_row())
        } else {
            None
        };
        let body = self.parse_block();
        FnDecl {
            vis,
            attrs,
            is_async,
            name,
            generics,
            params,
            return_type,
            effect_row,
            body,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    // ---- Types ----------------------------------------------------------

    fn parse_type(&mut self) -> Type {
        self.parse_type_inner(true, true)
    }

    /// Parse a type that may not consume a trailing `uses {...}` at its
    /// outermost level — used for the return-type slot of a `fn_decl`, where
    /// any trailing `uses` belongs to the function's effect row, not the
    /// type's. Function *types* like `(x: Int) -> Int uses { Net }` still
    /// pick up the `uses` because the recursive call below uses the full
    /// `parse_type` for their inner return type.
    fn parse_type_no_outer_effects(&mut self) -> Type {
        self.parse_type_inner(false, true)
    }

    /// Parse a type that doesn't consume trailing `@refinement(...)` at its
    /// outermost level — used in field positions, where refinements are
    /// collected on the field itself, not nested inside the type.
    fn parse_type_no_outer_refinements(&mut self) -> Type {
        self.parse_type_inner(true, false)
    }

    fn parse_type_inner(&mut self, allow_outer_uses: bool, allow_outer_refinements: bool) -> Type {
        let start = self.peek_span();
        let mut ty = self.parse_atom_type();
        loop {
            match self.peek() {
                TokenKind::Question => {
                    self.bump();
                    ty = Type {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: TypeKind::Option(Box::new(ty)),
                    };
                }
                TokenKind::Keyword(Kw::Uses) if allow_outer_uses => {
                    self.bump();
                    let row = self.parse_effect_row();
                    ty = Type {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: TypeKind::WithEffects {
                            inner: Box::new(ty),
                            effects: row,
                        },
                    };
                }
                TokenKind::At if allow_outer_refinements => {
                    let at_start = self.peek_span();
                    self.bump();
                    let name = self.parse_ident();
                    let args = if matches!(self.peek(), TokenKind::LParen) {
                        self.parse_paren_arg_exprs()
                    } else {
                        Vec::new()
                    };
                    let refinement = Refinement {
                        name,
                        args,
                        span: Span::in_file(at_start.start as usize, self.prev_end(), self.file_id),
                    };
                    ty = Type {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: TypeKind::Refined {
                            inner: Box::new(ty),
                            refinement,
                        },
                    };
                }
                _ => break,
            }
        }
        // Union: `T | U` (right-associative, low precedence).
        if matches!(self.peek(), TokenKind::Pipe) {
            self.bump();
            let rhs = self.parse_type();
            ty = Type {
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                kind: TypeKind::Union(Box::new(ty), Box::new(rhs)),
            };
        }
        ty
    }

    fn parse_atom_type(&mut self) -> Type {
        let start = self.peek_span();
        match self.peek().clone() {
            TokenKind::LBracket => {
                self.bump();
                let inner = self.parse_type();
                self.expect(&TokenKind::RBracket, "`]` to close list type");
                Type {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: TypeKind::List(Box::new(inner)),
                }
            }
            TokenKind::LBrace => {
                self.bump();
                self.paren_depth += 1;
                let first = self.parse_type();
                let kind = if matches!(self.peek(), TokenKind::Colon) {
                    self.bump();
                    let value = self.parse_type();
                    TypeKind::Map {
                        key: Box::new(first),
                        value: Box::new(value),
                    }
                } else {
                    TypeKind::Set(Box::new(first))
                };
                self.paren_depth = self.paren_depth.saturating_sub(1);
                self.expect(&TokenKind::RBrace, "`}` to close map/set type");
                Type {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind,
                }
            }
            TokenKind::LParen => {
                self.bump();
                if matches!(self.peek(), TokenKind::RParen) {
                    self.bump();
                    return Type {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: TypeKind::Unit,
                    };
                }
                let first = self.parse_type();
                let mut elems = vec![first];
                while matches!(self.peek(), TokenKind::Comma) {
                    self.bump();
                    if matches!(self.peek(), TokenKind::RParen) {
                        break;
                    }
                    elems.push(self.parse_type());
                }
                self.expect(&TokenKind::RParen, "`)` to close tuple/parenthesized type");
                let kind = if elems.len() == 1 {
                    elems.into_iter().next().unwrap().kind
                } else {
                    TypeKind::Tuple(elems)
                };
                Type {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind,
                }
            }
            TokenKind::Amp => {
                self.bump();
                let is_mut = self.eat_kw(Kw::Mut);
                let inner = self.parse_atom_type();
                Type {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: TypeKind::Ref {
                        is_mut,
                        inner: Box::new(inner),
                    },
                }
            }
            TokenKind::Ident(name) if name == "Tainted" => {
                self.bump();
                self.expect(&TokenKind::Lt, "`<` after `Tainted`");
                let inner = self.parse_type();
                self.expect(&TokenKind::Gt, "`>` to close `Tainted<...>`");
                Type {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: TypeKind::Tainted(Box::new(inner)),
                }
            }
            TokenKind::Ident(_) => {
                let path = self.parse_path();
                let mut generics = Vec::new();
                if matches!(self.peek(), TokenKind::Lt) {
                    self.bump();
                    self.paren_depth += 1;
                    if !matches!(self.peek(), TokenKind::Gt) {
                        generics.push(self.parse_type());
                        while matches!(self.peek(), TokenKind::Comma) {
                            self.bump();
                            if matches!(self.peek(), TokenKind::Gt) {
                                break;
                            }
                            generics.push(self.parse_type());
                        }
                    }
                    self.paren_depth = self.paren_depth.saturating_sub(1);
                    self.expect(&TokenKind::Gt, "`>` to close generics");
                }
                Type {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: TypeKind::Path { path, generics },
                }
            }
            other => {
                self.error(format!("expected a type, got {other:?}"), start);
                self.bump();
                Type {
                    span: start,
                    kind: TypeKind::Unit,
                }
            }
        }
    }

    // ---- Type / schema declarations ------------------------------------

    fn parse_type_decl(&mut self, vis: Visibility) -> TypeDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Type);
        let name = self.parse_ident();
        let generics = self.parse_generics();
        let body = if matches!(self.peek(), TokenKind::LBrace) {
            let fields = self.parse_field_block();
            TypeDeclBody::Record(fields)
        } else if matches!(self.peek(), TokenKind::Eq) {
            self.bump();
            // sum or alias
            if self.looks_like_sum_variants() {
                let variants = self.parse_sum_variants();
                TypeDeclBody::Sum(variants)
            } else {
                let inner = self.parse_type();
                let marker = if matches!(self.peek(), TokenKind::At) {
                    self.bump();
                    Some(self.parse_ident())
                } else {
                    None
                };
                if marker.is_some() {
                    TypeDeclBody::Newtype { inner, marker }
                } else {
                    TypeDeclBody::Alias(inner)
                }
            }
        } else if self.eat_kw(Kw::Type) {
            // `type X alias T` form, just in case the spec is flexible.
            TypeDeclBody::Alias(self.parse_type())
        } else {
            let span = self.peek_span();
            self.error("expected `{`, `=`, or `alias` after type name", span);
            TypeDeclBody::Alias(Type {
                span,
                kind: TypeKind::Unit,
            })
        };
        TypeDecl {
            vis,
            name,
            generics,
            body,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn looks_like_sum_variants(&self) -> bool {
        // Heuristic: a sum variant is `Ident` possibly followed by `(...)`,
        // and there's a `|` ahead before any block-terminating token.
        if !matches!(self.peek(), TokenKind::Ident(s) if s.chars().next().map_or(false, |c| c.is_uppercase()))
        {
            return false;
        }
        // Look ahead for a `|`.
        let mut i = self.pos;
        let mut depth = 0;
        while let Some(tok) = self.tokens.get(i) {
            match &tok.kind {
                TokenKind::Newline if depth == 0 => return false,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                TokenKind::Pipe if depth == 0 => return true,
                TokenKind::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn parse_sum_variants(&mut self) -> Vec<Variant> {
        let mut variants = vec![self.parse_variant()];
        while matches!(self.peek(), TokenKind::Pipe) {
            self.bump();
            variants.push(self.parse_variant());
        }
        variants
    }

    fn parse_variant(&mut self) -> Variant {
        let start = self.peek_span();
        let name = self.parse_ident();
        let mut fields = Vec::new();
        if matches!(self.peek(), TokenKind::LParen) {
            self.bump();
            if !matches!(self.peek(), TokenKind::RParen) {
                fields.push(self.parse_variant_field());
                while matches!(self.peek(), TokenKind::Comma) {
                    self.bump();
                    if matches!(self.peek(), TokenKind::RParen) {
                        break;
                    }
                    fields.push(self.parse_variant_field());
                }
            }
            self.expect(&TokenKind::RParen, "`)` to close variant");
        }
        Variant {
            name,
            fields,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_variant_field(&mut self) -> VariantField {
        // Named: `ident : type`. Anonymous: `type`.
        if matches!(self.peek(), TokenKind::Ident(_)) {
            // Lookahead one token: if next is `:`, it's named.
            let save = self.pos;
            let name = self.parse_ident();
            if matches!(self.peek(), TokenKind::Colon) {
                self.bump();
                let ty = self.parse_type();
                return VariantField::Named(Field {
                    doc: None,
                    name,
                    ty,
                    refinements: Vec::new(),
                    default: None,
                    span: Span::DUMMY,
                });
            }
            // Roll back.
            self.pos = save;
        }
        VariantField::Anonymous(self.parse_type())
    }

    fn parse_field_block(&mut self) -> Vec<Field> {
        self.expect(&TokenKind::LBrace, "`{` to start fields");
        self.eat_newlines();
        let mut fields = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            fields.push(self.parse_field());
            // Optional `,` between fields.
            if matches!(self.peek(), TokenKind::Comma) {
                self.bump();
            }
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close field block");
        fields
    }

    fn parse_field(&mut self) -> Field {
        let start = self.peek_span();
        let name = self.parse_ident();
        self.expect(&TokenKind::Colon, "`:` after field name");
        // Refinements live on the *field*, not nested in the type, so we
        // ask the type parser to leave any outer `@refinement(...)` for us.
        let ty = self.parse_type_no_outer_refinements();
        let mut refinements = Vec::new();
        while matches!(self.peek(), TokenKind::At) {
            self.bump();
            let r_start = self.peek_span();
            let r_name = self.parse_ident();
            let r_args = if matches!(self.peek(), TokenKind::LParen) {
                self.parse_paren_arg_exprs()
            } else {
                Vec::new()
            };
            refinements.push(Refinement {
                name: r_name,
                args: r_args,
                span: Span::in_file(r_start.start as usize, self.prev_end(), self.file_id),
            });
        }
        let default = if matches!(self.peek(), TokenKind::Eq) {
            self.bump();
            Some(self.parse_expr())
        } else {
            None
        };
        Field {
            doc: None,
            name,
            ty,
            refinements,
            default,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_schema_decl(&mut self, vis: Visibility) -> SchemaDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Schema);
        let name = self.parse_ident();
        let version = if matches!(self.peek(), TokenKind::At) {
            let save = self.pos;
            self.bump();
            if matches!(self.peek(), TokenKind::Ident(s) if s == "version") {
                self.bump();
                self.expect(&TokenKind::LParen, "`(` after @version");
                let v = if let TokenKind::Int { value, .. } = self.peek().clone() {
                    self.bump();
                    value as u32
                } else {
                    0
                };
                self.expect(&TokenKind::RParen, "`)` to close @version");
                Some(v)
            } else {
                self.pos = save;
                None
            }
        } else {
            None
        };
        // Body: fields and migrations.
        self.expect(&TokenKind::LBrace, "`{` to start schema body");
        self.eat_newlines();
        let mut fields = Vec::new();
        let mut migrations = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            if matches!(self.peek(), TokenKind::Ident(s) if s == "migrate") {
                let m_start = self.peek_span();
                self.bump();
                self.expect_kw_ident("from");
                let v = if let TokenKind::Ident(s) = self.peek().clone() {
                    if s.starts_with('v') {
                        self.bump();
                        s[1..].parse::<u32>().unwrap_or(0)
                    } else {
                        0
                    }
                } else if let TokenKind::Int { value, .. } = self.peek().clone() {
                    self.bump();
                    value as u32
                } else {
                    0
                };
                let body = self.parse_block();
                migrations.push(Migration {
                    from_version: v,
                    body,
                    span: Span::in_file(m_start.start as usize, self.prev_end(), self.file_id),
                });
            } else {
                fields.push(self.parse_field());
                if matches!(self.peek(), TokenKind::Comma) {
                    self.bump();
                }
            }
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close schema");
        SchemaDecl {
            vis,
            name,
            version,
            fields,
            migrations,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn expect_kw_ident(&mut self, what: &str) {
        if matches!(self.peek(), TokenKind::Ident(s) if s == what) {
            self.bump();
        } else {
            let span = self.peek_span();
            self.error(format!("expected `{what}`"), span);
        }
    }

    // ---- Agents / actors ------------------------------------------------

    fn parse_agent_decl(&mut self) -> AgentDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Agent);
        let name = self.parse_ident();
        let params = self.parse_params();
        let members = self.parse_agent_body();
        AgentDecl {
            name,
            params,
            members,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_actor_decl(&mut self) -> ActorDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Actor);
        let name = self.parse_ident();
        let params = self.parse_params();
        let members = self.parse_agent_body();
        ActorDecl {
            name,
            params,
            members,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_agent_body(&mut self) -> Vec<AgentMember> {
        self.expect(&TokenKind::LBrace, "`{` to start agent body");
        self.eat_newlines();
        let mut members = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            if let Some(m) = self.parse_agent_member() {
                members.push(m);
            } else {
                // Recover: skip to next newline.
                self.bump();
            }
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close agent body");
        members
    }

    fn parse_agent_member(&mut self) -> Option<AgentMember> {
        let start = self.peek_span();
        // `@durable state name : type [= expr]`
        let durable = if matches!(self.peek(), TokenKind::At) {
            let save = self.pos;
            self.bump();
            if matches!(self.peek(), TokenKind::Ident(s) if s == "durable") {
                self.bump();
                true
            } else {
                self.pos = save;
                false
            }
        } else {
            false
        };
        match self.peek().clone() {
            TokenKind::Keyword(Kw::State) => {
                self.bump();
                let name = self.parse_ident();
                self.expect(&TokenKind::Colon, "`:` after state name");
                let ty = self.parse_type();
                let init = if matches!(self.peek(), TokenKind::Eq) {
                    self.bump();
                    Some(self.parse_expr())
                } else {
                    None
                };
                Some(AgentMember::State {
                    durable,
                    name,
                    ty,
                    init,
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                })
            }
            TokenKind::Keyword(Kw::On) => {
                self.bump();
                // `on start`, `on stop`, `on error` lifecycle hooks
                let which = if matches!(self.peek(), TokenKind::Ident(s) if s == "start") {
                    self.bump();
                    Some(LifecycleEvent::Start)
                } else if matches!(self.peek(), TokenKind::Ident(s) if s == "stop") {
                    self.bump();
                    Some(LifecycleEvent::Stop)
                } else if matches!(self.peek(), TokenKind::Ident(s) if s == "error") {
                    self.bump();
                    Some(LifecycleEvent::Error)
                } else {
                    None
                };
                if let Some(which) = which {
                    let params = self.parse_params();
                    let return_type = if matches!(self.peek(), TokenKind::Arrow) {
                        self.bump();
                        Some(self.parse_type_no_outer_effects())
                    } else {
                        None
                    };
                    let body = self.parse_block();
                    return Some(AgentMember::Lifecycle(LifecycleHandler {
                        which,
                        params,
                        return_type,
                        body,
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    }));
                }
                let name = self.parse_ident();
                let params = self.parse_params();
                let return_type = if matches!(self.peek(), TokenKind::Arrow) {
                    self.bump();
                    Some(self.parse_type_no_outer_effects())
                } else {
                    None
                };
                let effect_row = if self.eat_kw(Kw::Uses) {
                    Some(self.parse_effect_row())
                } else {
                    None
                };
                let body = self.parse_block();
                Some(AgentMember::Handler(MessageHandler {
                    name,
                    params,
                    return_type,
                    effect_row,
                    body,
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                }))
            }
            TokenKind::Keyword(Kw::Model)
            | TokenKind::Keyword(Kw::Memory)
            | TokenKind::Ident(_) => {
                let setting = self.parse_agent_setting_or_fn(start)?;
                Some(setting)
            }
            TokenKind::Keyword(Kw::Fn) => Some(AgentMember::Fn(self.parse_fn_decl(
                Visibility::Private,
                Vec::new(),
                false,
            ))),
            _ => None,
        }
    }

    fn parse_agent_setting_or_fn(&mut self, start: Span) -> Option<AgentMember> {
        // Setting: `key: value`. Key may be a keyword (`model`, `memory`) or
        // an ident (`policy`, `mempolicy`, `context`, `budget`).
        let key = match self.peek().clone() {
            TokenKind::Keyword(kw) => {
                let span = self.bump().span;
                Ident {
                    name: kw.as_str().to_string(),
                    span,
                }
            }
            TokenKind::Ident(_) => self.parse_ident(),
            _ => return None,
        };
        self.expect(&TokenKind::Colon, "`:` after agent setting key");
        let value = match key.name.as_str() {
            "policy" | "mempolicy" => AgentSettingValue::Ident(self.parse_ident()),
            _ => AgentSettingValue::Expr(self.parse_expr()),
        };
        Some(AgentMember::Setting {
            key,
            value,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        })
    }

    // ---- Supervisor / graph / network / orchestrate / policy ---------

    fn parse_supervisor_decl(&mut self) -> SupervisorDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Supervisor);
        let name = self.parse_ident();
        self.expect(&TokenKind::LBrace, "`{` to start supervisor body");
        self.eat_newlines();
        let mut members = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            if matches!(self.peek(), TokenKind::Ident(s) if s == "child") {
                let m_start = self.peek_span();
                self.bump();
                let name = self.parse_ident();
                self.expect(&TokenKind::Eq, "`=` after child name");
                let call = self.parse_expr();
                let restart = if matches!(self.peek(), TokenKind::At) {
                    let save = self.pos;
                    self.bump();
                    if matches!(self.peek(), TokenKind::Ident(s) if s == "restart") {
                        self.bump();
                        self.expect(&TokenKind::LParen, "`(` after @restart");
                        let r = self.parse_ident();
                        self.expect(&TokenKind::RParen, "`)` to close @restart");
                        Some(r)
                    } else {
                        self.pos = save;
                        None
                    }
                } else {
                    None
                };
                members.push(SupervisorMember::Child {
                    name,
                    call,
                    restart,
                    span: Span::in_file(m_start.start as usize, self.prev_end(), self.file_id),
                });
            } else if matches!(self.peek(), TokenKind::Keyword(Kw::On)) {
                let m_start = self.peek_span();
                self.bump();
                let which = if matches!(self.peek(), TokenKind::Ident(s) if s == "start") {
                    self.bump();
                    LifecycleEvent::Start
                } else if matches!(self.peek(), TokenKind::Ident(s) if s == "stop") {
                    self.bump();
                    LifecycleEvent::Stop
                } else {
                    self.bump_if_ident("error");
                    LifecycleEvent::Error
                };
                let params = self.parse_params();
                let return_type = if matches!(self.peek(), TokenKind::Arrow) {
                    self.bump();
                    Some(self.parse_type())
                } else {
                    None
                };
                let body = self.parse_block();
                members.push(SupervisorMember::OnHandler(LifecycleHandler {
                    which,
                    params,
                    return_type,
                    body,
                    span: Span::in_file(m_start.start as usize, self.prev_end(), self.file_id),
                }));
            } else {
                let m_start = self.peek_span();
                let key = self.parse_ident();
                self.expect(&TokenKind::Colon, "`:` after supervisor setting key");
                let value = self.parse_expr();
                members.push(SupervisorMember::Setting {
                    key,
                    value,
                    span: Span::in_file(m_start.start as usize, self.prev_end(), self.file_id),
                });
            }
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close supervisor");
        SupervisorDecl {
            name,
            members,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn bump_if_ident(&mut self, what: &str) -> bool {
        if matches!(self.peek(), TokenKind::Ident(s) if s == what) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn parse_graph_decl(&mut self) -> GraphDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Graph);
        let name = self.parse_ident();
        let params = self.parse_params();
        self.expect(&TokenKind::Arrow, "`->` for graph return type");
        let return_type = self.parse_type();
        self.expect(&TokenKind::LBrace, "`{` to start graph body");
        self.eat_newlines();
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut done = None;
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            if self.bump_if_ident("node") {
                let n_name = self.parse_ident();
                self.expect(&TokenKind::Colon, "`:` after node name");
                let value = self.parse_expr();
                nodes.push(GraphNode {
                    name: n_name,
                    value,
                    span: Span::DUMMY,
                });
            } else if self.bump_if_ident("edge") {
                let e_start = self.peek_span();
                let raw = self.consume_raw_until_newline();
                edges.push(GraphEdge {
                    raw,
                    span: Span::in_file(e_start.start as usize, self.prev_end(), self.file_id),
                });
            } else if self.bump_if_ident("done") {
                self.expect(&TokenKind::Colon, "`:` after `done`");
                done = Some(self.parse_expr());
            } else {
                let span = self.peek_span();
                self.error("expected `node`, `edge`, or `done` inside `graph`", span);
                self.bump();
            }
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close graph");
        GraphDecl {
            name,
            params,
            return_type,
            nodes,
            edges,
            done,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn consume_raw_until_newline(&mut self) -> String {
        let start = self.pos;
        while let Some(t) = self.tokens.get(self.pos) {
            if matches!(t.kind, TokenKind::Newline | TokenKind::RBrace | TokenKind::Eof) {
                break;
            }
            self.pos += 1;
        }
        let from = self.tokens.get(start).map(|t| t.span.start as usize).unwrap_or(0);
        let to = self
            .tokens
            .get(self.pos.saturating_sub(1))
            .map(|t| t.span.end as usize)
            .unwrap_or(from);
        self.src[from..to].trim().to_string()
    }

    // ---- Models / tools / memory / prompt -------------------------------

    fn parse_model_decl(&mut self) -> ModelDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Model);
        let name = self.parse_ident();
        self.expect(&TokenKind::Eq, "`=` after model name");
        let call = self.parse_expr();
        let settings = if matches!(self.peek(), TokenKind::LBrace) {
            self.bump();
            self.eat_newlines();
            let mut out = Vec::new();
            while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
                let key = self.parse_ident();
                self.expect(&TokenKind::Colon, "`:` after setting key");
                let value = self.parse_expr();
                out.push((key, value));
                if matches!(self.peek(), TokenKind::Comma) {
                    self.bump();
                }
                self.eat_newlines();
            }
            self.expect(&TokenKind::RBrace, "`}` to close model settings");
            out
        } else {
            Vec::new()
        };
        ModelDecl {
            name,
            call,
            settings,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_tool_decl(&mut self) -> ToolDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Tool);
        let name = self.parse_ident();
        let params = self.parse_params();
        self.expect(&TokenKind::Arrow, "`->` for tool return type");
        let return_type = self.parse_type_no_outer_effects();
        let effect_row = if self.eat_kw(Kw::Uses) {
            Some(self.parse_effect_row())
        } else {
            None
        };
        let attrs = self.parse_attributes();
        let body = if self.eat_kw(Kw::Extern) {
            // Two accepted forms:
            //   extern "abi" "symbol"        (quoted ABI — original)
            //   extern python "path:fn"      (bare-ident ABI — §35.2 bridges)
            let abi = if let TokenKind::Ident(_) = self.peek() {
                self.parse_ident().name
            } else {
                self.expect_string_literal()
            };
            let symbol = self.expect_string_literal();
            ToolBody::Extern { abi, symbol }
        } else {
            ToolBody::Block(self.parse_block())
        };
        ToolDecl {
            doc: None,
            name,
            params,
            return_type,
            effect_row,
            attrs,
            body,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_memory_decl(&mut self) -> MemoryDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Memory);
        let name = self.parse_ident();
        self.expect(&TokenKind::Eq, "`=` after memory name");
        let call = self.parse_expr();
        MemoryDecl {
            name,
            call,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_prompt_decl(&mut self) -> PromptDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Prompt);
        let name = self.parse_ident();
        let params = self.parse_params();
        self.expect(&TokenKind::Arrow, "`->` for prompt return type");
        let return_type = self.parse_type();
        self.expect(&TokenKind::LBrace, "`{` to start prompt body");
        self.eat_newlines();
        let mut slots = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            slots.push(self.parse_prompt_slot());
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close prompt");
        PromptDecl {
            name,
            params,
            return_type,
            slots,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_prompt_slot(&mut self) -> PromptSlot {
        let start = self.peek_span();
        // Labeled slot: known label ident or keyword followed by `:`.
        let label = self.match_prompt_slot_label();
        if let Some(label) = label {
            self.expect(&TokenKind::Colon, "`:` after prompt slot label");
            let value = self.parse_expr();
            PromptSlot {
                label: Some(label),
                value,
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            }
        } else {
            // Bare string = system slot.
            let value = self.parse_expr();
            PromptSlot {
                label: None,
                value,
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            }
        }
    }

    fn match_prompt_slot_label(&mut self) -> Option<Ident> {
        const LABELS: &[&str] = &[
            "system",
            "user",
            "memory",
            "tools",
            "examples",
            "output",
            "max_steps",
            "budget",
            "stop",
            "context",
        ];
        // Special: `system+` is also accepted; we represent it as identifier
        // `system+`. Easier: accept ident `system` followed by `+` and merge.
        match self.peek().clone() {
            TokenKind::Ident(name) if LABELS.contains(&name.as_str()) => {
                // Lookahead — must be followed by `:` (or `+:` for system+).
                let save = self.pos;
                let id = self.parse_ident();
                if matches!(self.peek(), TokenKind::Plus) && id.name == "system" {
                    self.bump();
                    if matches!(self.peek(), TokenKind::Colon) {
                        return Some(Ident {
                            name: "system+".to_string(),
                            span: id.span,
                        });
                    }
                    self.pos = save;
                    return None;
                }
                if matches!(self.peek(), TokenKind::Colon) {
                    Some(id)
                } else {
                    self.pos = save;
                    None
                }
            }
            TokenKind::Keyword(Kw::Memory) => {
                let save = self.pos;
                let span = self.bump().span;
                if matches!(self.peek(), TokenKind::Colon) {
                    Some(Ident {
                        name: "memory".to_string(),
                        span,
                    })
                } else {
                    self.pos = save;
                    None
                }
            }
            _ => None,
        }
    }

    fn expect_string_literal(&mut self) -> String {
        if let TokenKind::String { parts, .. } = self.peek().clone() {
            self.bump();
            let mut out = String::new();
            for p in parts {
                if let LexStringPart::Text(t) = p {
                    out.push_str(&t);
                }
            }
            out
        } else {
            let span = self.peek_span();
            self.error("expected a string literal", span);
            String::new()
        }
    }

    // ---- Const / trait / impl ------------------------------------------

    fn parse_const_decl(&mut self, vis: Visibility) -> ConstDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Const);
        let name = self.parse_ident();
        let ty = if matches!(self.peek(), TokenKind::Colon) {
            self.bump();
            Some(self.parse_type())
        } else {
            None
        };
        self.expect(&TokenKind::Eq, "`=` after const name");
        let value = self.parse_expr();
        ConstDecl {
            vis,
            name,
            ty,
            value,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_trait_decl(&mut self, vis: Visibility) -> TraitDecl {
        let start = self.peek_span();
        self.expect_kw(Kw::Trait);
        let name = self.parse_ident();
        let generics = self.parse_generics();
        self.expect(&TokenKind::LBrace, "`{` to start trait body");
        self.eat_newlines();
        let mut items = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            items.push(self.parse_fn_decl(Visibility::Private, Vec::new(), false));
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close trait");
        TraitDecl {
            vis,
            name,
            generics,
            items,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_impl_block(&mut self) -> ImplBlock {
        let start = self.peek_span();
        self.expect_kw(Kw::Impl);
        let generics = self.parse_generics();
        // `impl Trait for Type` or `impl Type`.
        let first = self.parse_type();
        let (trait_path, target) = if matches!(self.peek(), TokenKind::Keyword(Kw::For)) {
            self.bump();
            let target = self.parse_type();
            let path = if let TypeKind::Path { path, .. } = first.kind {
                Some(path)
            } else {
                None
            };
            (path, target)
        } else {
            (None, first)
        };
        self.expect(&TokenKind::LBrace, "`{` to start impl block");
        self.eat_newlines();
        let mut items = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            let _attrs = self.parse_attributes();
            let vis = if self.eat_kw(Kw::Pub) {
                Visibility::Public
            } else {
                Visibility::Private
            };
            let is_async = self.eat_kw(Kw::Async);
            items.push(self.parse_fn_decl(vis, Vec::new(), is_async));
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close impl block");
        ImplBlock {
            generics,
            trait_path,
            target,
            items,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    // ---- Blocks & statements --------------------------------------------

    fn parse_block(&mut self) -> Block {
        let start = self.peek_span();
        self.expect(&TokenKind::LBrace, "`{` to start block");
        self.eat_newlines();
        let mut stmts = Vec::new();
        let mut tail: Option<Expr> = None;
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            let stmt_start = self.pos;
            let stmt = self.parse_stmt();
            // Semicolons (§9.2: legal but optional) and newlines act as
            // statement separators. After a statement, eat any of them; if
            // the *next* non-trivia is `}`, an expression statement becomes
            // the block's tail value.
            while matches!(self.peek_raw(), TokenKind::Semi | TokenKind::Newline) {
                self.pos += 1;
            }
            match stmt {
                Stmt::Expr(e) if matches!(self.peek(), TokenKind::RBrace) => {
                    tail = Some(e);
                    break;
                }
                s => stmts.push(s),
            }
            if self.pos == stmt_start {
                self.bump();
            }
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close block");
        Block {
            stmts,
            tail,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    fn parse_stmt(&mut self) -> Stmt {
        let start = self.peek_span();
        match self.peek().clone() {
            TokenKind::Keyword(Kw::Let) => {
                self.bump();
                let pattern = self.parse_pattern();
                let ty = if matches!(self.peek(), TokenKind::Colon) {
                    self.bump();
                    Some(self.parse_type())
                } else {
                    None
                };
                self.expect(&TokenKind::Eq, "`=` after let binding");
                let value = self.parse_expr();
                Stmt::Let {
                    pattern,
                    ty,
                    value,
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                }
            }
            TokenKind::Keyword(Kw::Var) => {
                self.bump();
                let name = self.parse_ident();
                let ty = if matches!(self.peek(), TokenKind::Colon) {
                    self.bump();
                    Some(self.parse_type())
                } else {
                    None
                };
                self.expect(&TokenKind::Eq, "`=` after var binding");
                let value = self.parse_expr();
                Stmt::Var {
                    name,
                    ty,
                    value,
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                }
            }
            _ => Stmt::Expr(self.parse_expr()),
        }
    }

    // ---- Patterns -------------------------------------------------------

    fn parse_pattern(&mut self) -> Pattern {
        let start = self.peek_span();
        let head = self.parse_pattern_primary();
        // `name @ inner` binder.
        let head = if let PatternKind::Binding(name) = &*head.kind {
            if matches!(self.peek(), TokenKind::At) {
                let name = name.clone();
                self.bump();
                let inner = self.parse_pattern_primary();
                Pattern {
                    kind: Box::new(PatternKind::Binder { name, inner }),
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                }
            } else {
                head
            }
        } else {
            head
        };
        // `a | b` or-pattern. The lexer's `Pipe` is shared with bit-or; we
        // accept it here unconditionally.
        if matches!(self.peek(), TokenKind::Pipe) {
            self.bump();
            let rhs = self.parse_pattern();
            return Pattern {
                kind: Box::new(PatternKind::Or(head, rhs)),
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            };
        }
        head
    }

    fn parse_pattern_primary(&mut self) -> Pattern {
        let start = self.peek_span();
        match self.peek().clone() {
            TokenKind::Ident(name) if name == "_" => {
                self.bump();
                Pattern {
                    kind: Box::new(PatternKind::Wildcard),
                    span: start,
                }
            }
            // §37 contextual keywords — `let prompt = ...`, `let model = ...`,
            // etc. The soft-keyword binding is a single-segment Path identical
            // in shape to an Ident binding.
            TokenKind::Keyword(kw) if Self::is_soft_keyword(kw) => {
                let name = kw.as_str().to_string();
                let span = self.bump().span;
                Pattern {
                    kind: Box::new(PatternKind::Binding(Ident { name, span })),
                    span,
                }
            }
            TokenKind::Ident(_) => {
                // Could be a binding or a constructor `Path(...)`.
                let path = self.parse_path();
                if matches!(self.peek(), TokenKind::LParen) {
                    self.bump();
                    let mut fields = Vec::new();
                    if !matches!(self.peek(), TokenKind::RParen) {
                        fields.push(self.parse_pattern());
                        while matches!(self.peek(), TokenKind::Comma) {
                            self.bump();
                            if matches!(self.peek(), TokenKind::RParen) {
                                break;
                            }
                            fields.push(self.parse_pattern());
                        }
                    }
                    self.expect(&TokenKind::RParen, "`)` to close constructor pattern");
                    Pattern {
                        kind: Box::new(PatternKind::Constructor { path, fields }),
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    }
                } else if path.segments.len() == 1 {
                    Pattern {
                        kind: Box::new(PatternKind::Binding(path.segments.into_iter().next().unwrap())),
                        span: start,
                    }
                } else {
                    Pattern {
                        kind: Box::new(PatternKind::Constructor {
                            path,
                            fields: Vec::new(),
                        }),
                        span: start,
                    }
                }
            }
            TokenKind::LBrace => {
                self.bump();
                self.paren_depth += 1;
                let mut fields = Vec::new();
                if !matches!(self.peek(), TokenKind::RBrace) {
                    fields.push(self.parse_field_pattern());
                    while matches!(self.peek(), TokenKind::Comma) {
                        self.bump();
                        if matches!(self.peek(), TokenKind::RBrace) {
                            break;
                        }
                        fields.push(self.parse_field_pattern());
                    }
                }
                self.paren_depth = self.paren_depth.saturating_sub(1);
                self.expect(&TokenKind::RBrace, "`}` to close record pattern");
                Pattern {
                    kind: Box::new(PatternKind::Record(fields)),
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                }
            }
            TokenKind::LBracket => {
                self.bump();
                let mut elems = Vec::new();
                if !matches!(self.peek(), TokenKind::RBracket) {
                    elems.push(self.parse_pattern());
                    while matches!(self.peek(), TokenKind::Comma) {
                        self.bump();
                        if matches!(self.peek(), TokenKind::RBracket) {
                            break;
                        }
                        elems.push(self.parse_pattern());
                    }
                }
                self.expect(&TokenKind::RBracket, "`]` to close list pattern");
                Pattern {
                    kind: Box::new(PatternKind::List(elems)),
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                }
            }
            TokenKind::LParen => {
                self.bump();
                let mut elems = Vec::new();
                if !matches!(self.peek(), TokenKind::RParen) {
                    elems.push(self.parse_pattern());
                    while matches!(self.peek(), TokenKind::Comma) {
                        self.bump();
                        if matches!(self.peek(), TokenKind::RParen) {
                            break;
                        }
                        elems.push(self.parse_pattern());
                    }
                }
                self.expect(&TokenKind::RParen, "`)` to close tuple pattern");
                let kind = if elems.len() == 1 {
                    let p = elems.into_iter().next().unwrap();
                    return p;
                } else {
                    PatternKind::Tuple(elems)
                };
                Pattern {
                    kind: Box::new(kind),
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                }
            }
            // Literal patterns.
            _ => {
                if let Some(lit) = self.try_parse_literal() {
                    Pattern {
                        kind: Box::new(PatternKind::Literal(lit)),
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    }
                } else {
                    self.error("expected a pattern", start);
                    Pattern {
                        kind: Box::new(PatternKind::Wildcard),
                        span: start,
                    }
                }
            }
        }
    }

    fn parse_field_pattern(&mut self) -> FieldPattern {
        let start = self.peek_span();
        let name = self.parse_ident();
        let pattern = if matches!(self.peek(), TokenKind::Colon) {
            self.bump();
            Some(self.parse_pattern())
        } else {
            None
        };
        FieldPattern {
            name,
            pattern,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        }
    }

    // ---- Expressions: Pratt precedence ----------------------------------

    fn parse_expr(&mut self) -> Expr {
        self.parse_expr_bp(0)
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Expr {
        let start = self.peek_span();
        let mut lhs = self.parse_unary_expr();
        loop {

            // Infix binary / pipeline operators.
            let (op, l_bp, r_bp) = match self.peek() {
                TokenKind::Eq => (BinOp::Assign, 1, 2),
                TokenKind::PlusEq => (BinOp::AddAssign, 1, 2),
                TokenKind::MinusEq => (BinOp::SubAssign, 1, 2),
                TokenKind::StarEq => (BinOp::MulAssign, 1, 2),
                TokenKind::SlashEq => (BinOp::DivAssign, 1, 2),
                TokenKind::PercentEq => (BinOp::RemAssign, 1, 2),
                TokenKind::Pipeline => (BinOp::Or /* placeholder */, 3, 4),
                // `??` binds looser than `||` so `a || b ?? c` groups as
                // `(a || b) ?? c` — the coalesce wraps the whole boolean.
                TokenKind::QuestionQuestion => (BinOp::Coalesce, 4, 5),
                TokenKind::Keyword(Kw::Or) | TokenKind::PipePipe => (BinOp::Or, 5, 6),
                TokenKind::Keyword(Kw::And) | TokenKind::AmpAmp => (BinOp::And, 7, 8),
                TokenKind::EqEq => (BinOp::Eq, 9, 10),
                TokenKind::NotEq => (BinOp::NotEq, 9, 10),
                TokenKind::Lt => (BinOp::Lt, 11, 12),
                TokenKind::LtEq => (BinOp::LtEq, 11, 12),
                TokenKind::Gt => (BinOp::Gt, 11, 12),
                TokenKind::GtEq => (BinOp::GtEq, 11, 12),
                TokenKind::DotDot => (BinOp::Range, 13, 14),
                TokenKind::DotDotEq => (BinOp::RangeInclusive, 13, 14),
                TokenKind::Pipe => (BinOp::BitOr, 15, 16),
                TokenKind::Caret => (BinOp::BitXor, 17, 18),
                TokenKind::Amp => (BinOp::BitAnd, 19, 20),
                TokenKind::Shl => (BinOp::Shl, 21, 22),
                TokenKind::Shr => (BinOp::Shr, 21, 22),
                TokenKind::Plus => (BinOp::Add, 23, 24),
                TokenKind::Minus => (BinOp::Sub, 23, 24),
                TokenKind::Star => (BinOp::Mul, 25, 26),
                TokenKind::Slash => (BinOp::Div, 25, 26),
                TokenKind::Percent => (BinOp::Rem, 25, 26),
                _ => break,
            };
            if l_bp < min_bp {
                break;
            }
            let is_pipeline = matches!(self.peek(), TokenKind::Pipeline);
            self.bump();
            let rhs = self.parse_expr_bp(r_bp);
            let span = Span::in_file(start.start as usize, self.prev_end(), self.file_id);
            lhs = if is_pipeline {
                Expr {
                    span,
                    kind: Box::new(ExprKind::Pipeline { lhs, rhs }),
                }
            } else {
                Expr {
                    span,
                    kind: Box::new(ExprKind::Binary { op, lhs, rhs }),
                }
            };
        }
        lhs
    }

    fn parse_unary_expr(&mut self) -> Expr {
        let start = self.peek_span();
        let op = match self.peek() {
            TokenKind::Minus => Some(UnOp::Neg),
            TokenKind::Bang | TokenKind::Keyword(Kw::Not) => Some(UnOp::Not),
            TokenKind::Tilde => Some(UnOp::BitNot),
            TokenKind::Amp => {
                self.bump();
                let is_mut = self.eat_kw(Kw::Mut);
                let operand = self.parse_unary_expr();
                return Expr {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: Box::new(ExprKind::Unary {
                        op: if is_mut { UnOp::RefMut } else { UnOp::Ref },
                        operand,
                    }),
                };
            }
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            // Recurse — the inner call also runs the postfix chain, so
            // `!is_even(3)` parses as `!(is_even(3))`. Without this the
            // outer Pratt loop would apply postfix to `!is_even` first,
            // which is the wrong precedence.
            let operand = self.parse_unary_expr();
            return Expr {
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                kind: Box::new(ExprKind::Unary { op, operand }),
            };
        }
        // No unary: parse atom + every postfix that wraps it.
        let atom = self.parse_atom_expr();
        self.apply_postfix_chain(atom, start)
    }

    /// Apply every postfix operator that wraps `lhs`: call, index, field,
    /// method, await, `?`, `!`, `as`, `is`. Returns when the next token
    /// isn't a postfix.
    fn apply_postfix_chain(&mut self, mut lhs: Expr, start: Span) -> Expr {
        loop {
            match self.peek() {
                TokenKind::LParen => {
                    let args = self.parse_paren_call_args();
                    lhs = Expr {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: Box::new(ExprKind::Call { callee: lhs, args }),
                    };
                }
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.parse_expr();
                    self.expect(&TokenKind::RBracket, "`]` to close index");
                    lhs = Expr {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: Box::new(ExprKind::Index {
                            receiver: lhs,
                            index,
                        }),
                    };
                }
                TokenKind::Dot => {
                    self.bump();
                    let name = self.parse_ident_or_kw();
                    if matches!(self.peek(), TokenKind::LParen) {
                        let args = self.parse_paren_call_args();
                        lhs = Expr {
                            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                            kind: Box::new(ExprKind::MethodCall {
                                receiver: lhs,
                                method: name,
                                generics: Vec::new(),
                                args,
                            }),
                        };
                    } else {
                        lhs = Expr {
                            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                            kind: Box::new(ExprKind::Field {
                                receiver: lhs,
                                name,
                            }),
                        };
                    }
                }
                TokenKind::QuestionDot => {
                    // `receiver?.name` — nil-safe field access. (Method
                    // form `?.m(...)` falls back to a plain method call on
                    // the non-nil branch; v0 supports the field form
                    // natively and lets `?.m()` parse as safe-field then
                    // call.)
                    self.bump();
                    let name = self.parse_ident_or_kw();
                    lhs = Expr {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: Box::new(ExprKind::SafeField {
                            receiver: lhs,
                            name,
                        }),
                    };
                }
                TokenKind::Question => {
                    self.bump();
                    lhs = Expr {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: Box::new(ExprKind::Try(lhs)),
                    };
                }
                TokenKind::Bang => {
                    self.bump();
                    lhs = Expr {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: Box::new(ExprKind::Force(lhs)),
                    };
                }
                TokenKind::Keyword(Kw::Await) => {
                    self.bump();
                    lhs = Expr {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: Box::new(ExprKind::Await(lhs)),
                    };
                }
                TokenKind::Keyword(Kw::As) => {
                    self.bump();
                    let ty = self.parse_type();
                    lhs = Expr {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: Box::new(ExprKind::Cast { expr: lhs, ty }),
                    };
                }
                TokenKind::Keyword(Kw::Is) => {
                    self.bump();
                    let target = IsTarget::Type(self.parse_type());
                    lhs = Expr {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: Box::new(ExprKind::Is { expr: lhs, target }),
                    };
                }
                _ => break,
            }
        }
        lhs
    }

    fn parse_atom_expr(&mut self) -> Expr {
        let start = self.peek_span();
        match self.peek().clone() {
            TokenKind::Keyword(Kw::SelfKw) => {
                self.bump();
                Expr {
                    span: start,
                    kind: Box::new(ExprKind::SelfExpr),
                }
            }
            TokenKind::Keyword(Kw::Nil) => {
                self.bump();
                Expr {
                    span: start,
                    kind: Box::new(ExprKind::Nil),
                }
            }
            // `chan` is a reserved keyword (it marks the channel type at
            // §9.3), but in expression-atom position users write `chan()`
            // to construct one. Treat the bare keyword as a path reference
            // to the runtime built-in.
            TokenKind::Keyword(Kw::Chan) => {
                let span_tok = self.bump().span;
                Expr {
                    span: span_tok,
                    kind: Box::new(ExprKind::Path(Path {
                        span: span_tok,
                        segments: vec![Ident {
                            name: "chan".to_string(),
                            span: span_tok,
                        }],
                    })),
                }
            }
            TokenKind::Keyword(Kw::If) => self.parse_if_expr(),
            TokenKind::Keyword(Kw::Match) => self.parse_match_expr(),
            TokenKind::Keyword(Kw::When) => self.parse_when_expr(),
            TokenKind::Keyword(Kw::For) => self.parse_for_expr(),
            TokenKind::Keyword(Kw::While) => self.parse_while_expr(),
            TokenKind::Keyword(Kw::Select) => self.parse_select_expr(),
            TokenKind::Keyword(Kw::Parallel) => self.parse_parallel_expr(),
            TokenKind::Keyword(Kw::Ask) => self.parse_ask_expr(),
            TokenKind::Keyword(Kw::Generate) | TokenKind::Keyword(Kw::Gen) => {
                self.parse_generate_expr()
            }
            TokenKind::Keyword(Kw::Plan) => self.parse_plan_expr(),
            TokenKind::Keyword(Kw::Stream) => self.parse_stream_expr(),
            TokenKind::Keyword(Kw::With) => self.parse_with_expr(),
            TokenKind::Keyword(Kw::Try) => self.parse_try_recover_expr(),
            TokenKind::Keyword(Kw::Spawn) => {
                self.bump();
                // Bind tight enough to absorb postfix call/index/field but
                // not any surrounding infix expression. `parse_unary_expr`
                // skipped postfix; this re-enters the Pratt loop with a
                // very high min_bp so only the postfix arm fires.
                let call = self.parse_expr_bp(99);
                Expr {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: Box::new(ExprKind::Spawn(call)),
                }
            }
            TokenKind::Keyword(Kw::Return) => {
                self.bump();
                let value = if self.expr_can_start() {
                    Some(self.parse_expr())
                } else {
                    None
                };
                Expr {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: Box::new(ExprKind::Return(value)),
                }
            }
            TokenKind::Keyword(Kw::Break) => {
                self.bump();
                let label = if let TokenKind::Ident(_) = self.peek() {
                    Some(self.parse_ident())
                } else {
                    None
                };
                Expr {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: Box::new(ExprKind::Break(label)),
                }
            }
            TokenKind::Keyword(Kw::Continue) => {
                self.bump();
                let label = if let TokenKind::Ident(_) = self.peek() {
                    Some(self.parse_ident())
                } else {
                    None
                };
                Expr {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: Box::new(ExprKind::Continue(label)),
                }
            }
            TokenKind::Keyword(Kw::Yield) => {
                self.bump();
                let value = self.parse_expr();
                Expr {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: Box::new(ExprKind::Yield(value)),
                }
            }
            TokenKind::Keyword(Kw::Defer) => {
                self.bump();
                let value = self.parse_expr();
                Expr {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: Box::new(ExprKind::Defer(value)),
                }
            }
            TokenKind::At => {
                // `@ident` in expression position is a static agent address.
                // Attribute and refinement uses of `@` are consumed by their
                // respective parsers before reaching this point.
                self.bump();
                if let TokenKind::Ident(name) = self.peek().clone() {
                    let ident_span = self.bump().span;
                    Expr {
                        span: Span::in_file(start.start as usize, ident_span.end as usize, self.file_id),
                        kind: Box::new(ExprKind::Literal(Literal::AgentAddr {
                            is_dynamic: false,
                            text: name,
                        })),
                    }
                } else {
                    let sp = self.peek_span();
                    self.error("expected identifier after `@` (agent address)", sp);
                    Expr {
                        span: start,
                        kind: Box::new(ExprKind::Nil),
                    }
                }
            }
            // `|x| …` and `fn (…) …` lambdas, plus the zero-param `|| …`
            // form (the lexer emits `||` as a single PipePipe token).
            TokenKind::Keyword(Kw::Fn) | TokenKind::Pipe | TokenKind::PipePipe => {
                self.parse_lambda()
            }
            TokenKind::LBrace => self.parse_brace_literal_or_block_as_expr(),
            TokenKind::LBracket => self.parse_list_literal(),
            TokenKind::LParen => self.parse_paren_or_tuple(),
            TokenKind::Ident(_) => {
                // In expression position, atoms are single identifiers — the
                // postfix `.` handler in the Pratt loop walks any field or
                // method access. Eating a multi-segment path here would
                // produce `Call { Path[obj.method] }` instead of a proper
                // `MethodCall`, which the type checker depends on.
                let id = self.parse_ident();
                Expr {
                    span: id.span,
                    kind: Box::new(ExprKind::Path(Path {
                        span: id.span,
                        segments: vec![id],
                    })),
                }
            }
            // §37 contextual keywords — same shape as the Ident arm above.
            // `print(prompt)`, `ask model { ... }`, `self.memory.recall(q)`,
            // `tool.input_schema()`, `for a in agents { ... }` all parse.
            TokenKind::Keyword(kw) if Self::is_soft_keyword(kw) => {
                let name = kw.as_str().to_string();
                let span = self.bump().span;
                let id = Ident { name, span };
                Expr {
                    span,
                    kind: Box::new(ExprKind::Path(Path {
                        span,
                        segments: vec![id],
                    })),
                }
            }
            _ => {
                if let Some(lit) = self.try_parse_literal() {
                    Expr {
                        span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                        kind: Box::new(ExprKind::Literal(lit)),
                    }
                } else {
                    let span = self.peek_span();
                    self.error(format!("expected an expression, got {:?}", self.peek()), span);
                    self.bump();
                    Expr {
                        span,
                        kind: Box::new(ExprKind::Nil),
                    }
                }
            }
        }
    }

    fn expr_can_start(&self) -> bool {
        match self.peek_raw() {
            TokenKind::Newline | TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof => false,
            _ => true,
        }
    }

    fn parse_if_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump(); // `if`
        let cond = self.parse_expr_bp(1); // exclude assignment
        let then_branch = self.parse_block();
        let else_branch = if self.eat_kw(Kw::Else) {
            if matches!(self.peek(), TokenKind::Keyword(Kw::If)) {
                Some(Box::new(ExprOrBlock::Expr(self.parse_if_expr())))
            } else {
                Some(Box::new(ExprOrBlock::Block(self.parse_block())))
            }
        } else {
            None
        };
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::If {
                cond,
                then_branch,
                else_branch,
            }),
        }
    }

    fn parse_match_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump();
        let scrutinee = self.parse_expr_bp(1);
        self.expect(&TokenKind::LBrace, "`{` to start match body");
        self.eat_newlines();
        let mut arms = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            let a_start = self.peek_span();
            let pattern = self.parse_pattern();
            let guard = if self.eat_kw(Kw::If) {
                Some(self.parse_expr())
            } else {
                None
            };
            self.expect(&TokenKind::FatArrow, "`=>` after match pattern");
            let body = self.parse_expr();
            arms.push(MatchArm {
                pattern,
                guard,
                body,
                span: Span::in_file(a_start.start as usize, self.prev_end(), self.file_id),
            });
            if matches!(self.peek(), TokenKind::Comma) {
                self.bump();
            }
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close match");
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::Match { scrutinee, arms }),
        }
    }

    fn parse_when_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump();
        let cond = self.parse_expr_bp(1);
        let then_branch = self.parse_block();
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::When { cond, then_branch }),
        }
    }

    fn parse_for_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump();
        // Optional `await` between `for` and the pattern → async-stream form.
        let is_await = self.eat_kw(Kw::Await);
        let pat = self.parse_pattern();
        self.expect_kw(Kw::In);
        let iter = self.parse_expr_bp(1);
        let body = self.parse_block();
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::For {
                pat,
                iter,
                body,
                is_await,
            }),
        }
    }

    fn parse_while_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump();
        let cond = self.parse_expr_bp(1);
        let body = self.parse_block();
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::While { cond, body }),
        }
    }

    /// Parse `select { ... arms ... }` where each arm has one of three
    /// shapes (using existing tokens — no `<-` operator):
    ///
    ///   * `name = recv(channel_expr) => block_or_expr`
    ///   * `_ = timeout(duration_expr) => block_or_expr`
    ///   * `else => block_or_expr`
    ///
    /// Arms are tried in declaration order. The first arm whose channel
    /// has a value pending wins; if none is ready the `timeout` arm fires
    /// (if any), or the `else` arm (if any), otherwise the runtime errors.
    fn parse_select_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump(); // `select`
        self.expect(&TokenKind::LBrace, "`{` to open `select`");
        self.eat_newlines();
        let mut arms: Vec<axon_ast::SelectArm> = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            let arm_start = self.peek_span();
            let kind = if matches!(self.peek(), TokenKind::Keyword(Kw::Else)) {
                self.bump();
                axon_ast::SelectArmKind::Else
            } else {
                // binding ident, then `=`
                let binding = self.parse_ident();
                self.expect(&TokenKind::Eq, "`=` after select-arm binding");
                // RHS is `recv(expr)` or `timeout(expr)` — recognized by name.
                let lookahead_ident = if let TokenKind::Ident(name) = self.peek() {
                    Some(name.clone())
                } else {
                    None
                };
                match lookahead_ident.as_deref() {
                    Some("recv") => {
                        self.bump();
                        self.expect(&TokenKind::LParen, "`(` after `recv`");
                        let channel = self.parse_expr_bp(1);
                        self.expect(&TokenKind::RParen, "`)` to close `recv(...)`");
                        axon_ast::SelectArmKind::Recv { binding, channel }
                    }
                    Some("timeout") => {
                        self.bump();
                        self.expect(&TokenKind::LParen, "`(` after `timeout`");
                        let duration = self.parse_expr_bp(1);
                        self.expect(&TokenKind::RParen, "`)` to close `timeout(...)`");
                        axon_ast::SelectArmKind::Timeout { duration }
                    }
                    _ => {
                        let span = self.peek_span();
                        self.error(
                            "expected `recv(chan)` or `timeout(dur)` in select arm",
                            span,
                        );
                        // Recover: skip to the next FatArrow or RBrace.
                        while !matches!(
                            self.peek(),
                            TokenKind::FatArrow | TokenKind::RBrace | TokenKind::Eof
                        ) {
                            self.bump();
                        }
                        axon_ast::SelectArmKind::Else
                    }
                }
            };
            self.expect(&TokenKind::FatArrow, "`=>` after select-arm header");
            let body = if matches!(self.peek(), TokenKind::LBrace) {
                self.parse_block()
            } else {
                // Single-expression body: wrap as a block with tail expr.
                let e = self.parse_expr_bp(1);
                let span = e.span;
                axon_ast::Block {
                    span,
                    stmts: Vec::new(),
                    tail: Some(e),
                }
            };
            arms.push(axon_ast::SelectArm {
                kind,
                body,
                span: Span::in_file(arm_start.start as usize, self.prev_end(), self.file_id),
            });
            self.eat_newlines();
            if matches!(self.peek(), TokenKind::Comma) {
                self.bump();
                self.eat_newlines();
            }
        }
        self.expect(&TokenKind::RBrace, "`}` to close `select`");
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::Select(arms)),
        }
    }

    /// Parse `parallel { arm, arm, ... }` (Stage 36).
    ///
    /// Each arm is an expression — the eval-time check enforces that each
    /// arm is a single `ask` expression (the only shape Stage 36 supports;
    /// Stage 37 lifts the restriction). Empty blocks are a parse error.
    /// Trailing commas accepted. Newlines tolerated like other block forms.
    fn parse_parallel_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump(); // `parallel`
        self.expect(&TokenKind::LBrace, "`{` to open `parallel`");
        self.eat_newlines();
        let mut arms: Vec<Expr> = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            arms.push(self.parse_expr_bp(1));
            self.eat_newlines();
            if matches!(self.peek(), TokenKind::Comma) {
                self.bump();
                self.eat_newlines();
            } else {
                break;
            }
        }
        self.expect(&TokenKind::RBrace, "`}` to close `parallel`");
        if arms.is_empty() {
            let span = Span::in_file(start.start as usize, self.prev_end(), self.file_id);
            self.error(
                "`parallel { }` requires at least one arm (each arm is an `ask` expression)",
                span,
            );
        }
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::Parallel(arms)),
        }
    }

    fn parse_ask_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump();
        let target = self.parse_expr_bp(20); // tight precedence so `ask x.method()` works
        self.expect(&TokenKind::LBrace, "`{` to start ask body");
        self.eat_newlines();
        let mut slots = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            slots.push(self.parse_prompt_slot());
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close ask body");
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::Ask { target, slots }),
        }
    }

    fn parse_generate_expr(&mut self) -> Expr {
        let start = self.peek_span();
        let is_gen = matches!(self.peek(), TokenKind::Keyword(Kw::Gen));
        self.bump();
        self.expect(&TokenKind::Lt, "`<` for generate type argument");
        let schema = self.parse_type();
        self.expect(&TokenKind::Gt, "`>` to close generate type argument");
        self.expect(&TokenKind::LParen, "`(` to start generate args");
        let model = self.parse_expr();
        self.expect(&TokenKind::Comma, "`,` between model and prompt");
        let prompt = self.parse_expr();
        let mut extra = Vec::new();
        while matches!(self.peek(), TokenKind::Comma) {
            self.bump();
            if matches!(self.peek(), TokenKind::RParen) {
                break;
            }
            extra.push(self.parse_call_arg());
        }
        self.expect(&TokenKind::RParen, "`)` to close generate args");
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::Generate {
                is_gen_shorthand: is_gen,
                schema,
                model,
                prompt,
                extra,
            }),
        }
    }

    fn parse_plan_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump();
        self.expect_kw(Kw::With);
        let target = self.parse_expr_bp(20);
        self.expect(&TokenKind::LBrace, "`{` to start plan body");
        self.eat_newlines();
        let mut slots = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) && !self.at_eof() {
            slots.push(self.parse_prompt_slot());
            self.eat_newlines();
        }
        self.expect(&TokenKind::RBrace, "`}` to close plan body");
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::Plan { target, slots }),
        }
    }

    fn parse_stream_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump();
        let item_type = if matches!(self.peek(), TokenKind::Lt) {
            self.bump();
            let t = self.parse_type();
            self.expect(&TokenKind::Gt, "`>` to close stream type");
            Some(t)
        } else {
            None
        };
        let body = self.parse_block();
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::Stream { item_type, body }),
        }
    }

    /// `try { ... } recover |e| { ... }`. The `recover` clause is
    /// required — a bare `try { ... }` has no meaning distinct from a
    /// plain block, so we require the handler and error otherwise.
    fn parse_try_recover_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump(); // consume `try`
        let body = self.parse_block();
        let recover = if self.eat_kw(Kw::Recover) {
            let lambda_e = self.parse_lambda();
            match *lambda_e.kind {
                ExprKind::Lambda(l) => l,
                _ => {
                    let span = self.peek_span();
                    self.error("expected a `|e| { ... }` lambda after `recover`", span);
                    LambdaExpr {
                        params: Vec::new(),
                        body: Expr {
                            span: lambda_e.span,
                            kind: Box::new(ExprKind::UnitLit),
                        },
                        span: lambda_e.span,
                    }
                }
            }
        } else {
            let span = self.peek_span();
            self.error("`try { ... }` must be followed by `recover |e| { ... }`", span);
            LambdaExpr {
                params: Vec::new(),
                body: Expr {
                    span,
                    kind: Box::new(ExprKind::UnitLit),
                },
                span,
            }
        };
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::TryRecover { body, recover }),
        }
    }

    fn parse_with_expr(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump();
        let head = if self.bump_if_ident("budget") {
            let args = self.parse_paren_call_args();
            WithHead::Budget(args)
        } else if self.bump_if_ident("recording") {
            self.expect(&TokenKind::LParen, "`(` after `recording`");
            let e = self.parse_expr();
            self.expect(&TokenKind::RParen, "`)` to close `recording`");
            WithHead::Recording(e)
        } else if self.bump_if_ident("scope") {
            self.expect_kw(Kw::As);
            WithHead::Scope(self.parse_ident())
        } else if self.bump_if_ident("span") {
            let args = self.parse_paren_call_args();
            WithHead::Span(args)
        } else {
            let span = self.peek_span();
            self.error(
                "expected `budget`, `recording`, `scope`, or `span` after `with`",
                span,
            );
            WithHead::Budget(Vec::new())
        };
        let body = self.parse_block();
        let on_exceeded = if self.bump_if_ident("on_exceeded") {
            let lambda_e = self.parse_lambda();
            if let ExprKind::Lambda(l) = *lambda_e.kind {
                Some(l)
            } else {
                None
            }
        } else {
            None
        };
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::With {
                head,
                body,
                on_exceeded,
            }),
        }
    }

    fn parse_lambda(&mut self) -> Expr {
        let start = self.peek_span();
        // The zero-param form `|| …` lexes as a single `||` token.
        if matches!(self.peek(), TokenKind::PipePipe) {
            self.bump();
            let body = if matches!(self.peek(), TokenKind::LBrace) {
                let blk = self.parse_block();
                Expr {
                    span: blk.span,
                    kind: Box::new(ExprKind::Block(blk)),
                }
            } else {
                self.parse_expr_bp(1)
            };
            let mut params: Vec<axon_ast::Ident> = Vec::new();
            if expr_uses_it(&body) {
                params.push(axon_ast::Ident {
                    name: "it".to_string(),
                    span: body.span,
                });
            }
            let span = Span::in_file(start.start as usize, self.prev_end(), self.file_id);
            return Expr {
                span,
                kind: Box::new(ExprKind::Lambda(LambdaExpr { params, body, span })),
            };
        }
        let mut params = if matches!(self.peek(), TokenKind::Pipe) {
            self.bump();
            let mut params = Vec::new();
            if !matches!(self.peek(), TokenKind::Pipe) {
                params.push(self.parse_ident());
                while matches!(self.peek(), TokenKind::Comma) {
                    self.bump();
                    if matches!(self.peek(), TokenKind::Pipe) {
                        break;
                    }
                    params.push(self.parse_ident());
                }
            }
            self.expect(&TokenKind::Pipe, "`|` to close lambda params");
            params
        } else {
            // `fn (params) block` form.
            self.expect_kw(Kw::Fn);
            // Build params from a parenthesized list, then a block.
            let parsed_params = self.parse_params();
            let body = self.parse_block();
            let lambda = LambdaExpr {
                params: parsed_params.into_iter().map(|p| p.name).collect(),
                body: Expr {
                    span: body.span,
                    kind: Box::new(ExprKind::Block(body)),
                },
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            };
            return Expr {
                span: lambda.span,
                kind: Box::new(ExprKind::Lambda(lambda)),
            };
        };
        // A `{` immediately after `|params|` is a *block* body (like the
        // `fn (params) { ... }` form above), not a set/record literal.
        // To return a brace literal from a lambda, parenthesize it:
        // `|x| ({ a: 1 })`.
        let body = if matches!(self.peek(), TokenKind::LBrace) {
            let blk = self.parse_block();
            Expr {
                span: blk.span,
                kind: Box::new(ExprKind::Block(blk)),
            }
        } else {
            self.parse_expr_bp(1)
        };
        // §64.1: `it` in a single-arg closure. A zero-param `|| …` whose
        // body references `it` gets an implicit parameter named `it`, so
        // `xs.map(|| it * 2)` works. A zero-param body that doesn't
        // mention `it` stays a true thunk (`|| expensive()`), preserving
        // the flow-combinator usage where closures are called with no
        // args.
        if params.is_empty() && expr_uses_it(&body) {
            params.push(axon_ast::Ident {
                name: "it".to_string(),
                span: body.span,
            });
        }
        let lambda = LambdaExpr {
            params,
            body,
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
        };
        Expr {
            span: lambda.span,
            kind: Box::new(ExprKind::Lambda(lambda)),
        }
    }

    fn parse_brace_literal_or_block_as_expr(&mut self) -> Expr {
        // Used in expression position: this is a brace literal, not a block,
        // because block expressions appear only after if/match/etc.
        let start = self.peek_span();
        self.bump();
        self.paren_depth += 1;
        self.eat_newlines();
        if matches!(self.peek(), TokenKind::RBrace) {
            self.paren_depth = self.paren_depth.saturating_sub(1);
            self.bump();
            return Expr {
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                kind: Box::new(ExprKind::BraceLit(BraceLit::Empty)),
            };
        }
        // Decide: record (`ident: expr` or `ident = expr`), map (`expr: expr`),
        // or set (`expr,`). Use lookahead one step.
        // Try `ident colon` or `ident eq` first.
        let saved = self.pos;
        if let TokenKind::Ident(_) = self.peek() {
            let _ident_save = self.pos;
            let _ = self.parse_ident();
            if matches!(self.peek(), TokenKind::Colon | TokenKind::Eq) {
                // Record literal.
                self.pos = saved;
                let mut fields = Vec::new();
                fields.push(self.parse_record_field());
                while matches!(self.peek(), TokenKind::Comma) {
                    self.bump();
                    self.eat_newlines();
                    if matches!(self.peek(), TokenKind::RBrace) {
                        break;
                    }
                    fields.push(self.parse_record_field());
                }
                self.paren_depth = self.paren_depth.saturating_sub(1);
                self.expect(&TokenKind::RBrace, "`}` to close record literal");
                return Expr {
                    span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                    kind: Box::new(ExprKind::BraceLit(BraceLit::Record(fields))),
                };
            }
            self.pos = saved;
        }
        // Otherwise: a map `K: V` or a set.
        let first = self.parse_expr();
        if matches!(self.peek(), TokenKind::Colon) {
            self.bump();
            let value = self.parse_expr();
            let mut entries = vec![(first, value)];
            while matches!(self.peek(), TokenKind::Comma) {
                self.bump();
                self.eat_newlines();
                if matches!(self.peek(), TokenKind::RBrace) {
                    break;
                }
                let k = self.parse_expr();
                self.expect(&TokenKind::Colon, "`:` in map literal");
                let v = self.parse_expr();
                entries.push((k, v));
            }
            self.paren_depth = self.paren_depth.saturating_sub(1);
            self.expect(&TokenKind::RBrace, "`}` to close map literal");
            Expr {
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                kind: Box::new(ExprKind::BraceLit(BraceLit::Map(entries))),
            }
        } else {
            let mut elems = vec![first];
            while matches!(self.peek(), TokenKind::Comma) {
                self.bump();
                self.eat_newlines();
                if matches!(self.peek(), TokenKind::RBrace) {
                    break;
                }
                elems.push(self.parse_expr());
            }
            self.paren_depth = self.paren_depth.saturating_sub(1);
            self.expect(&TokenKind::RBrace, "`}` to close set literal");
            Expr {
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                kind: Box::new(ExprKind::BraceLit(BraceLit::Set(elems))),
            }
        }
    }

    fn parse_record_field(&mut self) -> (Ident, Expr) {
        // Record-literal keys, like named-arg names, may be keywords —
        // `tools = { search = web_search }` from the README binds `search`,
        // and constructs like `{ model = brain, ... }` are common.
        let name = match self.peek().clone() {
            TokenKind::Keyword(kw) => {
                let span = self.bump().span;
                Ident {
                    name: kw.as_str().to_string(),
                    span,
                }
            }
            _ => self.parse_ident(),
        };
        // accept `:` or `=` as the separator.
        if !(matches!(self.peek(), TokenKind::Colon) || matches!(self.peek(), TokenKind::Eq)) {
            let span = self.peek_span();
            self.error("expected `:` or `=` in record field", span);
        } else {
            self.bump();
        }
        let value = self.parse_expr();
        (name, value)
    }

    fn parse_list_literal(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump();
        let mut elems = Vec::new();
        if !matches!(self.peek(), TokenKind::RBracket) {
            elems.push(self.parse_expr());
            while matches!(self.peek(), TokenKind::Comma) {
                self.bump();
                if matches!(self.peek(), TokenKind::RBracket) {
                    break;
                }
                elems.push(self.parse_expr());
            }
        }
        self.expect(&TokenKind::RBracket, "`]` to close list literal");
        Expr {
            span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
            kind: Box::new(ExprKind::ListLit(elems)),
        }
    }

    fn parse_paren_or_tuple(&mut self) -> Expr {
        let start = self.peek_span();
        self.bump();
        if matches!(self.peek(), TokenKind::RParen) {
            self.bump();
            return Expr {
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                kind: Box::new(ExprKind::UnitLit),
            };
        }
        let first = self.parse_expr();
        if matches!(self.peek(), TokenKind::Comma) {
            let mut elems = vec![first];
            while matches!(self.peek(), TokenKind::Comma) {
                self.bump();
                if matches!(self.peek(), TokenKind::RParen) {
                    break;
                }
                elems.push(self.parse_expr());
            }
            self.expect(&TokenKind::RParen, "`)` to close tuple");
            return Expr {
                span: Span::in_file(start.start as usize, self.prev_end(), self.file_id),
                kind: Box::new(ExprKind::Tuple(elems)),
            };
        }
        self.expect(&TokenKind::RParen, "`)` to close parenthesized expression");
        first
    }

    fn parse_paren_call_args(&mut self) -> Vec<CallArg> {
        self.expect(&TokenKind::LParen, "`(` to start call args");
        let mut args = Vec::new();
        if !matches!(self.peek(), TokenKind::RParen) {
            args.push(self.parse_call_arg());
            while matches!(self.peek(), TokenKind::Comma) {
                self.bump();
                if matches!(self.peek(), TokenKind::RParen) {
                    break;
                }
                args.push(self.parse_call_arg());
            }
        }
        self.expect(&TokenKind::RParen, "`)` to close call args");
        args
    }

    fn parse_call_arg(&mut self) -> CallArg {
        // Try named: `ident = expr` or `ident : expr`.
        //
        // Named-arg names may be keywords (`model = brain`, `memory = kb`,
        // `tools = { ... }` per the README), so we accept any Ident *or*
        // Keyword token in this position.
        let is_name = matches!(self.peek(), TokenKind::Ident(_) | TokenKind::Keyword(_));
        if is_name {
            let save = self.pos;
            let name = match self.peek().clone() {
                TokenKind::Ident(s) => {
                    let span = self.bump().span;
                    Ident { name: s, span }
                }
                TokenKind::Keyword(kw) => {
                    let span = self.bump().span;
                    Ident {
                        name: kw.as_str().to_string(),
                        span,
                    }
                }
                _ => unreachable!(),
            };
            if matches!(self.peek(), TokenKind::Eq) || matches!(self.peek(), TokenKind::Colon) {
                self.bump();
                let value = self.parse_expr();
                return CallArg::Named { name, value };
            }
            self.pos = save;
        }
        CallArg::Positional(self.parse_expr())
    }

    fn parse_paren_arg_exprs(&mut self) -> Vec<Expr> {
        let args = self.parse_paren_call_args();
        args.into_iter()
            .map(|a| match a {
                CallArg::Positional(e) => e,
                CallArg::Named { value, .. } => value,
            })
            .collect()
    }

    fn try_parse_literal(&mut self) -> Option<Literal> {
        match self.peek().clone() {
            TokenKind::Int { value, .. } => {
                self.bump();
                Some(Literal::Int { value })
            }
            TokenKind::Float { lexeme } => {
                self.bump();
                Some(Literal::Float { lexeme })
            }
            TokenKind::Decimal { lexeme } => {
                self.bump();
                Some(Literal::Decimal { lexeme })
            }
            TokenKind::Money { amount, currency } => {
                self.bump();
                Some(Literal::Money { amount, currency })
            }
            TokenKind::Duration { nanos, original } => {
                self.bump();
                Some(Literal::Duration { nanos, original })
            }
            TokenKind::Date { y, m, d } => {
                self.bump();
                Some(Literal::Date { y, m, d })
            }
            TokenKind::DateTime {
                y, m, d, hh, mm, ss, tz,
            } => {
                self.bump();
                Some(Literal::DateTime {
                    y,
                    m,
                    d,
                    hh,
                    mm,
                    ss,
                    utc: matches!(tz, Tz::Utc),
                })
            }
            TokenKind::Time { hh, mm, ss } => {
                self.bump();
                Some(Literal::Time { hh, mm, ss })
            }
            TokenKind::Bool(b) => {
                self.bump();
                Some(Literal::Bool(b))
            }
            TokenKind::Char(c) => {
                self.bump();
                Some(Literal::Char(c))
            }
            TokenKind::String { kind, parts } => {
                self.bump();
                let lit_kind = match kind {
                    LexStringKind::Regular => StringLitKind::Regular,
                    LexStringKind::Bytes => StringLitKind::Bytes,
                    LexStringKind::Raw => StringLitKind::Raw,
                    LexStringKind::MultiLine => StringLitKind::MultiLine,
                    LexStringKind::Prompt => StringLitKind::Prompt,
                };
                let ast_parts = parts
                    .into_iter()
                    .map(|p| match p {
                        LexStringPart::Text(s) => StringPart::Text(s),
                        LexStringPart::Interp { text, span } => {
                            // Re-parse interpolated expressions through the
                            // full parser. Errors here are propagated.
                            let sub = SourceFile::new("<interp>", text);
                            let (sub_tokens, sub_diags) = lex::tokenize(&sub);
                            self.diagnostics.extend(sub_diags);
                            let mut sub_parser = Parser::new(sub_tokens, sub.text(), self.file_id);
                            let expr = sub_parser.parse_expr();
                            self.diagnostics.extend(sub_parser.diagnostics);
                            StringPart::Interp(Expr {
                                span,
                                kind: expr.kind,
                            })
                        }
                    })
                    .collect();
                Some(Literal::String {
                    kind: lit_kind,
                    parts: ast_parts,
                })
            }
            TokenKind::HashLit { algo, hex } => {
                self.bump();
                Some(Literal::HashLit { algo, hex })
            }
            TokenKind::AddrLit { kind, text } => {
                self.bump();
                Some(Literal::AgentAddr {
                    is_dynamic: matches!(kind, AddrLitKind::Dynamic),
                    text,
                })
            }
            _ => None,
        }
    }
}

// Suppress an unused-import warning on the `Token` re-export; it is part of
// our minimum public surface for downstream consumers.
#[allow(dead_code)]
fn _assert_token_used(_t: Token) {}

/// Does `e` reference the bare identifier `it`? Used to decide whether
/// a zero-param `|| …` closure gets an implicit `it` parameter (§64.1).
/// Covers the expression forms `it` realistically appears in; unknown
/// forms default to `false` (treated as a thunk).
fn expr_uses_it(e: &Expr) -> bool {
    use axon_ast::ExprKind as K;
    fn is_it_path(p: &axon_ast::Path) -> bool {
        p.segments.len() == 1 && p.segments[0].name == "it"
    }
    match &*e.kind {
        K::Path(p) => is_it_path(p),
        K::Binary { lhs, rhs, .. } => expr_uses_it(lhs) || expr_uses_it(rhs),
        K::Unary { operand, .. } => expr_uses_it(operand),
        K::Pipeline { lhs, rhs } => expr_uses_it(lhs) || expr_uses_it(rhs),
        K::Field { receiver, .. } | K::SafeField { receiver, .. } => expr_uses_it(receiver),
        K::Index { receiver, index } => expr_uses_it(receiver) || expr_uses_it(index),
        K::Await(x) | K::Try(x) | K::Force(x) | K::Spawn(x) => expr_uses_it(x),
        K::Cast { expr, .. } => expr_uses_it(expr),
        K::Is { expr, .. } => expr_uses_it(expr),
        K::Call { callee, args } => {
            expr_uses_it(callee) || args.iter().any(call_arg_uses_it)
        }
        K::MethodCall { receiver, args, .. } => {
            expr_uses_it(receiver) || args.iter().any(call_arg_uses_it)
        }
        K::ListLit(xs) | K::Tuple(xs) => xs.iter().any(expr_uses_it),
        K::Block(b) => block_uses_it(b),
        K::If { cond, then_branch, else_branch } => {
            expr_uses_it(cond)
                || block_uses_it(then_branch)
                || else_branch.as_ref().map(|eb| match &**eb {
                    axon_ast::ExprOrBlock::Block(b) => block_uses_it(b),
                    axon_ast::ExprOrBlock::Expr(x) => expr_uses_it(x),
                }).unwrap_or(false)
        }
        _ => false,
    }
}

fn call_arg_uses_it(a: &axon_ast::CallArg) -> bool {
    match a {
        axon_ast::CallArg::Positional(e) => expr_uses_it(e),
        axon_ast::CallArg::Named { value, .. } => expr_uses_it(value),
    }
}

fn block_uses_it(b: &axon_ast::Block) -> bool {
    b.tail.as_ref().map(expr_uses_it).unwrap_or(false)
        || b.stmts.iter().any(|s| match s {
            axon_ast::Stmt::Expr(e) => expr_uses_it(e),
            axon_ast::Stmt::Let { value, .. } | axon_ast::Stmt::Var { value, .. } => {
                expr_uses_it(value)
            }
            _ => false,
        })
}

/// Convert a money amount lexeme (e.g. `"0.50"`, `"20"`, `"1.5"`) into
/// integer cents, rounding to the nearest cent. The lexer stores the
/// digit/decimal text without the currency, so this never has to parse
/// a currency symbol.
fn money_str_to_cents(amount: &str) -> i64 {
    let cleaned = amount.replace('_', "");
    let f: f64 = cleaned.parse().unwrap_or(0.0);
    (f * 100.0).round() as i64
}
