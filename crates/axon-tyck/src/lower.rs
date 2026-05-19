//! Lower AST type expressions to canonical [`Ty`] values.
//!
//! The lowering is *contextual*: a generic-parameter name visible in the
//! current item's signature resolves to a [`Ty::Param`]; other paths resolve
//! to built-in or user-defined names. Unknown names produce a diagnostic and
//! the special [`Ty::Error`] sentinel so the rest of checking can continue.

use std::collections::HashMap;

use axon_ast::{Type, TypeKind};
use axon_types::{EffectRow, ItemSigKind, ParamId, Ty};

use crate::builtins;
use crate::errors;
use crate::Checker;

/// Lookup-only environment used during lowering: maps generic-parameter
/// names to their [`ParamId`]. The checker creates one of these per item.
#[derive(Default, Clone)]
pub struct ParamEnv {
    by_name: HashMap<String, ParamId>,
}

impl ParamEnv {
    pub fn add(&mut self, name: String, id: ParamId) {
        self.by_name.insert(name, id);
    }

    pub fn get(&self, name: &str) -> Option<ParamId> {
        self.by_name.get(name).copied()
    }
}

impl<'a> Checker<'a> {
    pub(crate) fn lower_type(&mut self, ty: &Type, params: &ParamEnv) -> Ty {
        match &ty.kind {
            TypeKind::Unit => Ty::Unit,
            TypeKind::List(t) => Ty::List(Box::new(self.lower_type(t, params))),
            TypeKind::Map { key, value } => Ty::Map(
                Box::new(self.lower_type(key, params)),
                Box::new(self.lower_type(value, params)),
            ),
            TypeKind::Set(t) => Ty::Set(Box::new(self.lower_type(t, params))),
            TypeKind::Tuple(xs) => {
                Ty::Tuple(xs.iter().map(|t| self.lower_type(t, params)).collect())
            }
            TypeKind::Ref { is_mut, inner } => Ty::Ref {
                mutable: *is_mut,
                inner: Box::new(self.lower_type(inner, params)),
            },
            TypeKind::Tainted(t) => Ty::Tainted(Box::new(self.lower_type(t, params))),
            TypeKind::Option(t) => Ty::Nullable(Box::new(self.lower_type(t, params))),
            TypeKind::Fn {
                params: ast_params,
                return_type,
                effects,
            } => {
                let param_tys = ast_params
                    .iter()
                    .map(|p| self.lower_type(&p.ty, params))
                    .collect();
                let ret = Box::new(self.lower_type(return_type, params));
                let effects = effects
                    .as_ref()
                    .map(|r| self.lower_effect_row(r))
                    .unwrap_or_default();
                Ty::Fn {
                    params: param_tys,
                    ret,
                    effects,
                }
            }
            TypeKind::WithEffects { inner, effects } => {
                let inner_ty = self.lower_type(inner, params);
                let row = self.lower_effect_row(effects);
                match inner_ty {
                    Ty::Fn {
                        params: ps,
                        ret,
                        effects: existing,
                    } => Ty::Fn {
                        params: ps,
                        ret,
                        effects: existing.union(&row),
                    },
                    other => {
                        // The spec allows `T uses {...}` only on function
                        // types in practice; we accept it and warn so the
                        // surface stays permissive.
                        self.report(
                            errors::note(
                                format!(
                                    "`uses {{...}}` suffix on non-function type `{other}` has no effect"
                                ),
                                ty.span,
                            ),
                        );
                        other
                    }
                }
            }
            TypeKind::Refined { inner, refinement: _ } => {
                // Refinements are *recorded* on fields/params elsewhere;
                // structurally the refined type is the inner type for the
                // monomorphic checker's purposes.
                self.lower_type(inner, params)
            }
            TypeKind::Union(a, b) => {
                let mut flat = Vec::new();
                self.flatten_union(a, params, &mut flat);
                self.flatten_union(b, params, &mut flat);
                Ty::Union(flat)
            }
            TypeKind::Path {
                path,
                generics: args,
            } => self.lower_path_type(ty.span, path, args, params),
        }
    }

    fn flatten_union(&mut self, ty: &Type, params: &ParamEnv, out: &mut Vec<Ty>) {
        match &ty.kind {
            TypeKind::Union(a, b) => {
                self.flatten_union(a, params, out);
                self.flatten_union(b, params, out);
            }
            _ => out.push(self.lower_type(ty, params)),
        }
    }

    fn lower_path_type(
        &mut self,
        span: axon_diag::Span,
        path: &axon_ast::Path,
        args: &[Type],
        params: &ParamEnv,
    ) -> Ty {
        if path.segments.len() == 1 {
            let name = path.segments[0].name.as_str();
            // Generic parameter?
            if let Some(pid) = params.get(name) {
                if !args.is_empty() {
                    self.report(
                        axon_diag::Diagnostic::error(
                            format!("type parameter `{name}` cannot take type arguments"),
                            span,
                        )
                        .with_code("E0220"),
                    );
                }
                return Ty::Param(pid);
            }
            // Container built-ins (parametric).
            if builtins::is_builtin_container(name) {
                return self.lower_builtin_container(span, name, args, params);
            }
            // Non-parametric built-in.
            if let Some(ty) = builtins::builtin_type(name) {
                if !args.is_empty() {
                    self.report(
                        axon_diag::Diagnostic::error(
                            format!("built-in type `{name}` does not take type arguments"),
                            span,
                        )
                        .with_code("E0221"),
                    );
                }
                return ty;
            }
            // User-defined item.
            if let Some(id) = self.ctx.lookup(name) {
                let lowered_args: Vec<Ty> =
                    args.iter().map(|t| self.lower_type(t, params)).collect();
                let sig = self.ctx.get(id).cloned();
                if let Some(sig) = sig {
                    if !sig.generics.is_empty() && lowered_args.len() != sig.generics.len() {
                        self.report(
                            axon_diag::Diagnostic::error(
                                format!(
                                    "type `{name}` expects {} type argument(s), found {}",
                                    sig.generics.len(),
                                    lowered_args.len()
                                ),
                                span,
                            )
                            .with_code("E0222"),
                        );
                    }
                    return match sig.kind {
                        ItemSigKind::Agent { .. } => Ty::AgentHandle(id),
                        ItemSigKind::Actor { .. } => Ty::ActorHandle(id),
                        ItemSigKind::Alias(t) | ItemSigKind::Newtype(t) if args.is_empty() => t,
                        _ => Ty::Named {
                            id,
                            args: lowered_args,
                        },
                    };
                }
            }
            self.report(errors::type_not_found(span, name));
            return Ty::Error;
        }
        // Dotted paths (e.g. `std.io.Reader`) are not yet routed through a
        // module system. Treat them as unresolved for now.
        self.report(
            axon_diag::Diagnostic::error(
                format!(
                    "dotted type path `{}` is not yet resolved (modules land in stage 8)",
                    path.segments
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".")
                ),
                span,
            )
            .with_code("E0223"),
        );
        Ty::Error
    }

    fn lower_builtin_container(
        &mut self,
        span: axon_diag::Span,
        name: &str,
        args: &[Type],
        params: &ParamEnv,
    ) -> Ty {
        let expected = match name {
            "Map" | "Tool" => 2,
            _ => 1,
        };
        if args.len() != expected {
            self.report(
                axon_diag::Diagnostic::error(
                    format!(
                        "`{name}` expects {expected} type argument(s), found {}",
                        args.len()
                    ),
                    span,
                )
                .with_code("E0224"),
            );
            return Ty::Error;
        }
        match name {
            "Option" => Ty::Option(Box::new(self.lower_type(&args[0], params))),
            "List" => Ty::List(Box::new(self.lower_type(&args[0], params))),
            "Set" => Ty::Set(Box::new(self.lower_type(&args[0], params))),
            "Map" => Ty::Map(
                Box::new(self.lower_type(&args[0], params)),
                Box::new(self.lower_type(&args[1], params)),
            ),
            "Tool" => Ty::Tool(
                Box::new(self.lower_type(&args[0], params)),
                Box::new(self.lower_type(&args[1], params)),
            ),
            "Stream" => Ty::Stream(Box::new(self.lower_type(&args[0], params))),
            "Chan" => Ty::Chan(Box::new(self.lower_type(&args[0], params))),
            "Secret" => Ty::Secret(Box::new(self.lower_type(&args[0], params))),
            "Tainted" => Ty::Tainted(Box::new(self.lower_type(&args[0], params))),
            _ => unreachable!(),
        }
    }

    pub(crate) fn lower_effect_row(&mut self, row: &axon_ast::EffectRow) -> EffectRow {
        let mut out = EffectRow::pure();
        for atom in &row.effects {
            let name = atom
                .path
                .segments
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(".");
            out.add(name);
        }
        out
    }
}
