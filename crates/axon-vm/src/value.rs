//! Runtime values for the AxVM.
//!
//! Structurally mirrors `axon_runtime::Value` so a program's observable
//! behavior is identical under either backend, but the two enums are kept
//! independent so the VM and tree-walker can evolve their representations
//! separately (e.g. the VM might later switch to a NaN-boxed tagged-pointer
//! representation while the tree-walker stays simple).

use std::cell::RefCell;
use std::cmp::Ordering;
use std::fmt;
use std::rc::Rc;

use axon_diag::Span;

use crate::ops::Function;

/// A runtime value visible from Axon source.
#[derive(Clone)]
pub enum Value {
    Unit,
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Char(char),
    String(Rc<String>),
    Bytes(Rc<Vec<u8>>),
    Decimal(Rc<String>),
    Money {
        amount: Rc<String>,
        currency: Rc<String>,
    },
    Duration(i64),
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
        utc: bool,
    },
    Time {
        hh: u8,
        mm: u8,
        ss: u8,
    },
    ContentHash {
        algo: Rc<String>,
        hex: Rc<String>,
    },
    AgentAddr {
        is_dynamic: bool,
        text: Rc<String>,
    },

    List(Rc<RefCell<Vec<Value>>>),
    Map(Rc<RefCell<Vec<(Value, Value)>>>),
    Set(Rc<RefCell<Vec<Value>>>),
    Tuple(Rc<Vec<Value>>),
    Record(Rc<RefCell<Vec<(String, Value)>>>),
    Tainted(Rc<Value>),

    /// A compiled closure: a function chunk plus captured upvalue cells.
    /// `upvalues[i]` corresponds to `function.upvalues[i]`.
    Closure(Rc<Closure>),

    /// A native (Rust) callable.
    Native(Rc<NativeFn>),
}

pub struct Closure {
    pub function: Rc<Function>,
    pub upvalues: Vec<Rc<RefCell<Value>>>,
}

/// Identical shape to `axon_runtime::NativeFn`; kept here so the VM has no
/// runtime dep and can run standalone. Built-ins are registered twice — once
/// in each backend — but the dispatch contract is the same.
pub struct NativeFn {
    pub name: &'static str,
    pub min_arity: usize,
    pub max_arity: Option<usize>,
    pub required_caps: &'static [&'static str],
    pub call: fn(&[Value]) -> Result<Value, String>,
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        use Value::*;
        match self {
            Unit => "Unit",
            Nil => "Nil",
            Bool(_) => "Bool",
            Int(_) => "Int",
            Float(_) => "Float",
            Char(_) => "Char",
            String(_) => "String",
            Bytes(_) => "Bytes",
            Decimal(_) => "Decimal",
            Money { .. } => "Money",
            Duration(_) => "Duration",
            Date { .. } => "Date",
            DateTime { .. } => "DateTime",
            Time { .. } => "Time",
            ContentHash { .. } => "ContentHash",
            AgentAddr { .. } => "AgentAddr",
            List(_) => "List",
            Map(_) => "Map",
            Set(_) => "Set",
            Tuple(_) => "Tuple",
            Record(_) => "Record",
            Tainted(_) => "Tainted",
            Closure(_) => "Fn",
            Native(_) => "Native",
        }
    }

    /// Strict truthiness — only `Bool(true)` is true. See the matching note
    /// in `axon_runtime::Value::is_truthy`.
    pub fn is_truthy(&self) -> bool {
        matches!(self, Value::Bool(true))
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (self, other) {
            (Unit, Unit) | (Nil, Nil) => true,
            (Bool(a), Bool(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Int(a), Float(b)) | (Float(b), Int(a)) => (*a as f64) == *b,
            (Char(a), Char(b)) => a == b,
            (String(a), String(b)) => a == b,
            (Bytes(a), Bytes(b)) => a == b,
            (Decimal(a), Decimal(b)) => a == b,
            (
                Money { amount: a1, currency: c1 },
                Money { amount: a2, currency: c2 },
            ) => a1 == a2 && c1 == c2,
            (Duration(a), Duration(b)) => a == b,
            (Date { y: y1, m: m1, d: d1 }, Date { y: y2, m: m2, d: d2 }) => {
                y1 == y2 && m1 == m2 && d1 == d2
            }
            (
                Time { hh: h1, mm: m1, ss: s1 },
                Time { hh: h2, mm: m2, ss: s2 },
            ) => h1 == h2 && m1 == m2 && s1 == s2,
            (
                AgentAddr { is_dynamic: d1, text: t1 },
                AgentAddr { is_dynamic: d2, text: t2 },
            ) => d1 == d2 && t1 == t2,
            (
                ContentHash { algo: a1, hex: h1 },
                ContentHash { algo: a2, hex: h2 },
            ) => a1 == a2 && h1 == h2,
            (List(a), List(b)) => Rc::ptr_eq(a, b) || *a.borrow() == *b.borrow(),
            (Map(a), Map(b)) => Rc::ptr_eq(a, b) || *a.borrow() == *b.borrow(),
            (Set(a), Set(b)) => Rc::ptr_eq(a, b) || *a.borrow() == *b.borrow(),
            (Tuple(a), Tuple(b)) => Rc::ptr_eq(a, b) || **a == **b,
            (Record(a), Record(b)) => Rc::ptr_eq(a, b) || *a.borrow() == *b.borrow(),
            (Tainted(a), Tainted(b)) => a == b,
            (Closure(a), Closure(b)) => Rc::ptr_eq(a, b),
            (Native(a), Native(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Value {
    pub fn cmp(&self, other: &Self) -> Option<Ordering> {
        use Value::*;
        Some(match (self, other) {
            (Int(a), Int(b)) => a.cmp(b),
            (Float(a), Float(b)) => a.partial_cmp(b)?,
            (Int(a), Float(b)) => (*a as f64).partial_cmp(b)?,
            (Float(a), Int(b)) => a.partial_cmp(&(*b as f64))?,
            (String(a), String(b)) => a.as_str().cmp(b.as_str()),
            (Char(a), Char(b)) => a.cmp(b),
            (Bool(a), Bool(b)) => a.cmp(b),
            (Duration(a), Duration(b)) => a.cmp(b),
            (Date { y: y1, m: m1, d: d1 }, Date { y: y2, m: m2, d: d2 }) => {
                (y1, m1, d1).cmp(&(y2, m2, d2))
            }
            (
                Time { hh: h1, mm: m1, ss: s1 },
                Time { hh: h2, mm: m2, ss: s2 },
            ) => (h1, m1, s1).cmp(&(h2, m2, s2)),
            _ => return None,
        })
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Value::*;
        match self {
            Unit => f.write_str("()"),
            Nil => f.write_str("nil"),
            Bool(b) => write!(f, "{b}"),
            Int(i) => write!(f, "{i}"),
            Float(x) => write!(f, "{x}"),
            Char(c) => write!(f, "'{c}'"),
            String(s) => f.write_str(s),
            Bytes(b) => {
                f.write_str("b\"")?;
                for byte in b.iter() {
                    if byte.is_ascii_graphic() || *byte == b' ' {
                        write!(f, "{}", *byte as char)?;
                    } else {
                        write!(f, "\\x{byte:02x}")?;
                    }
                }
                f.write_str("\"")
            }
            Decimal(s) => write!(f, "{s}dec"),
            Money { amount, currency } => write!(f, "{amount}{currency}"),
            Duration(n) => {
                let ns = *n;
                if ns % 86_400_000_000_000 == 0 {
                    write!(f, "{}d", ns / 86_400_000_000_000)
                } else if ns % 3_600_000_000_000 == 0 {
                    write!(f, "{}h", ns / 3_600_000_000_000)
                } else if ns % 60_000_000_000 == 0 {
                    write!(f, "{}m", ns / 60_000_000_000)
                } else if ns % 1_000_000_000 == 0 {
                    write!(f, "{}s", ns / 1_000_000_000)
                } else if ns % 1_000_000 == 0 {
                    write!(f, "{}ms", ns / 1_000_000)
                } else {
                    write!(f, "{ns}ns")
                }
            }
            Date { y, m, d } => write!(f, "{y:04}-{m:02}-{d:02}"),
            DateTime { y, m, d, hh, mm, ss, utc } => {
                write!(f, "{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}")?;
                if *utc {
                    f.write_str("Z")?;
                }
                Ok(())
            }
            Time { hh, mm, ss } => write!(f, "{hh:02}:{mm:02}:{ss:02}"),
            ContentHash { algo, hex } => write!(f, "#{algo}:{hex}"),
            AgentAddr { is_dynamic, text } => {
                if *is_dynamic {
                    write!(f, "@{{{text}}}")
                } else {
                    write!(f, "@{text}")
                }
            }
            List(xs) => {
                f.write_str("[")?;
                let xs = xs.borrow();
                for (i, v) in xs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("]")
            }
            Set(xs) => {
                f.write_str("{")?;
                let xs = xs.borrow();
                for (i, v) in xs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("}")
            }
            Map(entries) => {
                f.write_str("{")?;
                let entries = entries.borrow();
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                f.write_str("}")
            }
            Tuple(xs) => {
                f.write_str("(")?;
                for (i, v) in xs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                if xs.len() == 1 {
                    f.write_str(",")?;
                }
                f.write_str(")")
            }
            Record(fields) => {
                f.write_str("{")?;
                let fields = fields.borrow();
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                f.write_str("}")
            }
            Tainted(inner) => write!(f, "Tainted({inner})"),
            Closure(c) => match &c.function.name {
                Some(n) => write!(f, "<fn {n}>"),
                None => f.write_str("<lambda>"),
            },
            Native(n) => write!(f, "<native {}>", n.name),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Source span associated with the most recent value-producing op. Used to
/// attribute runtime errors back to the right place in source.
#[derive(Copy, Clone, Debug, Default)]
pub struct SpanRef(pub Span);
