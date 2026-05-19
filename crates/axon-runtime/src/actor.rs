//! Actor / agent runtime support.
//!
//! Stage 5.5 implements the *synchronous* slice of the actor model: spawn
//! creates a fresh actor with its constructor parameters and `state`
//! declarations initialized; method calls on a spawned handle dispatch the
//! named message handler inline. State lives behind `Rc<RefCell>` so the
//! handler can mutate it across calls, and so two handles to the same actor
//! observe each other's writes.
//!
//! Lifecycle hooks:
//!
//!   * `on start(...)` runs once, right after the constructor params and
//!     state fields are initialized.
//!   * `on error(...)` runs whenever a message handler raises a runtime
//!     error. If the lifecycle hook *itself* runs cleanly the error still
//!     propagates to the caller — `on error` is for logging/observability
//!     in v0, not recovery. (Supervisor-based recovery lands in 5.5b.)
//!   * `on stop` is parsed and registered, but isn't invoked yet — there's
//!     no explicit shutdown signal in this stage.
//!
//! Concurrency is *not* implemented: there is no scheduler, no mailbox FIFO,
//! no parallelism. The point is to get the *programming model* right (agent
//! identity, message dispatch, state encapsulation, lifecycle, capability
//! attenuation per handler) before adding the multitasking layer.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use axon_ast::{
    AgentDecl, AgentMember, AgentSettingValue, Block, LifecycleEvent, LifecycleHandler,
    MessageHandler, Param,
};

/// A *spawned* agent or actor. Distinct from the syntactic [`AgentDef`]:
/// the def is the *class*; this is the *instance*.
pub struct Actor {
    pub id: u64,
    pub type_name: Rc<String>,
    /// Ordered map of fields. Constructor params and `state` declarations
    /// land here at spawn time; handlers read and write through it as
    /// `self.<field>`.
    pub state: Rc<RefCell<Vec<(String, super::value::Value)>>>,
    pub def: Rc<AgentDef>,
}

/// The *class* form of an agent — collected once when the program loads.
pub struct AgentDef {
    pub name: String,
    pub ctor_params: Vec<Param>,
    pub state_fields: Vec<StateField>,
    /// Other named settings (`model:`, `memory:`, `policy:`, ...). They are
    /// captured here so the type-checking / future stages can apply them;
    /// at runtime they become regular fields on the actor.
    pub settings: Vec<(String, AgentSettingValue)>,
    pub handlers: HashMap<String, Rc<HandlerDef>>,
    pub lifecycle: Lifecycle,
}

#[derive(Clone)]
pub struct StateField {
    pub name: String,
    pub init: Option<axon_ast::Expr>,
    pub durable: bool,
}

pub struct HandlerDef {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Block,
    pub declared_effects: Option<Vec<String>>,
}

#[derive(Default)]
pub struct Lifecycle {
    pub on_start: Option<Rc<LifecycleHandlerDef>>,
    pub on_stop: Option<Rc<LifecycleHandlerDef>>,
    pub on_error: Option<Rc<LifecycleHandlerDef>>,
}

pub struct LifecycleHandlerDef {
    pub which: LifecycleEvent,
    pub params: Vec<Param>,
    pub body: Block,
}

impl AgentDef {
    /// Build an `AgentDef` from a parsed `AgentDecl`. The conversion is
    /// pure — no evaluation happens here; state init exprs are run later
    /// at spawn time.
    pub fn from_decl(decl: &AgentDecl) -> Self {
        let mut state_fields: Vec<StateField> = Vec::new();
        let mut settings: Vec<(String, AgentSettingValue)> = Vec::new();
        let mut handlers: HashMap<String, Rc<HandlerDef>> = HashMap::new();
        let mut lifecycle = Lifecycle::default();
        for m in &decl.members {
            match m {
                AgentMember::State {
                    name, init, durable, ..
                } => state_fields.push(StateField {
                    name: name.name.clone(),
                    init: init.clone(),
                    durable: *durable,
                }),
                AgentMember::Setting { key, value, .. } => {
                    settings.push((key.name.clone(), value.clone()));
                }
                AgentMember::Handler(h) => {
                    handlers.insert(h.name.name.clone(), Rc::new(handler_from(h)));
                }
                AgentMember::Lifecycle(lh) => {
                    let def = Rc::new(lifecycle_from(lh));
                    match lh.which {
                        LifecycleEvent::Start => lifecycle.on_start = Some(def),
                        LifecycleEvent::Stop => lifecycle.on_stop = Some(def),
                        LifecycleEvent::Error => lifecycle.on_error = Some(def),
                    }
                }
                AgentMember::Fn(_) => {
                    // Inner `fn` declarations inside an agent block are not
                    // yet exposed as members; the parser captures them but
                    // they have no runtime semantics in Stage 5.5.
                }
            }
        }
        Self {
            name: decl.name.name.clone(),
            ctor_params: decl.params.clone(),
            state_fields,
            settings,
            handlers,
            lifecycle,
        }
    }
}

fn handler_from(h: &MessageHandler) -> HandlerDef {
    HandlerDef {
        name: h.name.name.clone(),
        params: h.params.clone(),
        body: h.body.clone(),
        declared_effects: h.effect_row.as_ref().map(|row| {
            row.effects
                .iter()
                .map(|e| {
                    e.path
                        .segments
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".")
                })
                .collect()
        }),
    }
}

fn lifecycle_from(lh: &LifecycleHandler) -> LifecycleHandlerDef {
    LifecycleHandlerDef {
        which: lh.which,
        params: lh.params.clone(),
        body: lh.body.clone(),
    }
}
