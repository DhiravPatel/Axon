//! The global context — the table of registered top-level items.
//!
//! Population: pass 1 (`pass_register_items`) calls [`Ctx::register`] for
//! every item it encounters. Lookups: pass 2 and the CLI walk the table by
//! name (`by_name`) or by id (`get`).

use std::collections::HashMap;

use axon_diag::Span;
use axon_types::{EffectRow, ItemId, ItemSig, ItemSigKind, Ty};

/// Resolved item table built by the type checker.
#[derive(Default)]
pub struct Ctx {
    items: Vec<ItemSig>,
    by_name: HashMap<String, ItemId>,
    /// Effect rows the runtime requires when a given built-in is called.
    /// Populated at startup by `pass_register_items`. Consulted in
    /// `call_ty` to flag missing-effect errors statically.
    builtin_effects: HashMap<String, EffectRow>,
}

impl Ctx {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new item. Returns the assigned [`ItemId`]. If an item with
    /// the same name was already registered, the existing id is returned and
    /// a duplicate-name diagnostic is the caller's responsibility.
    pub fn register(&mut self, sig: ItemSig) -> (ItemId, Option<ItemId>) {
        if let Some(existing) = self.by_name.get(&sig.name).copied() {
            return (existing, Some(existing));
        }
        let id = ItemId(self.items.len() as u32);
        self.by_name.insert(sig.name.clone(), id);
        self.items.push(sig);
        (id, None)
    }

    pub fn get(&self, id: ItemId) -> Option<&ItemSig> {
        self.items.get(id.0 as usize)
    }

    pub fn lookup(&self, name: &str) -> Option<ItemId> {
        self.by_name.get(name).copied()
    }

    /// Record that calling the built-in `name` requires effect row `row`.
    /// Callers of these names will have those effects added to the used
    /// row of the enclosing function at type-check time.
    pub fn register_builtin_effects(&mut self, name: impl Into<String>, row: EffectRow) {
        self.builtin_effects.insert(name.into(), row);
    }

    /// Look up the registered effect row for `name`, if any.
    pub fn builtin_effects_for(&self, name: &str) -> Option<&EffectRow> {
        self.builtin_effects.get(name)
    }

    /// Overwrite the signature at `id`. Used by the registration pass to
    /// replace placeholder signatures with their fully-lowered form on the
    /// second pass.
    pub fn replace(&mut self, id: ItemId, sig: ItemSig) {
        let idx = id.0 as usize;
        if idx < self.items.len() {
            // Keep the name->id mapping consistent if the name changed
            // (it should not, but be defensive).
            let old_name = self.items[idx].name.clone();
            if old_name != sig.name {
                self.by_name.remove(&old_name);
                self.by_name.insert(sig.name.clone(), id);
            }
            self.items[idx] = sig;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (ItemId, &ItemSig)> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, s)| (ItemId(i as u32), s))
    }

    /// Every top-level item name. Used by did-you-mean fixes when a
    /// reference can't be resolved.
    pub fn item_names(&self) -> Vec<String> {
        self.by_name.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Resolve a named item to its canonical [`Ty`] when used at a *value*
    /// position. Records / schemas / agents themselves have no value-level
    /// "naming yourself" type; this returns [`Ty::Error`] for those.
    pub fn value_ty(&self, name: &str) -> Option<(Ty, Span)> {
        let id = self.lookup(name)?;
        let sig = self.get(id)?;
        let ty = match &sig.kind {
            ItemSigKind::Fn(fs) | ItemSigKind::Tool(fs) | ItemSigKind::Prompt(fs) => Ty::Fn {
                params: fs.params.iter().map(|p| p.ty.clone()).collect(),
                ret: Box::new(fs.ret.clone()),
                effects: fs.effects.clone(),
            },
            ItemSigKind::Const(t) => t.clone(),
            ItemSigKind::Model => Ty::Model,
            ItemSigKind::Memory => Ty::Memory,
            ItemSigKind::Alias(t) | ItemSigKind::Newtype(t) => t.clone(),
            _ => return None,
        };
        Some((ty, sig.span))
    }
}
