//! `std.time` — pure date/duration helpers.
//!
//! Clock-reading primitives (`time_now`, sleep) require the `Time` capability
//! and live in `axon-runtime`'s built-ins. Everything here is pure: parsing,
//! field access, arithmetic on `Date` and `Duration` values.

use axon_runtime::{NativeFn, Value};

pub const COUNT: usize = 12;

pub(crate) fn register(reg: &mut dyn FnMut(&'static str, NativeFn)) {
    reg("dur_seconds", n("dur_seconds", 1, Some(1), dur_seconds));
    reg("dur_millis", n("dur_millis", 1, Some(1), dur_millis));
    // §36.B.2 — `.as_ms()` / `.as_ns()` shaped accessors. The bench/eval
    // reports the async-eval work produces want sub-millisecond precision;
    // `dur_millis` rounds; these expose the full range.
    reg("dur_micros", n("dur_micros", 1, Some(1), dur_micros));
    reg("dur_nanos", n("dur_nanos", 1, Some(1), dur_nanos));
    reg("dur_seconds_f64", n("dur_seconds_f64", 1, Some(1), dur_seconds_f64));
    reg("dur_from_seconds", n("dur_from_seconds", 1, Some(1), dur_from_seconds));
    reg("dur_from_millis", n("dur_from_millis", 1, Some(1), dur_from_millis));
    reg("date_year", n("date_year", 1, Some(1), date_year));
    reg("date_month", n("date_month", 1, Some(1), date_month));
    reg("date_day", n("date_day", 1, Some(1), date_day));
    reg("date_make", n("date_make", 3, Some(3), date_make));
    reg("date_is_leap", n("date_is_leap", 1, Some(1), date_is_leap));
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

fn dur_seconds(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Duration(ns) => Ok(Value::Int(*ns / 1_000_000_000)),
        other => Err(format!(
            "dur_seconds: expected Duration, got `{}`",
            other.type_name()
        )),
    }
}

fn dur_millis(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Duration(ns) => Ok(Value::Int(*ns / 1_000_000)),
        other => Err(format!(
            "dur_millis: expected Duration, got `{}`",
            other.type_name()
        )),
    }
}

fn dur_micros(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Duration(ns) => Ok(Value::Int(*ns / 1_000)),
        other => Err(format!(
            "dur_micros: expected Duration, got `{}`",
            other.type_name()
        )),
    }
}

fn dur_nanos(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Duration(ns) => Ok(Value::Int(*ns)),
        other => Err(format!(
            "dur_nanos: expected Duration, got `{}`",
            other.type_name()
        )),
    }
}

fn dur_seconds_f64(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Duration(ns) => Ok(Value::Float(*ns as f64 / 1_000_000_000.0)),
        other => Err(format!(
            "dur_seconds_f64: expected Duration, got `{}`",
            other.type_name()
        )),
    }
}

fn dur_from_seconds(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Int(s) => s
            .checked_mul(1_000_000_000)
            .map(Value::Duration)
            .ok_or_else(|| "dur_from_seconds: overflow".to_string()),
        other => Err(format!(
            "dur_from_seconds: expected Int, got `{}`",
            other.type_name()
        )),
    }
}

fn dur_from_millis(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Int(ms) => ms
            .checked_mul(1_000_000)
            .map(Value::Duration)
            .ok_or_else(|| "dur_from_millis: overflow".to_string()),
        other => Err(format!(
            "dur_from_millis: expected Int, got `{}`",
            other.type_name()
        )),
    }
}

fn date_year(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Date { y, .. } => Ok(Value::Int(*y as i64)),
        Value::DateTime { y, .. } => Ok(Value::Int(*y as i64)),
        other => Err(format!(
            "date_year: expected Date or DateTime, got `{}`",
            other.type_name()
        )),
    }
}

fn date_month(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Date { m, .. } => Ok(Value::Int(*m as i64)),
        Value::DateTime { m, .. } => Ok(Value::Int(*m as i64)),
        other => Err(format!(
            "date_month: expected Date or DateTime, got `{}`",
            other.type_name()
        )),
    }
}

fn date_day(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Date { d, .. } => Ok(Value::Int(*d as i64)),
        Value::DateTime { d, .. } => Ok(Value::Int(*d as i64)),
        other => Err(format!(
            "date_day: expected Date or DateTime, got `{}`",
            other.type_name()
        )),
    }
}

fn date_make(args: &[Value]) -> Result<Value, String> {
    let y = match &args[0] {
        Value::Int(i) if (1..=9999).contains(i) => *i as u16,
        _ => return Err("date_make: year must be in 1..=9999".into()),
    };
    let m = match &args[1] {
        Value::Int(i) if (1..=12).contains(i) => *i as u8,
        _ => return Err("date_make: month must be in 1..=12".into()),
    };
    let d = match &args[2] {
        Value::Int(i) if (1..=31).contains(i) => *i as u8,
        _ => return Err("date_make: day must be in 1..=31".into()),
    };
    let max_day = days_in_month(y, m);
    if d > max_day {
        return Err(format!(
            "date_make: {y}-{m:02} has only {max_day} days (got {d})"
        ));
    }
    Ok(Value::Date { y, m, d })
}

fn date_is_leap(args: &[Value]) -> Result<Value, String> {
    let y = match &args[0] {
        Value::Int(i) => *i,
        Value::Date { y, .. } | Value::DateTime { y, .. } => *y as i64,
        other => {
            return Err(format!(
                "date_is_leap: expected Int or Date, got `{}`",
                other.type_name()
            ));
        }
    };
    Ok(Value::Bool(is_leap(y)))
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn days_in_month(y: u16, m: u8) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y as i64) => 29,
        2 => 28,
        _ => 0,
    }
}
