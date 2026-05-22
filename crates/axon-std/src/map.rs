//! `std.map` — key/value map operations.
//!
//! Maps are `Rc<RefCell<Vec<(Value, Value)>>>` at runtime — a small-map
//! representation that preserves insertion order. Lookups are O(n) but
//! exact-equality and Value-keyed.

use std::cell::RefCell;
use std::rc::Rc;

use axon_runtime::{NativeFn, Value};

pub const COUNT: usize = 10;

pub(crate) fn register(reg: &mut dyn FnMut(&'static str, NativeFn)) {
    reg("map_new", n("map_new", 0, Some(0), map_new));
    reg("map_len", n("map_len", 1, Some(1), map_len));
    reg("map_get", n("map_get", 2, Some(2), map_get));
    reg("map_get_or", n("map_get_or", 3, Some(3), map_get_or));
    reg("map_set", n("map_set", 3, Some(3), map_set));
    reg("map_remove", n("map_remove", 2, Some(2), map_remove));
    reg("map_contains", n("map_contains", 2, Some(2), map_contains));
    reg("map_keys", n("map_keys", 1, Some(1), map_keys));
    reg("map_values", n("map_values", 1, Some(1), map_values));
    reg("map_merge", n("map_merge", 2, Some(2), map_merge));
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

fn map_arg(
    args: &[Value],
    idx: usize,
    fn_name: &str,
) -> Result<Rc<RefCell<Vec<(Value, Value)>>>, String> {
    match &args[idx] {
        Value::Map(m) => Ok(m.clone()),
        other => Err(format!(
            "`{fn_name}` expected a Map at position {idx}, got `{}`",
            other.type_name()
        )),
    }
}

fn map_new(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::Map(Rc::new(RefCell::new(Vec::new()))))
}

fn map_len(args: &[Value]) -> Result<Value, String> {
    let m = map_arg(args, 0, "map_len")?;
    let n = m.borrow().len() as i64;
    Ok(Value::Int(n))
}

fn map_get(args: &[Value]) -> Result<Value, String> {
    let m = map_arg(args, 0, "map_get")?;
    let key = &args[1];
    for (k, v) in m.borrow().iter() {
        if k == key {
            return Ok(v.clone());
        }
    }
    Ok(Value::Nil)
}

fn map_get_or(args: &[Value]) -> Result<Value, String> {
    let m = map_arg(args, 0, "map_get_or")?;
    let key = &args[1];
    for (k, v) in m.borrow().iter() {
        if k == key {
            return Ok(v.clone());
        }
    }
    Ok(args[2].clone())
}

fn map_set(args: &[Value]) -> Result<Value, String> {
    let m = map_arg(args, 0, "map_set")?;
    let key = args[1].clone();
    let value = args[2].clone();
    let mut entries = m.borrow_mut();
    for (k, v) in entries.iter_mut() {
        if *k == key {
            *v = value;
            return Ok(Value::Map(m.clone()));
        }
    }
    entries.push((key, value));
    drop(entries);
    Ok(Value::Map(m))
}

fn map_remove(args: &[Value]) -> Result<Value, String> {
    let m = map_arg(args, 0, "map_remove")?;
    let key = &args[1];
    let mut entries = m.borrow_mut();
    if let Some(pos) = entries.iter().position(|(k, _)| k == key) {
        let (_, v) = entries.remove(pos);
        Ok(v)
    } else {
        Ok(Value::Nil)
    }
}

fn map_contains(args: &[Value]) -> Result<Value, String> {
    let m = map_arg(args, 0, "map_contains")?;
    let key = &args[1];
    let found = m.borrow().iter().any(|(k, _)| k == key);
    Ok(Value::Bool(found))
}

fn map_keys(args: &[Value]) -> Result<Value, String> {
    let m = map_arg(args, 0, "map_keys")?;
    let keys: Vec<Value> = m.borrow().iter().map(|(k, _)| k.clone()).collect();
    Ok(Value::List(Rc::new(RefCell::new(keys))))
}

fn map_values(args: &[Value]) -> Result<Value, String> {
    let m = map_arg(args, 0, "map_values")?;
    let vals: Vec<Value> = m.borrow().iter().map(|(_, v)| v.clone()).collect();
    Ok(Value::List(Rc::new(RefCell::new(vals))))
}

fn map_merge(args: &[Value]) -> Result<Value, String> {
    let a = map_arg(args, 0, "map_merge")?;
    let b = map_arg(args, 1, "map_merge")?;
    let mut out: Vec<(Value, Value)> = a.borrow().clone();
    for (bk, bv) in b.borrow().iter() {
        if let Some(slot) = out.iter_mut().find(|(k, _)| k == bk) {
            slot.1 = bv.clone();
        } else {
            out.push((bk.clone(), bv.clone()));
        }
    }
    Ok(Value::Map(Rc::new(RefCell::new(out))))
}
