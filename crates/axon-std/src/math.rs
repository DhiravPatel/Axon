//! `std.math` — numeric helpers.
//!
//! All functions accept Int or Float and (where it makes sense) return Float.
//! Int-only operations like `gcd` stay in the integer domain.

use axon_runtime::{NativeFn, Value};

pub const COUNT: usize = 14;

pub(crate) fn register(reg: &mut dyn FnMut(&'static str, NativeFn)) {
    reg("math_pow", n("math_pow", 2, Some(2), math_pow));
    reg("math_sqrt", n("math_sqrt", 1, Some(1), math_sqrt));
    reg("math_floor", n("math_floor", 1, Some(1), math_floor));
    reg("math_ceil", n("math_ceil", 1, Some(1), math_ceil));
    reg("math_round", n("math_round", 1, Some(1), math_round));
    reg("math_sin", n("math_sin", 1, Some(1), math_sin));
    reg("math_cos", n("math_cos", 1, Some(1), math_cos));
    reg("math_tan", n("math_tan", 1, Some(1), math_tan));
    reg("math_log", n("math_log", 1, Some(1), math_log));
    reg("math_log2", n("math_log2", 1, Some(1), math_log2));
    reg("math_exp", n("math_exp", 1, Some(1), math_exp));
    reg("math_pi", n("math_pi", 0, Some(0), math_pi));
    reg("math_e", n("math_e", 0, Some(0), math_e));
    reg("math_gcd", n("math_gcd", 2, Some(2), math_gcd));
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

fn as_f64(v: &Value, fn_name: &str, idx: usize) -> Result<f64, String> {
    match v {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(f) => Ok(*f),
        other => Err(format!(
            "`{fn_name}` expected a number at position {idx}, got `{}`",
            other.type_name()
        )),
    }
}

fn math_pow(args: &[Value]) -> Result<Value, String> {
    let base = as_f64(&args[0], "math_pow", 0)?;
    let exp = as_f64(&args[1], "math_pow", 1)?;
    Ok(Value::Float(base.powf(exp)))
}

fn math_sqrt(args: &[Value]) -> Result<Value, String> {
    let v = as_f64(&args[0], "math_sqrt", 0)?;
    if v < 0.0 {
        return Err(format!("math_sqrt: domain error for negative input {v}"));
    }
    Ok(Value::Float(v.sqrt()))
}

fn math_floor(args: &[Value]) -> Result<Value, String> {
    let v = as_f64(&args[0], "math_floor", 0)?;
    Ok(Value::Int(v.floor() as i64))
}

fn math_ceil(args: &[Value]) -> Result<Value, String> {
    let v = as_f64(&args[0], "math_ceil", 0)?;
    Ok(Value::Int(v.ceil() as i64))
}

fn math_round(args: &[Value]) -> Result<Value, String> {
    let v = as_f64(&args[0], "math_round", 0)?;
    Ok(Value::Int(v.round() as i64))
}

fn math_sin(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Float(as_f64(&args[0], "math_sin", 0)?.sin()))
}

fn math_cos(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Float(as_f64(&args[0], "math_cos", 0)?.cos()))
}

fn math_tan(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Float(as_f64(&args[0], "math_tan", 0)?.tan()))
}

fn math_log(args: &[Value]) -> Result<Value, String> {
    let v = as_f64(&args[0], "math_log", 0)?;
    if v <= 0.0 {
        return Err(format!("math_log: domain error for non-positive input {v}"));
    }
    Ok(Value::Float(v.ln()))
}

fn math_log2(args: &[Value]) -> Result<Value, String> {
    let v = as_f64(&args[0], "math_log2", 0)?;
    if v <= 0.0 {
        return Err(format!(
            "math_log2: domain error for non-positive input {v}"
        ));
    }
    Ok(Value::Float(v.log2()))
}

fn math_exp(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Float(as_f64(&args[0], "math_exp", 0)?.exp()))
}

fn math_pi(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::Float(std::f64::consts::PI))
}

fn math_e(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::Float(std::f64::consts::E))
}

fn math_gcd(args: &[Value]) -> Result<Value, String> {
    let a = match &args[0] {
        Value::Int(i) => i.unsigned_abs() as i128,
        other => {
            return Err(format!(
                "math_gcd: expected Int, got `{}`",
                other.type_name()
            ));
        }
    };
    let b = match &args[1] {
        Value::Int(i) => i.unsigned_abs() as i128,
        other => {
            return Err(format!(
                "math_gcd: expected Int, got `{}`",
                other.type_name()
            ));
        }
    };
    let (mut x, mut y) = (a, b);
    while y != 0 {
        let r = x % y;
        x = y;
        y = r;
    }
    Ok(Value::Int(x as i64))
}
