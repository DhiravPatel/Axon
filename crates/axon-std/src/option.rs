//! `std.option` — first-order Option operations.
//!
//! The v0 runtime models `Option<T>` as a nullable value: `None` is `Nil`
//! and `Some(v)` is `v` itself. Higher-order map/and_then land when native
//! callbacks can re-enter the interpreter (Stage 12+).

use axon_runtime::{NativeFn, Value};

pub const COUNT: usize = 6;

pub(crate) fn register(reg: &mut dyn FnMut(&'static str, NativeFn)) {
    reg("opt_some", n("opt_some", 1, Some(1), opt_some));
    reg("opt_none", n("opt_none", 0, Some(0), opt_none));
    reg("opt_is_some", n("opt_is_some", 1, Some(1), opt_is_some));
    reg("opt_is_none", n("opt_is_none", 1, Some(1), opt_is_none));
    reg("opt_unwrap_or", n("opt_unwrap_or", 2, Some(2), opt_unwrap_or));
    reg("opt_or", n("opt_or", 2, Some(2), opt_or));
}

fn n(
    name: &'static str,
    min_arity: usize,
    max_arity: Option<usize>,
    call: fn(&[Value]) -> Result<Value, String>,
) -> NativeFn {
    NativeFn {
        name,
        min_arity,
        max_arity,
        required_caps: &[],
        call,
    }
}

fn opt_some(args: &[Value]) -> Result<Value, String> {
    Ok(args[0].clone())
}

fn opt_none(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::Nil)
}

fn opt_is_some(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(!matches!(args[0], Value::Nil)))
}

fn opt_is_none(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(matches!(args[0], Value::Nil)))
}

fn opt_unwrap_or(args: &[Value]) -> Result<Value, String> {
    Ok(if matches!(args[0], Value::Nil) {
        args[1].clone()
    } else {
        args[0].clone()
    })
}

fn opt_or(args: &[Value]) -> Result<Value, String> {
    Ok(if matches!(args[0], Value::Nil) {
        args[1].clone()
    } else {
        args[0].clone()
    })
}
