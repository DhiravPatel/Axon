//! `std.set` — unordered set operations.
//!
//! Sets are `Rc<RefCell<Vec<Value>>>` at runtime with an "insert keeps the
//! first occurrence" invariant maintained by every mutator.

use std::cell::RefCell;
use std::rc::Rc;

use axon_runtime::{NativeFn, Value};

pub const COUNT: usize = 9;

pub(crate) fn register(reg: &mut dyn FnMut(&'static str, NativeFn)) {
    reg("set_new", n("set_new", 0, None, set_new));
    reg("set_len", n("set_len", 1, Some(1), set_len));
    reg("set_add", n("set_add", 2, Some(2), set_add));
    reg("set_remove", n("set_remove", 2, Some(2), set_remove));
    reg("set_contains", n("set_contains", 2, Some(2), set_contains));
    reg("set_union", n("set_union", 2, Some(2), set_union));
    reg("set_intersection", n("set_intersection", 2, Some(2), set_intersection));
    reg("set_difference", n("set_difference", 2, Some(2), set_difference));
    reg("set_to_list", n("set_to_list", 1, Some(1), set_to_list));
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

fn set_arg(args: &[Value], idx: usize, fn_name: &str) -> Result<Rc<RefCell<Vec<Value>>>, String> {
    match &args[idx] {
        Value::Set(s) => Ok(s.clone()),
        other => Err(format!(
            "`{fn_name}` expected a Set at position {idx}, got `{}`",
            other.type_name()
        )),
    }
}

fn set_new(args: &[Value]) -> Result<Value, String> {
    let mut out: Vec<Value> = Vec::new();
    for v in args {
        if !out.iter().any(|x| x == v) {
            out.push(v.clone());
        }
    }
    Ok(Value::Set(Rc::new(RefCell::new(out))))
}

fn set_len(args: &[Value]) -> Result<Value, String> {
    let s = set_arg(args, 0, "set_len")?;
    let n = s.borrow().len() as i64;
    Ok(Value::Int(n))
}

fn set_add(args: &[Value]) -> Result<Value, String> {
    let s = set_arg(args, 0, "set_add")?;
    let v = args[1].clone();
    let mut entries = s.borrow_mut();
    if !entries.iter().any(|x| *x == v) {
        entries.push(v);
    }
    drop(entries);
    Ok(Value::Set(s))
}

fn set_remove(args: &[Value]) -> Result<Value, String> {
    let s = set_arg(args, 0, "set_remove")?;
    let v = &args[1];
    let mut entries = s.borrow_mut();
    if let Some(pos) = entries.iter().position(|x| x == v) {
        entries.remove(pos);
    }
    drop(entries);
    Ok(Value::Set(s))
}

fn set_contains(args: &[Value]) -> Result<Value, String> {
    let s = set_arg(args, 0, "set_contains")?;
    let v = &args[1];
    let found = s.borrow().iter().any(|x| x == v);
    Ok(Value::Bool(found))
}

fn set_union(args: &[Value]) -> Result<Value, String> {
    let a = set_arg(args, 0, "set_union")?;
    let b = set_arg(args, 1, "set_union")?;
    let mut out: Vec<Value> = a.borrow().clone();
    for v in b.borrow().iter() {
        if !out.iter().any(|x| x == v) {
            out.push(v.clone());
        }
    }
    Ok(Value::Set(Rc::new(RefCell::new(out))))
}

fn set_intersection(args: &[Value]) -> Result<Value, String> {
    let a = set_arg(args, 0, "set_intersection")?;
    let b = set_arg(args, 1, "set_intersection")?;
    let b_items = b.borrow();
    let out: Vec<Value> = a
        .borrow()
        .iter()
        .filter(|v| b_items.iter().any(|x| x == *v))
        .cloned()
        .collect();
    Ok(Value::Set(Rc::new(RefCell::new(out))))
}

fn set_difference(args: &[Value]) -> Result<Value, String> {
    let a = set_arg(args, 0, "set_difference")?;
    let b = set_arg(args, 1, "set_difference")?;
    let b_items = b.borrow();
    let out: Vec<Value> = a
        .borrow()
        .iter()
        .filter(|v| !b_items.iter().any(|x| x == *v))
        .cloned()
        .collect();
    Ok(Value::Set(Rc::new(RefCell::new(out))))
}

fn set_to_list(args: &[Value]) -> Result<Value, String> {
    let s = set_arg(args, 0, "set_to_list")?;
    let copy = s.borrow().clone();
    Ok(Value::List(Rc::new(RefCell::new(copy))))
}
