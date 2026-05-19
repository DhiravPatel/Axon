//! Lexical environment for the tree-walking interpreter.
//!
//! Each `Env` is a chain of frames pointing back to a parent. Frames are
//! `Rc<RefCell<Frame>>` so closures can share a frame with their enclosing
//! scope and observe mutations the surrounding code makes after they're
//! captured.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::value::Value;

#[derive(Clone)]
pub struct Env {
    inner: Rc<RefCell<Frame>>,
}

struct Frame {
    bindings: HashMap<String, Value>,
    parent: Option<Env>,
}

impl Env {
    /// Construct a fresh root environment with no bindings.
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Frame {
                bindings: HashMap::new(),
                parent: None,
            })),
        }
    }

    /// Push a child frame on top of `self`. The new env shares its parent
    /// chain with `self`; bindings in the child don't shadow at the parent.
    pub fn child(&self) -> Self {
        Self {
            inner: Rc::new(RefCell::new(Frame {
                bindings: HashMap::new(),
                parent: Some(self.clone()),
            })),
        }
    }

    pub fn define(&self, name: impl Into<String>, value: Value) {
        self.inner.borrow_mut().bindings.insert(name.into(), value);
    }

    pub fn lookup(&self, name: &str) -> Option<Value> {
        let f = self.inner.borrow();
        if let Some(v) = f.bindings.get(name) {
            return Some(v.clone());
        }
        f.parent.as_ref().and_then(|p| p.lookup(name))
    }

    /// Assign to an existing binding, walking the parent chain. Returns
    /// `true` if the assignment found a binding; `false` if no binding by
    /// that name was found anywhere (callers can choose to introduce a new
    /// one or report an error).
    pub fn assign(&self, name: &str, value: Value) -> bool {
        // Try the current frame first.
        {
            let mut f = self.inner.borrow_mut();
            if f.bindings.contains_key(name) {
                f.bindings.insert(name.to_string(), value);
                return true;
            }
        }
        // Then walk up.
        let parent = self.inner.borrow().parent.clone();
        match parent {
            Some(p) => p.assign(name, value),
            None => false,
        }
    }
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}
