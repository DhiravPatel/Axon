//! Function-attribute parsing & runtime semantics.
//!
//! Lifted from the AST `Vec<Attribute>` so the eval loop has a typed,
//! easy-to-test view of behavior modifiers. Stage 18 implements:
//!
//!   * `@retry(times = N, backoff_ms = M)` — re-invoke on a runtime
//!     panic up to `times` extra attempts with a fixed-millisecond
//!     backoff between attempts. Final failure surfaces as the
//!     original error.
//!   * `@memoize(ttl_ms = M)` — per-closure result cache keyed by the
//!     argument tuple's `Value::fmt` printout. TTL is optional; `None`
//!     means "cache forever for this process".
//!   * `@deadline(ms = N)` — wall-clock budget. Currently enforced as
//!     a *post-call* check: if `elapsed > deadline`, the call returns
//!     a `deadline exceeded` error. (A true mid-call abort needs
//!     scheduler integration — Stage 19's `select`/`for await` work.)
//!   * `@idempotent` — pure metadata bit consumed by supervisors and
//!     the replay engine; doesn't change call semantics.
//!
//! The attribute reader is intentionally forgiving: unknown attribute
//! names are ignored (treated as future-compatible markers), and an
//! attribute with malformed arguments produces a typed error that the
//! caller can choose to surface as a lint instead of a hard failure.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use axon_ast::Attribute;

#[derive(Clone, Debug, Default)]
pub struct CallPolicy {
    pub retry: Option<RetryPolicy>,
    pub memoize: Option<MemoizePolicy>,
    pub deadline_ms: Option<u64>,
    pub idempotent: bool,
}

impl CallPolicy {
    /// Is this policy a no-op? `true` when nothing was set — call sites
    /// fast-path the un-attributed case.
    pub fn is_default(&self) -> bool {
        self.retry.is_none()
            && self.memoize.is_none()
            && self.deadline_ms.is_none()
            && !self.idempotent
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    pub times: u32,
    pub backoff_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct MemoizePolicy {
    /// `None` → cache forever; `Some(d)` → entries expire after `d`.
    pub ttl: Option<Duration>,
    /// One slot per closure; shared across calls of the *same* closure
    /// value. Keyed by the formatted argument tuple.
    pub cache: std::rc::Rc<RefCell<HashMap<String, CacheEntry>>>,
}

impl PartialEq for MemoizePolicy {
    fn eq(&self, other: &Self) -> bool {
        self.ttl == other.ttl && std::rc::Rc::ptr_eq(&self.cache, &other.cache)
    }
}
impl Eq for MemoizePolicy {}

#[derive(Clone)]
pub struct CacheEntry {
    pub value: crate::value::Value,
    pub stored_at_ns: i64,
}

impl std::fmt::Debug for CacheEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheEntry")
            .field("stored_at_ns", &self.stored_at_ns)
            .finish_non_exhaustive()
    }
}

/// Parse the AST attribute list. Returns `(policy, warnings)` so the
/// caller can decide whether to surface unknown/bad-arg attributes.
pub fn parse_attrs(attrs: &[Attribute]) -> (CallPolicy, Vec<String>) {
    let mut p = CallPolicy::default();
    let mut warnings = Vec::new();
    for a in attrs {
        let name = a.name.segments.last().map(|s| s.name.as_str()).unwrap_or("");
        match name {
            "retry" => match read_retry(&a.args) {
                Ok(r) => p.retry = Some(r),
                Err(e) => warnings.push(format!("@retry: {e}")),
            },
            "memoize" => match read_memoize(&a.args) {
                Ok(m) => p.memoize = Some(m),
                Err(e) => warnings.push(format!("@memoize: {e}")),
            },
            "deadline" => match read_deadline(&a.args) {
                Ok(d) => p.deadline_ms = Some(d),
                Err(e) => warnings.push(format!("@deadline: {e}")),
            },
            "idempotent" => p.idempotent = true,
            "pure" | "test" | "desc" | "redact" => {} // known non-policy markers
            _ => {} // forward-compat: unknown attrs ignored
        }
    }
    (p, warnings)
}

// The attribute parser stores arg *values* but discards keyword names, so
// `@retry(times = 3)` and `@retry(3)` produce the same arg list here.
// Argument order: first arg = primary, second = secondary.

fn read_retry(args: &[axon_ast::Expr]) -> Result<RetryPolicy, String> {
    let times = match args.first() {
        Some(e) => u_int(e, "times")? as u32,
        None => return Err("expected at least one Int arg (times)".into()),
    };
    let backoff_ms = match args.get(1) {
        Some(e) => u_int(e, "backoff_ms")?,
        None => 0,
    };
    if times == 0 {
        return Err("times must be ≥ 1".into());
    }
    Ok(RetryPolicy { times, backoff_ms })
}

fn read_memoize(args: &[axon_ast::Expr]) -> Result<MemoizePolicy, String> {
    let ttl = match args.first() {
        Some(e) => Some(Duration::from_millis(u_int(e, "ttl_ms")?)),
        None => None,
    };
    Ok(MemoizePolicy {
        ttl,
        cache: std::rc::Rc::new(RefCell::new(HashMap::new())),
    })
}

fn read_deadline(args: &[axon_ast::Expr]) -> Result<u64, String> {
    match args.first() {
        Some(e) => u_int(e, "ms"),
        None => Err("missing required Int arg (ms)".into()),
    }
}

fn u_int(e: &axon_ast::Expr, label: &str) -> Result<u64, String> {
    match e.kind.as_ref() {
        axon_ast::ExprKind::Literal(axon_ast::Literal::Int { value }) if *value >= 0 => {
            Ok(*value as u64)
        }
        _ => Err(format!("`{label}` must be a non-negative Int")),
    }
}
