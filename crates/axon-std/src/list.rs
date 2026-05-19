//! `std.list` — list/sequence operations.
//!
//! Lists are `Rc<RefCell<Vec<Value>>>` at runtime, so mutating operations
//! affect the caller's list (Python-style reference semantics — see
//! `value.rs` for the rationale).

use std::cell::RefCell;
use std::rc::Rc;

use axon_runtime::{NativeFn, Value};

pub const COUNT: usize = 16;

pub(crate) fn register(reg: &mut dyn FnMut(&'static str, NativeFn)) {
    reg("list_new", n("list_new", 0, None, list_new));
    reg("list_len", n("list_len", 1, Some(1), list_len));
    reg("list_push", n("list_push", 2, Some(2), list_push));
    reg("list_pop", n("list_pop", 1, Some(1), list_pop));
    reg("list_get", n("list_get", 2, Some(2), list_get));
    reg("list_set", n("list_set", 3, Some(3), list_set));
    reg("list_first", n("list_first", 1, Some(1), list_first));
    reg("list_last", n("list_last", 1, Some(1), list_last));
    reg("list_contains", n("list_contains", 2, Some(2), list_contains));
    reg("list_reverse", n("list_reverse", 1, Some(1), list_reverse));
    reg("list_sort", n("list_sort", 1, Some(1), list_sort));
    reg("list_take", n("list_take", 2, Some(2), list_take));
    reg("list_drop", n("list_drop", 2, Some(2), list_drop));
    reg("list_concat", n("list_concat", 2, Some(2), list_concat));
    reg("list_index_of", n("list_index_of", 2, Some(2), list_index_of));
    reg("list_remove_at", n("list_remove_at", 2, Some(2), list_remove_at));
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

fn list_arg(args: &[Value], idx: usize, fn_name: &str) -> Result<Rc<RefCell<Vec<Value>>>, String> {
    match &args[idx] {
        Value::List(l) => Ok(l.clone()),
        other => Err(format!(
            "`{fn_name}` expected a List at position {idx}, got `{}`",
            other.type_name()
        )),
    }
}

fn int_arg(args: &[Value], idx: usize, fn_name: &str) -> Result<i64, String> {
    match &args[idx] {
        Value::Int(i) => Ok(*i),
        other => Err(format!(
            "`{fn_name}` expected an Int at position {idx}, got `{}`",
            other.type_name()
        )),
    }
}

fn list_new(args: &[Value]) -> Result<Value, String> {
    Ok(Value::List(Rc::new(RefCell::new(args.to_vec()))))
}

fn list_len(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_len")?;
    let n = l.borrow().len() as i64;
    Ok(Value::Int(n))
}

fn list_push(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_push")?;
    l.borrow_mut().push(args[1].clone());
    Ok(Value::List(l))
}

fn list_pop(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_pop")?;
    let v = l.borrow_mut().pop().unwrap_or(Value::Nil);
    Ok(v)
}

fn list_get(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_get")?;
    let idx = int_arg(args, 1, "list_get")?;
    let (len, value) = {
        let items = l.borrow();
        let len = items.len() as i64;
        let real = if idx < 0 { idx + len } else { idx };
        if real < 0 || real >= len {
            return Err(format!("list_get: index {idx} out of bounds (len {len})"));
        }
        (len, items[real as usize].clone())
    };
    let _ = len;
    Ok(value)
}

fn list_set(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_set")?;
    let idx = int_arg(args, 1, "list_set")?;
    let len = l.borrow().len() as i64;
    let real = if idx < 0 { idx + len } else { idx };
    if real < 0 || real >= len {
        return Err(format!("list_set: index {idx} out of bounds (len {len})"));
    }
    l.borrow_mut()[real as usize] = args[2].clone();
    Ok(Value::List(l))
}

fn list_first(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_first")?;
    let v = l.borrow().first().cloned().unwrap_or(Value::Nil);
    Ok(v)
}

fn list_last(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_last")?;
    let v = l.borrow().last().cloned().unwrap_or(Value::Nil);
    Ok(v)
}

fn list_contains(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_contains")?;
    let needle = &args[1];
    let found = l.borrow().iter().any(|v| v == needle);
    Ok(Value::Bool(found))
}

fn list_reverse(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_reverse")?;
    let mut copy: Vec<Value> = l.borrow().clone();
    copy.reverse();
    Ok(Value::List(Rc::new(RefCell::new(copy))))
}

fn list_sort(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_sort")?;
    let mut copy: Vec<Value> = l.borrow().clone();
    let mut err: Option<String> = None;
    copy.sort_by(|a, b| match a.cmp(b) {
        Some(o) => o,
        None => {
            if err.is_none() {
                err = Some(format!(
                    "list_sort: cannot compare `{}` and `{}`",
                    a.type_name(),
                    b.type_name()
                ));
            }
            std::cmp::Ordering::Equal
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    Ok(Value::List(Rc::new(RefCell::new(copy))))
}

fn list_take(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_take")?;
    let n = int_arg(args, 1, "list_take")?.max(0) as usize;
    let items = l.borrow();
    let taken: Vec<Value> = items.iter().take(n).cloned().collect();
    Ok(Value::List(Rc::new(RefCell::new(taken))))
}

fn list_drop(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_drop")?;
    let n = int_arg(args, 1, "list_drop")?.max(0) as usize;
    let items = l.borrow();
    let dropped: Vec<Value> = items.iter().skip(n).cloned().collect();
    Ok(Value::List(Rc::new(RefCell::new(dropped))))
}

fn list_concat(args: &[Value]) -> Result<Value, String> {
    let a = list_arg(args, 0, "list_concat")?;
    let b = list_arg(args, 1, "list_concat")?;
    let mut joined = a.borrow().clone();
    joined.extend(b.borrow().iter().cloned());
    Ok(Value::List(Rc::new(RefCell::new(joined))))
}

fn list_index_of(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_index_of")?;
    let needle = &args[1];
    let pos = {
        let items = l.borrow();
        items.iter().position(|v| v == needle)
    };
    Ok(Value::Int(pos.map(|i| i as i64).unwrap_or(-1)))
}

fn list_remove_at(args: &[Value]) -> Result<Value, String> {
    let l = list_arg(args, 0, "list_remove_at")?;
    let idx = int_arg(args, 1, "list_remove_at")?;
    let len = l.borrow().len() as i64;
    let real = if idx < 0 { idx + len } else { idx };
    if real < 0 || real >= len {
        return Err(format!(
            "list_remove_at: index {idx} out of bounds (len {len})"
        ));
    }
    let v = l.borrow_mut().remove(real as usize);
    Ok(v)
}
