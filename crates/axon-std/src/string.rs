//! `std.string` — UTF-8 string operations.
//!
//! Every function takes the receiver as the first positional argument so
//! call syntax stays unified (`str_upper(s)`).

use std::rc::Rc;

use axon_runtime::{NativeFn, Value};

pub const COUNT: usize = 18;

pub(crate) fn register(reg: &mut dyn FnMut(&'static str, NativeFn)) {
    reg("str_upper", n("str_upper", 1, Some(1), str_upper));
    reg("str_lower", n("str_lower", 1, Some(1), str_lower));
    reg("str_trim", n("str_trim", 1, Some(1), str_trim));
    reg("str_trim_start", n("str_trim_start", 1, Some(1), str_trim_start));
    reg("str_trim_end", n("str_trim_end", 1, Some(1), str_trim_end));
    reg("str_split", n("str_split", 2, Some(2), str_split));
    // §36.B.2 sugar — these two reduce the dogfood-agent's hand-rolled
    // `split_pipe` / `split_lines` helpers (~30 lines) to one call each.
    reg("str_split_lines", n("str_split_lines", 1, Some(1), str_split_lines));
    reg("str_split_once", n("str_split_once", 2, Some(2), str_split_once));
    reg("str_join", n("str_join", 2, Some(2), str_join));
    reg("str_contains", n("str_contains", 2, Some(2), str_contains));
    reg("str_starts_with", n("str_starts_with", 2, Some(2), str_starts_with));
    reg("str_ends_with", n("str_ends_with", 2, Some(2), str_ends_with));
    reg("str_replace", n("str_replace", 3, Some(3), str_replace));
    reg("str_repeat", n("str_repeat", 2, Some(2), str_repeat));
    reg("str_len", n("str_len", 1, Some(1), str_len));
    reg("str_chars", n("str_chars", 1, Some(1), str_chars));
    reg("str_index_of", n("str_index_of", 2, Some(2), str_index_of));
    reg("str_substring", n("str_substring", 3, Some(3), str_substring));
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

fn s_arg(args: &[Value], idx: usize, fn_name: &str) -> Result<Rc<String>, String> {
    match &args[idx] {
        Value::String(s) => Ok(s.clone()),
        other => Err(format!(
            "`{fn_name}` expected a String at position {idx}, got `{}`",
            other.type_name()
        )),
    }
}

fn str_upper(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_upper")?;
    Ok(Value::String(Rc::new(s.to_uppercase())))
}

fn str_lower(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_lower")?;
    Ok(Value::String(Rc::new(s.to_lowercase())))
}

fn str_trim(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_trim")?;
    Ok(Value::String(Rc::new(s.trim().to_string())))
}

fn str_trim_start(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_trim_start")?;
    Ok(Value::String(Rc::new(s.trim_start().to_string())))
}

fn str_trim_end(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_trim_end")?;
    Ok(Value::String(Rc::new(s.trim_end().to_string())))
}

fn str_split(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_split")?;
    let sep = s_arg(args, 1, "str_split")?;
    let parts: Vec<Value> = if sep.is_empty() {
        s.chars()
            .map(|c| Value::String(Rc::new(c.to_string())))
            .collect()
    } else {
        s.split(sep.as_str())
            .map(|p| Value::String(Rc::new(p.to_string())))
            .collect()
    };
    Ok(Value::List(Rc::new(std::cell::RefCell::new(parts))))
}

/// `str_split_lines(s)` — split on every LF, returning one element per line.
/// Trailing newline does NOT produce a trailing empty string (matches Rust's
/// `lines()`). CR before LF is included with the LF; this matches BufRead::lines.
fn str_split_lines(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_split_lines")?;
    let parts: Vec<Value> = s
        .lines()
        .map(|p| Value::String(Rc::new(p.to_string())))
        .collect();
    Ok(Value::List(Rc::new(std::cell::RefCell::new(parts))))
}

/// `str_split_once(s, sep)` — split at the FIRST occurrence of `sep`,
/// returning `[head, tail]`. Returns the original `[s]` (single-element)
/// when `sep` doesn't appear. Useful for parsing `key=value`, `path:line`.
fn str_split_once(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_split_once")?;
    let sep = s_arg(args, 1, "str_split_once")?;
    if sep.is_empty() {
        return Err("str_split_once: separator must not be empty".into());
    }
    let parts: Vec<Value> = match s.split_once(sep.as_str()) {
        Some((head, tail)) => vec![
            Value::String(Rc::new(head.to_string())),
            Value::String(Rc::new(tail.to_string())),
        ],
        None => vec![Value::String(s.clone())],
    };
    Ok(Value::List(Rc::new(std::cell::RefCell::new(parts))))
}

fn str_join(args: &[Value]) -> Result<Value, String> {
    let sep = s_arg(args, 0, "str_join")?;
    let list = match &args[1] {
        Value::List(l) => l.clone(),
        other => {
            return Err(format!(
                "`str_join` expected a List of String, got `{}`",
                other.type_name()
            ));
        }
    };
    let items = list.borrow();
    let mut pieces = Vec::with_capacity(items.len());
    for v in items.iter() {
        match v {
            Value::String(s) => pieces.push(s.as_str().to_string()),
            other => {
                return Err(format!(
                    "`str_join` expected List<String>, found `{}`",
                    other.type_name()
                ));
            }
        }
    }
    Ok(Value::String(Rc::new(pieces.join(sep.as_str()))))
}

fn str_contains(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_contains")?;
    let needle = s_arg(args, 1, "str_contains")?;
    Ok(Value::Bool(s.contains(needle.as_str())))
}

fn str_starts_with(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_starts_with")?;
    let prefix = s_arg(args, 1, "str_starts_with")?;
    Ok(Value::Bool(s.starts_with(prefix.as_str())))
}

fn str_ends_with(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_ends_with")?;
    let suffix = s_arg(args, 1, "str_ends_with")?;
    Ok(Value::Bool(s.ends_with(suffix.as_str())))
}

fn str_replace(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_replace")?;
    let from = s_arg(args, 1, "str_replace")?;
    let to = s_arg(args, 2, "str_replace")?;
    Ok(Value::String(Rc::new(
        s.replace(from.as_str(), to.as_str()),
    )))
}

fn str_repeat(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_repeat")?;
    let n = match &args[1] {
        Value::Int(i) if *i >= 0 => *i as usize,
        Value::Int(_) => return Err("`str_repeat` count must be non-negative".into()),
        other => {
            return Err(format!(
                "`str_repeat` count must be Int, got `{}`",
                other.type_name()
            ));
        }
    };
    Ok(Value::String(Rc::new(s.repeat(n))))
}

fn str_len(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_len")?;
    Ok(Value::Int(s.chars().count() as i64))
}

fn str_chars(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_chars")?;
    let chars: Vec<Value> = s.chars().map(Value::Char).collect();
    Ok(Value::List(Rc::new(std::cell::RefCell::new(chars))))
}

fn str_index_of(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_index_of")?;
    let needle = s_arg(args, 1, "str_index_of")?;
    Ok(match s.find(needle.as_str()) {
        Some(byte_idx) => {
            let char_idx = s[..byte_idx].chars().count();
            Value::Int(char_idx as i64)
        }
        None => Value::Int(-1),
    })
}

fn str_substring(args: &[Value]) -> Result<Value, String> {
    let s = s_arg(args, 0, "str_substring")?;
    let start = match &args[1] {
        Value::Int(i) if *i >= 0 => *i as usize,
        _ => return Err("`str_substring` start must be a non-negative Int".into()),
    };
    let end = match &args[2] {
        Value::Int(i) if *i >= 0 => *i as usize,
        _ => return Err("`str_substring` end must be a non-negative Int".into()),
    };
    if start > end {
        return Err("`str_substring` start cannot exceed end".into());
    }
    let total = s.chars().count();
    let end_clamped = end.min(total);
    let start_clamped = start.min(end_clamped);
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= start_clamped && i < end_clamped {
            out.push(c);
        }
    }
    Ok(Value::String(Rc::new(out)))
}
