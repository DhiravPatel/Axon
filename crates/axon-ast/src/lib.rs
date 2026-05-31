//! Abstract syntax tree for the Axon programming language.
//!
//! Every production in §43 of the language reference has a matching node here.
//! Less-detailed items in §43 (trait/impl/const/effect/test/eval/config) get
//! structural nodes that capture enough to round-trip; the parser fills them
//! out as those sections are spec'd in more detail.
//!
//! Every node carries a [`Span`] pointing back into its source. Identifiers
//! are owned `String`s — Axon source is small enough that interning is not
//! worth the complexity until we measure a real bottleneck.

use axon_diag::Span;

// ===========================================================================
// Root
// ===========================================================================

#[derive(Clone, Debug)]
pub struct Program {
    pub items: Vec<Item>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Item {
    Use(UseDecl),
    Fn(FnDecl),
    Type(TypeDecl),
    Schema(SchemaDecl),
    Agent(AgentDecl),
    Actor(ActorDecl),
    Supervisor(SupervisorDecl),
    Graph(GraphDecl),
    Network(NetworkDecl),
    Orchestrate(OrchestrateDecl),
    Policy(PolicyDecl),
    MemPolicy(MemPolicyDecl),
    Model(ModelDecl),
    Tool(ToolDecl),
    Memory(MemoryDecl),
    Prompt(PromptDecl),
    Trait(TraitDecl),
    Impl(ImplBlock),
    Const(ConstDecl),
    Effect(EffectDecl),
    Test(TestDecl),
    Eval(EvalDecl),
    Config(ConfigDecl),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Use(x) => x.span,
            Item::Fn(x) => x.span,
            Item::Type(x) => x.span,
            Item::Schema(x) => x.span,
            Item::Agent(x) => x.span,
            Item::Actor(x) => x.span,
            Item::Supervisor(x) => x.span,
            Item::Graph(x) => x.span,
            Item::Network(x) => x.span,
            Item::Orchestrate(x) => x.span,
            Item::Policy(x) => x.span,
            Item::MemPolicy(x) => x.span,
            Item::Model(x) => x.span,
            Item::Tool(x) => x.span,
            Item::Memory(x) => x.span,
            Item::Prompt(x) => x.span,
            Item::Trait(x) => x.span,
            Item::Impl(x) => x.span,
            Item::Const(x) => x.span,
            Item::Effect(x) => x.span,
            Item::Test(x) => x.span,
            Item::Eval(x) => x.span,
            Item::Config(x) => x.span,
        }
    }
}

// ===========================================================================
// Common pieces
// ===========================================================================

#[derive(Clone, Debug)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Path {
    pub segments: Vec<Ident>,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Visibility {
    Private,
    Public,
}

#[derive(Clone, Debug)]
pub struct Attribute {
    /// `@retry(times = 3)` vs `#[derive(Eq)]`.
    pub style: AttrStyle,
    pub name: Path,
    pub args: Vec<Expr>,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AttrStyle {
    /// `@name(args)` — applied to the following declaration / message handler.
    At,
    /// `#[name(args)]` — outer attribute, Rust-style.
    Outer,
}

#[derive(Clone, Debug, Default)]
pub struct Generics {
    pub params: Vec<GenericParam>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum GenericParam {
    /// `T`, `T: Bound + Bound`.
    Type {
        name: Ident,
        bounds: Vec<Path>,
        span: Span,
    },
    /// `+T` covariant marker.
    Covariant { name: Ident, span: Span },
    /// `-T` contravariant marker.
    Contravariant { name: Ident, span: Span },
    /// A lowercase row variable used in effect rows.
    Effect { name: Ident, span: Span },
}

#[derive(Clone, Debug, Default)]
pub struct EffectRow {
    pub effects: Vec<EffectAtom>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EffectAtom {
    /// `Net`, `Fs.Read`, or a row variable like `e`.
    pub path: Path,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Refinement {
    /// `@range(0.0, 100.0)`, `@min_len(1)`, `@matches(/.../)`.
    pub name: Ident,
    pub args: Vec<Expr>,
    pub span: Span,
}

// ===========================================================================
// Use declarations
// ===========================================================================

#[derive(Clone, Debug)]
pub struct UseDecl {
    pub path: Path,
    /// `use foo.bar.{a, b, c}` — `Some(idents)` if a brace list was given.
    pub items: Option<Vec<Ident>>,
    pub alias: Option<Ident>,
    pub span: Span,
}

// ===========================================================================
// Functions
// ===========================================================================

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub vis: Visibility,
    pub attrs: Vec<Attribute>,
    pub is_async: bool,
    pub name: Ident,
    pub generics: Generics,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub effect_row: Option<EffectRow>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: Ident,
    pub ty: Type,
    pub default: Option<Expr>,
    pub variadic: bool,
    pub span: Span,
}

// ===========================================================================
// Types
// ===========================================================================

#[derive(Clone, Debug)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeKind {
    Path {
        path: Path,
        generics: Vec<Type>,
    },
    List(Box<Type>),
    Map {
        key: Box<Type>,
        value: Box<Type>,
    },
    Set(Box<Type>),
    Tuple(Vec<Type>),
    /// `()`
    Unit,
    Ref {
        is_mut: bool,
        inner: Box<Type>,
    },
    Tainted(Box<Type>),
    Fn {
        params: Vec<Param>,
        return_type: Box<Type>,
        effects: Option<EffectRow>,
    },
    /// `T?` — Option<T>.
    Option(Box<Type>),
    /// `T uses {...}` suffix on another type.
    WithEffects {
        inner: Box<Type>,
        effects: EffectRow,
    },
    /// `T @refinement(...)`.
    Refined {
        inner: Box<Type>,
        refinement: Refinement,
    },
    /// `T | U`.
    Union(Box<Type>, Box<Type>),
}

// ===========================================================================
// Type declarations (records, sums, aliases, newtypes)
// ===========================================================================

#[derive(Clone, Debug)]
pub struct TypeDecl {
    pub vis: Visibility,
    pub name: Ident,
    pub generics: Generics,
    pub body: TypeDeclBody,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeDeclBody {
    Record(Vec<Field>),
    Sum(Vec<Variant>),
    Alias(Type),
    /// `type X = T @nominal` — a distinct-from-the-base newtype.
    Newtype { inner: Type, marker: Option<Ident> },
}

#[derive(Clone, Debug)]
pub struct Field {
    pub doc: Option<String>,
    pub name: Ident,
    pub ty: Type,
    pub refinements: Vec<Refinement>,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Variant {
    pub name: Ident,
    pub fields: Vec<VariantField>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum VariantField {
    Named(Field),
    Anonymous(Type),
}

// ===========================================================================
// Schema declarations
// ===========================================================================

#[derive(Clone, Debug)]
pub struct SchemaDecl {
    pub vis: Visibility,
    pub name: Ident,
    pub version: Option<u32>,
    pub fields: Vec<Field>,
    pub migrations: Vec<Migration>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Migration {
    pub from_version: u32,
    pub body: Block,
    pub span: Span,
}

// ===========================================================================
// Agents, actors, supervisors, graphs, networks, orchestrators
// ===========================================================================

#[derive(Clone, Debug)]
pub struct AgentDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub members: Vec<AgentMember>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ActorDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub members: Vec<AgentMember>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum AgentMember {
    State {
        durable: bool,
        name: Ident,
        ty: Type,
        init: Option<Expr>,
        span: Span,
    },
    /// `model: expr`, `memory: expr`, `policy: ident`, `mempolicy: ident`,
    /// `context: expr`, `budget: expr`.
    Setting {
        key: Ident,
        value: AgentSettingValue,
        span: Span,
    },
    /// `on message_name(params) -> Type uses { ... } { ... }`.
    Handler(MessageHandler),
    /// `on start(...)`, `on stop(...)`, `on error(...)`.
    Lifecycle(LifecycleHandler),
    /// Nested `fn` decls inside the agent block.
    Fn(FnDecl),
}

#[derive(Clone, Debug)]
pub enum AgentSettingValue {
    Expr(Expr),
    Ident(Ident),
}

#[derive(Clone, Debug)]
pub struct MessageHandler {
    pub name: Ident,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub effect_row: Option<EffectRow>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct LifecycleHandler {
    pub which: LifecycleEvent,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub body: Block,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LifecycleEvent {
    Start,
    Stop,
    Error,
}

#[derive(Clone, Debug)]
pub struct SupervisorDecl {
    pub name: Ident,
    pub members: Vec<SupervisorMember>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum SupervisorMember {
    Setting {
        key: Ident,
        value: Expr,
        span: Span,
    },
    Child {
        name: Ident,
        call: Expr,
        restart: Option<Ident>,
        span: Span,
    },
    OnHandler(LifecycleHandler),
}

#[derive(Clone, Debug)]
pub struct GraphDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub done: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct GraphNode {
    pub name: Ident,
    pub value: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct GraphEdge {
    /// Raw text of the edge expression — sub-grammar deferred.
    pub raw: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct NetworkDecl {
    pub name: Ident,
    pub agents: Vec<Field>,
    pub topology: Vec<GraphEdge>,
    pub members: Vec<AgentMember>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct OrchestrateDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct PolicyDecl {
    pub name: Ident,
    /// Structured policy clauses (§30). The sub-grammar covers
    /// allow/deny rules, budgets, rate limits, and audit declarations.
    pub clauses: Vec<PolicyClause>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum PolicyClause {
    /// `allow tool kb.search, tickets.get [when <cond>]`
    /// `deny  net  "*"`
    Rule {
        action: PolicyAction,
        /// Effect kind: `tool` | `net` | `fs` | `llm` | `memory` | `io`.
        effect: String,
        /// One or more target patterns (dotted idents or string globs).
        patterns: Vec<String>,
        /// Raw text of an optional `when <cond>` guard. Evaluated by the
        /// host at the call site; `None` means "always applies".
        when: Option<String>,
    },
    /// `budget per_request { usd = 0.50, tokens = 60_000 }`
    Budget {
        scope: String,
        usd_cents: Option<i64>,
        tokens: Option<i64>,
    },
    /// `rate per_user { 30 per 1m }`
    Rate {
        scope: String,
        max_calls: u32,
        window_secs: u64,
    },
    /// `audit all_tool_calls, all_policy_denials`
    Audit(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyAction {
    Allow,
    Deny,
}

#[derive(Clone, Debug)]
pub struct MemPolicyDecl {
    pub name: Ident,
    pub rules: Vec<RawRule>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct RawRule {
    pub raw: String,
    pub span: Span,
}

// ===========================================================================
// Models, tools, memory, prompts
// ===========================================================================

#[derive(Clone, Debug)]
pub struct ModelDecl {
    pub name: Ident,
    pub call: Expr,
    pub settings: Vec<(Ident, Expr)>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ToolDecl {
    pub doc: Option<String>,
    pub name: Ident,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub effect_row: Option<EffectRow>,
    pub attrs: Vec<Attribute>,
    pub body: ToolBody,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ToolBody {
    Block(Block),
    /// `extern "abi" "symbol"`.
    Extern { abi: String, symbol: String },
}

#[derive(Clone, Debug)]
pub struct MemoryDecl {
    pub name: Ident,
    pub call: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct PromptDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub slots: Vec<PromptSlot>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct PromptSlot {
    /// `system`, `user`, `memory`, etc. None if this slot was a bare string
    /// (which the spec treats as a `system:` slot).
    pub label: Option<Ident>,
    pub value: Expr,
    pub span: Span,
}

// ===========================================================================
// Traits, impls, consts, effects, tests, evals, config
// ===========================================================================

#[derive(Clone, Debug)]
pub struct TraitDecl {
    pub vis: Visibility,
    pub name: Ident,
    pub generics: Generics,
    pub items: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ImplBlock {
    pub generics: Generics,
    pub trait_path: Option<Path>,
    pub target: Type,
    pub items: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub vis: Visibility,
    pub name: Ident,
    pub ty: Option<Type>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EffectDecl {
    pub vis: Visibility,
    pub name: Ident,
    pub parents: Vec<Path>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TestDecl {
    pub name: String,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EvalDecl {
    pub name: String,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ConfigDecl {
    pub name: Ident,
    pub fields: Vec<Field>,
    pub span: Span,
}

// ===========================================================================
// Expressions
// ===========================================================================

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: Box<ExprKind>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Literal(Literal),
    Path(Path),
    /// `self`.
    SelfExpr,
    /// `nil`.
    Nil,
    /// `()`.
    UnitLit,
    /// `(a, b, c)`.
    Tuple(Vec<Expr>),
    /// `[a, b, c]`.
    ListLit(Vec<Expr>),
    /// `{k: v, ...}` or `{...}` for a set / record / map (parser disambiguates).
    BraceLit(BraceLit),
    Call {
        callee: Expr,
        args: Vec<CallArg>,
    },
    MethodCall {
        receiver: Expr,
        method: Ident,
        generics: Vec<Type>,
        args: Vec<CallArg>,
    },
    Field {
        receiver: Expr,
        name: Ident,
    },
    /// `receiver?.name` — nil-safe field access. Evaluates to `nil` when
    /// `receiver` is `nil`, otherwise behaves like `Field`.
    SafeField {
        receiver: Expr,
        name: Ident,
    },
    Index {
        receiver: Expr,
        index: Expr,
    },
    /// `expr await`.
    Await(Expr),
    /// `expr?`.
    Try(Expr),
    /// `try { ... } recover |e| { ... }` — run `body`; if it raises a
    /// runtime error, bind the error message to the recover lambda's
    /// single parameter and evaluate the recover branch. The value of
    /// the whole expression is the body's value on success, or the
    /// recover branch's value on failure.
    TryRecover {
        body: Block,
        recover: LambdaExpr,
    },
    /// `expr!`.
    Force(Expr),
    /// `spawn call`.
    Spawn(Expr),
    Block(Block),
    If {
        cond: Expr,
        then_branch: Block,
        else_branch: Option<Box<ExprOrBlock>>,
    },
    Match {
        scrutinee: Expr,
        arms: Vec<MatchArm>,
    },
    When {
        cond: Expr,
        then_branch: Block,
    },
    For {
        pat: Pattern,
        iter: Expr,
        body: Block,
        /// `true` for `for await pat in stream { ... }`. The runtime
        /// dispatches to the async-stream-aware iterator when set.
        is_await: bool,
    },
    While {
        cond: Expr,
        body: Block,
    },
    Select(Vec<SelectArm>),
    /// `parallel { ask m1 { ... }, ask m2 { ... }, ... }` — Stage 36.
    ///
    /// Each arm is restricted to a single `ask` expression (the only shape
    /// that does not require `Interpreter: Send`). The runtime dispatches
    /// all arms concurrently and joins in input order. Stage 37 will lift
    /// the arm restriction. Stage 36's `eval_parallel` enforces the shape
    /// with a clear error message naming the limitation.
    Parallel(Vec<Expr>),
    Ask {
        target: Expr,
        slots: Vec<PromptSlot>,
    },
    Generate {
        is_gen_shorthand: bool,
        schema: Type,
        model: Expr,
        prompt: Expr,
        extra: Vec<CallArg>,
    },
    Plan {
        target: Expr,
        slots: Vec<PromptSlot>,
    },
    Stream {
        item_type: Option<Type>,
        body: Block,
    },
    With {
        head: WithHead,
        body: Block,
        on_exceeded: Option<LambdaExpr>,
    },
    Lambda(LambdaExpr),
    Binary {
        op: BinOp,
        lhs: Expr,
        rhs: Expr,
    },
    Unary {
        op: UnOp,
        operand: Expr,
    },
    /// `lhs |> rhs`.
    Pipeline {
        lhs: Expr,
        rhs: Expr,
    },
    /// `expr as Type`.
    Cast {
        expr: Expr,
        ty: Type,
    },
    /// `expr is Type` / `expr is Pattern`.
    Is {
        expr: Expr,
        target: IsTarget,
    },
    /// `return [expr]`, `break [label]`, `continue [label]`, `yield expr`.
    Return(Option<Expr>),
    Break(Option<Ident>),
    Continue(Option<Ident>),
    Yield(Expr),
    /// `defer expr` — only valid in statement position; we still model as expr.
    Defer(Expr),
    /// String-with-interpolation as an expression.
    StringExpr(Vec<StringPart>),
}

#[derive(Clone, Debug)]
pub enum ExprOrBlock {
    Expr(Expr),
    Block(Block),
}

#[derive(Clone, Debug)]
pub enum CallArg {
    Positional(Expr),
    Named { name: Ident, value: Expr },
}

#[derive(Clone, Debug)]
pub struct LambdaExpr {
    pub params: Vec<Ident>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum WithHead {
    Budget(Vec<CallArg>),
    Recording(Expr),
    Scope(Ident),
    Span(Vec<CallArg>),
}

#[derive(Clone, Debug)]
pub enum BraceLit {
    /// `{}` — empty (treated as empty map by convention).
    Empty,
    /// `{a, b, c}` — set literal.
    Set(Vec<Expr>),
    /// `{k: v, ...}` — map literal.
    Map(Vec<(Expr, Expr)>),
    /// `{ field: expr, ... }` — record literal. The key is an identifier.
    Record(Vec<(Ident, Expr)>),
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct SelectArm {
    pub kind: SelectArmKind,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum SelectArmKind {
    /// `name = <-channel_expr => body` — receive from a channel.
    /// `binding` may be `_` for a discard.
    Recv { binding: Ident, channel: Expr },
    /// `_ = timeout(duration_expr) => body` — fires when no channel arm is
    /// ready.
    Timeout { duration: Expr },
    /// `else => body` — fires when no channel arm is ready and no
    /// `timeout(...)` arm is present. At most one allowed per `select`.
    Else,
}

#[derive(Clone, Debug)]
pub enum IsTarget {
    Type(Type),
    Pattern(Pattern),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    RemAssign,
    Range,
    RangeInclusive,
    /// `??` — null-coalescing: `a ?? b` is `a` if `a` is non-nil, else `b`.
    Coalesce,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
    Ref,
    RefMut,
}

// ===========================================================================
// Statements & blocks
// ===========================================================================

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// Trailing expression with no semicolon — the block's value.
    pub tail: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        pattern: Pattern,
        ty: Option<Type>,
        value: Expr,
        span: Span,
    },
    Var {
        name: Ident,
        ty: Option<Type>,
        value: Expr,
        span: Span,
    },
    Expr(Expr),
}

// ===========================================================================
// Patterns
// ===========================================================================

#[derive(Clone, Debug)]
pub struct Pattern {
    pub kind: Box<PatternKind>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum PatternKind {
    Literal(Literal),
    /// `_`.
    Wildcard,
    /// `name` — binds.
    Binding(Ident),
    /// `Path(p1, p2, ...)` — constructor / variant pattern.
    Constructor { path: Path, fields: Vec<Pattern> },
    /// `{field: pat, field2}` — record pattern. Identifier-only entries are
    /// shorthand for `field: field`.
    Record(Vec<FieldPattern>),
    /// `[p1, p2, ...]`.
    List(Vec<Pattern>),
    /// `(p1, p2, ...)`.
    Tuple(Vec<Pattern>),
    /// `name @ inner`.
    Binder { name: Ident, inner: Pattern },
    /// `a | b`.
    Or(Pattern, Pattern),
}

#[derive(Clone, Debug)]
pub struct FieldPattern {
    pub name: Ident,
    pub pattern: Option<Pattern>,
    pub span: Span,
}

// ===========================================================================
// Literals (mirrors lexer payloads but as AST values)
// ===========================================================================

#[derive(Clone, Debug)]
pub enum Literal {
    Int { value: i128 },
    Float { lexeme: String },
    Decimal { lexeme: String },
    Money { amount: String, currency: String },
    Duration { nanos: i128, original: String },
    Date { y: u16, m: u8, d: u8 },
    DateTime { y: u16, m: u8, d: u8, hh: u8, mm: u8, ss: u8, utc: bool },
    Time { hh: u8, mm: u8, ss: u8 },
    Bool(bool),
    Char(char),
    /// A complete string literal; if it contained `{expr}` interpolation the
    /// parser keeps each interpolated expression alongside the surrounding
    /// text parts (so we never lose the value-vs-text distinction).
    String { kind: StringLitKind, parts: Vec<StringPart> },
    HashLit { algo: String, hex: String },
    AgentAddr { is_dynamic: bool, text: String },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StringLitKind {
    Regular,
    Bytes,
    Raw,
    MultiLine,
    Prompt,
}

#[derive(Clone, Debug)]
pub enum StringPart {
    Text(String),
    Interp(Expr),
}
