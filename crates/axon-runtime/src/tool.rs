//! Runtime representation of a user-declared `tool`.
//!
//! When the program declares `tool search(q: String) -> String uses { Net } { ... }`,
//! we build a [`ToolDef`] at load time: the body's AST, the captured env,
//! the declared input/return types, the description, and the effect row
//! the runtime attenuates to when the *model* invokes the tool mid-turn.
//!
//! Tool dispatch lives in `eval.rs`; this module is purely the data shape.

use std::rc::Rc;

use axon_ast::{Block, Param, Type};

use crate::env::Env;

#[derive(Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: ToolBody,
    pub env: Env,
    pub declared_effects: Option<Vec<String>>,
}

#[derive(Clone)]
pub enum ToolBody {
    Block(Block),
    /// Reserved for native (Rust-implemented) tools — wired in a later
    /// stage when the stdlib ships built-in tools.
    #[allow(dead_code)]
    Native(Rc<dyn Fn(&[crate::value::Value]) -> Result<crate::value::Value, String>>),
}
