//! Canonical type representation for the Axon compiler.
//!
//! The AST has a *syntactic* `axon_ast::Type` that mirrors the source. This
//! crate is the *semantic* layer: a normalized [`Ty`] enum the type checker
//! actually compares and unifies. Lowering AST→Ty happens in `axon-tyck`.
//!
//! Design notes:
//!
//! * Types are owned, recursively constructed values. We do *not* intern.
//!   Most Axon types are small (a couple of pointer-sized words) and the
//!   workload is dominated by parsing/checking the user's items, not
//!   pushing types through tight inner loops. If we measure this becomes a
//!   bottleneck, swap to an arena + `TyId` without changing the public API.
//!
//! * [`Ty::Var`] is reserved for the unification path the inferencer uses.
//!   v0 of the checker is monomorphic at call sites, so variables only show
//!   up inside `fn`/`agent` signatures with declared generic parameters.

use std::fmt;

use axon_diag::Span;

// ---------------------------------------------------------------------------
// Item identity
// ---------------------------------------------------------------------------

/// Stable, dense index for a named top-level item (type / agent / fn / ...).
///
/// The checker registers every item it sees in a flat `Vec<ItemSig>` keyed by
/// `ItemId`. We never *delete* items, so the id remains a meaningful handle
/// for the rest of the pipeline (IR lowering, codegen, debug info).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ItemId(pub u32);

/// Free type variable used during unification.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TyVarId(pub u32);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    // --- Primitives ---------------------------------------------------
    Int,
    Float,
    Decimal,
    Money,
    Duration,
    Date,
    DateTime,
    Time,
    Bool,
    Char,
    String,
    Bytes,
    Unit,
    /// The empty type — inhabits no values. `return`/`break`/`continue` and
    /// diverging calls have type `Never`. `Never <: T` for every `T`.
    Never,
    /// The gradual-typing bridge: every static type is a subtype of `Dyn`
    /// and crossing the boundary inserts a runtime contract check.
    Dyn,

    // --- Built-in parametric -----------------------------------------
    Option(Box<Ty>),
    List(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    Set(Box<Ty>),
    Tuple(Vec<Ty>),
    Ref {
        mutable: bool,
        inner: Box<Ty>,
    },

    /// Untrusted data. Distinct from the inner type — `Tainted<String>` and
    /// `String` do *not* unify (§10.6). A sanitizer transition produces the
    /// plain inner type.
    Tainted(Box<Ty>),

    /// `T?` — Option<T> spelled differently in source.
    ///
    /// Kept separate from `Option<T>` only for diagnostic fidelity. They are
    /// considered equal for unification purposes; canonicalization in the
    /// checker collapses the two at construction time.
    Nullable(Box<Ty>),

    // --- Function and effect-carrying function ------------------------
    Fn {
        params: Vec<Ty>,
        ret: Box<Ty>,
        effects: EffectRow,
    },

    // --- User-defined names -------------------------------------------
    /// A reference to a registered top-level item. `args` are concrete type
    /// arguments at the use site — empty for non-generic items.
    Named { id: ItemId, args: Vec<Ty> },

    /// A bound generic parameter inside a declaration scope. Distinct from
    /// `Var` (which is for inference) because parameters are *rigid*.
    Param(ParamId),

    // --- Domain types -------------------------------------------------
    /// Handle to an agent of a particular declared shape.
    AgentHandle(ItemId),
    ActorHandle(ItemId),
    Model,
    /// `Tool<Input, Output>`.
    Tool(Box<Ty>, Box<Ty>),
    Memory,
    Stream(Box<Ty>),
    Chan(Box<Ty>),
    Secret(Box<Ty>),
    /// A content hash literal (`#sha256:...`). Distinct value type so it
    /// can't be confused with an arbitrary string.
    ContentHash,
    /// An agent-address literal (`@alice` / `@{...}`). Held as a separate
    /// nominal type so addresses can be passed through APIs without losing
    /// their nature.
    AgentAddr,

    // --- Anonymous union ----------------------------------------------
    /// `T | U` as written in source. Unification on unions is conservative
    /// in v0: a union unifies with another type only if some member does.
    Union(Vec<Ty>),

    // --- Unification machinery ----------------------------------------
    Var(TyVarId),

    /// Placeholder when checking has already failed for this expression.
    /// Used to suppress cascading errors.
    Error,
}

/// Identifier for a rigid type parameter introduced by `fn f<T>(...)`,
/// `agent A<T>(...)`, etc. The name lives in the surrounding item's scope.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParamId(pub u32);

impl Ty {
    pub fn is_error(&self) -> bool {
        matches!(self, Ty::Error)
    }

    /// True if `self` is `Never`.
    pub fn is_never(&self) -> bool {
        matches!(self, Ty::Never)
    }

    /// True if any unification variables remain in `self`.
    pub fn has_vars(&self) -> bool {
        match self {
            Ty::Var(_) => true,
            Ty::Option(t) | Ty::List(t) | Ty::Set(t) | Ty::Nullable(t) | Ty::Tainted(t)
            | Ty::Stream(t) | Ty::Chan(t) | Ty::Secret(t) | Ty::Ref { inner: t, .. } => {
                t.has_vars()
            }
            Ty::Map(k, v) | Ty::Tool(k, v) => k.has_vars() || v.has_vars(),
            Ty::Tuple(xs) | Ty::Union(xs) => xs.iter().any(Self::has_vars),
            Ty::Fn { params, ret, .. } => {
                params.iter().any(Self::has_vars) || ret.has_vars()
            }
            Ty::Named { args, .. } => args.iter().any(Self::has_vars),
            _ => false,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int => f.write_str("Int"),
            Ty::Float => f.write_str("Float"),
            Ty::Decimal => f.write_str("Decimal"),
            Ty::Money => f.write_str("Money"),
            Ty::Duration => f.write_str("Duration"),
            Ty::Date => f.write_str("Date"),
            Ty::DateTime => f.write_str("DateTime"),
            Ty::Time => f.write_str("Time"),
            Ty::Bool => f.write_str("Bool"),
            Ty::Char => f.write_str("Char"),
            Ty::String => f.write_str("String"),
            Ty::Bytes => f.write_str("Bytes"),
            Ty::Unit => f.write_str("Unit"),
            Ty::Never => f.write_str("Never"),
            Ty::Dyn => f.write_str("dyn"),
            Ty::Option(t) => write!(f, "Option<{t}>"),
            Ty::Nullable(t) => write!(f, "{t}?"),
            Ty::List(t) => write!(f, "[{t}]"),
            Ty::Map(k, v) => write!(f, "{{{k}: {v}}}"),
            Ty::Set(t) => write!(f, "{{{t}}}"),
            Ty::Tuple(xs) => {
                f.write_str("(")?;
                for (i, t) in xs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{t}")?;
                }
                f.write_str(")")
            }
            Ty::Ref { mutable, inner } => {
                f.write_str(if *mutable { "&mut " } else { "&" })?;
                write!(f, "{inner}")
            }
            Ty::Tainted(t) => write!(f, "Tainted<{t}>"),
            Ty::Fn {
                params,
                ret,
                effects,
            } => {
                f.write_str("(")?;
                for (i, t) in params.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, ") -> {ret}")?;
                if !effects.is_pure() {
                    write!(f, " uses {effects}")?;
                }
                Ok(())
            }
            Ty::Named { id, args } => {
                write!(f, "<#{:?}>", id.0)?;
                if !args.is_empty() {
                    f.write_str("<")?;
                    for (i, t) in args.iter().enumerate() {
                        if i > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "{t}")?;
                    }
                    f.write_str(">")?;
                }
                Ok(())
            }
            Ty::Param(id) => write!(f, "<param#{}>", id.0),
            Ty::AgentHandle(id) => write!(f, "Agent<#{}>", id.0),
            Ty::ActorHandle(id) => write!(f, "Actor<#{}>", id.0),
            Ty::Model => f.write_str("Model"),
            Ty::Tool(i, o) => write!(f, "Tool<{i}, {o}>"),
            Ty::Memory => f.write_str("Memory"),
            Ty::Stream(t) => write!(f, "Stream<{t}>"),
            Ty::Chan(t) => write!(f, "Chan<{t}>"),
            Ty::Secret(t) => write!(f, "Secret<{t}>"),
            Ty::ContentHash => f.write_str("ContentHash"),
            Ty::AgentAddr => f.write_str("AgentAddr"),
            Ty::Union(xs) => {
                for (i, t) in xs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{t}")?;
                }
                Ok(())
            }
            Ty::Var(v) => write!(f, "?{}", v.0),
            Ty::Error => f.write_str("<error>"),
        }
    }
}

// ---------------------------------------------------------------------------
// Effect rows (§20)
// ---------------------------------------------------------------------------

/// An effect row tracks the side-effect "capabilities" a piece of code uses.
///
/// Concretely: a sorted, de-duplicated set of effect atoms plus an optional
/// trailing row variable for polymorphism (`fn f() -> T uses e`). The empty
/// row is the *pure* row; "any effect" is represented by `Dyn` and is only
/// produced when interop with the gradual layer requires it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct EffectRow {
    atoms: Vec<EffectAtom>,
    var: Option<TyVarId>,
}

/// One named effect, dotted form supported (`Fs.Read`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EffectAtom {
    pub name: String,
}

impl EffectRow {
    pub fn pure() -> Self {
        Self::default()
    }

    pub fn singleton(name: impl Into<String>) -> Self {
        Self {
            atoms: vec![EffectAtom { name: name.into() }],
            var: None,
        }
    }

    pub fn from_atoms<I: IntoIterator<Item = EffectAtom>>(it: I) -> Self {
        let mut atoms: Vec<EffectAtom> = it.into_iter().collect();
        atoms.sort();
        atoms.dedup();
        Self { atoms, var: None }
    }

    pub fn is_pure(&self) -> bool {
        self.atoms.is_empty() && self.var.is_none()
    }

    pub fn atoms(&self) -> &[EffectAtom] {
        &self.atoms
    }

    pub fn contains(&self, name: &str) -> bool {
        self.atoms.iter().any(|a| a.name == name)
    }

    /// Add a single effect. No-op if already present.
    pub fn add(&mut self, name: impl Into<String>) {
        let atom = EffectAtom { name: name.into() };
        if !self.atoms.contains(&atom) {
            self.atoms.push(atom);
            self.atoms.sort();
        }
    }

    /// Set-union of two rows.
    pub fn union(&self, other: &EffectRow) -> EffectRow {
        let mut out = self.atoms.clone();
        for a in &other.atoms {
            if !out.contains(a) {
                out.push(a.clone());
            }
        }
        out.sort();
        EffectRow {
            atoms: out,
            var: self.var.or(other.var),
        }
    }

    /// True iff every effect in `self` is also in `other`. Used to check
    /// "the inferred row is allowed by the declared row" at function bodies.
    pub fn subset_of(&self, other: &EffectRow) -> bool {
        self.atoms.iter().all(|a| other.contains(&a.name))
    }

    /// Effects in `self` that are *not* present in `allowed`. Used to build
    /// "effect not declared" diagnostics.
    pub fn difference(&self, allowed: &EffectRow) -> Vec<EffectAtom> {
        self.atoms
            .iter()
            .filter(|a| !allowed.contains(&a.name))
            .cloned()
            .collect()
    }
}

impl fmt::Display for EffectRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{")?;
        for (i, a) in self.atoms.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            f.write_str(&a.name)?;
        }
        if let Some(v) = self.var {
            if !self.atoms.is_empty() {
                f.write_str(", ")?;
            }
            write!(f, "?{}", v.0)?;
        }
        f.write_str("}")
    }
}

// ---------------------------------------------------------------------------
// Item signatures
// ---------------------------------------------------------------------------

/// A registered top-level item along with the metadata the checker needs.
#[derive(Clone, Debug)]
pub struct ItemSig {
    pub name: String,
    pub span: Span,
    pub kind: ItemSigKind,
    pub generics: Vec<GenericParam>,
}

#[derive(Clone, Debug)]
pub struct GenericParam {
    pub name: String,
    pub kind: GenericParamKind,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GenericParamKind {
    Type,
    Covariant,
    Contravariant,
    Effect,
}

#[derive(Clone, Debug)]
pub enum ItemSigKind {
    /// `fn name<...>(...) -> R uses {...}`.
    Fn(FnSig),
    /// Record type: ordered, named fields.
    Record(Vec<FieldSig>),
    /// Sum type: a fixed set of variants, possibly each with their own fields.
    Sum(Vec<VariantSig>),
    /// `type Foo = Bar` — an alias to another type.
    Alias(Ty),
    /// `type Email = String @nominal` — distinct nominal newtype.
    Newtype(Ty),
    /// `schema S { fields; migrations }`. Carries everything `Record` does
    /// plus a version number; migrations are stored in the checker's
    /// auxiliary data so we don't bloat this struct.
    Schema {
        version: Option<u32>,
        fields: Vec<FieldSig>,
    },
    /// `agent Name(...) { ... }` — exposes a constructor signature, the
    /// `state` field types (which become `self.<field>` lookups), and a
    /// table of message handlers.
    Agent {
        params: Vec<ParamSig>,
        state_fields: Vec<FieldSig>,
        handlers: Vec<HandlerSig>,
    },
    /// `actor Name(...) { ... }`.
    Actor {
        params: Vec<ParamSig>,
        state_fields: Vec<FieldSig>,
        handlers: Vec<HandlerSig>,
    },
    /// `const NAME: T = expr`.
    Const(Ty),
    /// `model name = call`. The bound name has type `Model`.
    Model,
    /// `tool name(...) -> T uses {...}`.
    Tool(FnSig),
    /// `memory name = call`.
    Memory,
    /// `prompt name(...) -> T { slots }`. Looks like a function returning a
    /// rendered prompt; we treat it as such for the purpose of typing.
    Prompt(FnSig),
    /// Items we recognize but don't fully type yet (supervisor / graph /
    /// network / orchestrate / policy / mempolicy / trait / impl / effect /
    /// test / eval / config). The checker still registers the name to avoid
    /// "duplicate definition" misses but doesn't synthesize a fn signature.
    Opaque,
}

#[derive(Clone, Debug)]
pub struct FnSig {
    pub params: Vec<ParamSig>,
    pub ret: Ty,
    pub effects: EffectRow,
}

#[derive(Clone, Debug)]
pub struct ParamSig {
    pub name: String,
    pub ty: Ty,
    pub has_default: bool,
}

#[derive(Clone, Debug)]
pub struct FieldSig {
    pub name: String,
    pub ty: Ty,
    pub has_default: bool,
    /// We carry the *names* of refinements so the checker / runtime can find
    /// them again. Their argument exprs live in the AST; the checker walks
    /// the AST when it needs them.
    pub refinements: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct VariantSig {
    pub name: String,
    pub fields: Vec<FieldSig>,
}

#[derive(Clone, Debug)]
pub struct HandlerSig {
    pub name: String,
    pub params: Vec<ParamSig>,
    pub ret: Ty,
    pub effects: EffectRow,
}
