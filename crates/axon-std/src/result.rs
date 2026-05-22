//! `std.result` — first-order Result operations.
//!
//! `Result<T, E>` is modeled as a tagged `Value::Instance`:
//!
//!   * `Ok(v)`  → `Instance { type_name: "Result", variant: "Ok",  fields: [("0", v)] }`
//!   * `Err(e)` → `Instance { type_name: "Result", variant: "Err", fields: [("0", e)] }`
//!
//! This matches how user-defined sum types are already represented in the
//! runtime, so pattern matching against `Result.Ok(v)` works uniformly.

use std::cell::RefCell;
use std::rc::Rc;

use axon_runtime::{NativeFn, Value};

pub const COUNT: usize = 7;

pub(crate) fn register(reg: &mut dyn FnMut(&'static str, NativeFn)) {
    reg("result_ok", n("result_ok", 1, Some(1), result_ok));
    reg("result_err", n("result_err", 1, Some(1), result_err));
    reg("result_is_ok", n("result_is_ok", 1, Some(1), result_is_ok));
    reg("result_is_err", n("result_is_err", 1, Some(1), result_is_err));
    reg("result_unwrap_or", n("result_unwrap_or", 2, Some(2), result_unwrap_or));
    reg("result_value", n("result_value", 1, Some(1), result_value));
    reg("result_error", n("result_error", 1, Some(1), result_error));
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

fn make(variant: &'static str, inner: Value) -> Value {
    Value::Instance {
        type_name: Rc::new("Result".to_string()),
        variant: Some(Rc::new(variant.to_string())),
        fields: Rc::new(RefCell::new(vec![("0".to_string(), inner)])),
    }
}

fn is_variant(v: &Value, want: &str) -> bool {
    match v {
        Value::Instance {
            type_name,
            variant: Some(var),
            ..
        } => type_name.as_str() == "Result" && var.as_str() == want,
        _ => false,
    }
}

fn unwrap_inner(v: &Value) -> Option<Value> {
    if let Value::Instance { fields, .. } = v {
        let f = fields.borrow();
        f.first().map(|(_, val)| val.clone())
    } else {
        None
    }
}

fn result_ok(args: &[Value]) -> Result<Value, String> {
    Ok(make("Ok", args[0].clone()))
}

fn result_err(args: &[Value]) -> Result<Value, String> {
    Ok(make("Err", args[0].clone()))
}

fn result_is_ok(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(is_variant(&args[0], "Ok")))
}

fn result_is_err(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(is_variant(&args[0], "Err")))
}

fn result_unwrap_or(args: &[Value]) -> Result<Value, String> {
    if is_variant(&args[0], "Ok") {
        unwrap_inner(&args[0]).ok_or_else(|| "result_unwrap_or: malformed Result".to_string())
    } else {
        Ok(args[1].clone())
    }
}

fn result_value(args: &[Value]) -> Result<Value, String> {
    if is_variant(&args[0], "Ok") {
        unwrap_inner(&args[0]).ok_or_else(|| "result_value: malformed Result".to_string())
    } else {
        Ok(Value::Nil)
    }
}

fn result_error(args: &[Value]) -> Result<Value, String> {
    if is_variant(&args[0], "Err") {
        unwrap_inner(&args[0]).ok_or_else(|| "result_error: malformed Result".to_string())
    } else {
        Ok(Value::Nil)
    }
}
