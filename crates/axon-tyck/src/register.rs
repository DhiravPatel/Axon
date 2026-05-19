//! Pass 1 — register every top-level item by name so later items (and the
//! body-checking pass) can reference them in any order.

use axon_ast::{AgentMember, GenericParam as AstGen, Item, Program, TypeDeclBody};
use axon_types::{
    EffectRow, FieldSig, FnSig, GenericParam, GenericParamKind, HandlerSig, ItemSig,
    ItemSigKind, ParamSig, Ty, VariantSig,
};

use crate::errors;
use crate::lower::ParamEnv;
use crate::Checker;

impl<'a> Checker<'a> {
    pub(crate) fn pass_register_items(&mut self, program: &Program) {
        // Seed built-in functions provided by the runtime so user code
        // referring to them type-checks. They're typed `Dyn` for now;
        // when the stdlib lands we tighten these to their real signatures.
        //
        // Side-effect built-ins also register their *effect row* so the
        // type checker can flag missing-effect at call sites — the runtime
        // would catch the same violation, but failing early at compile time
        // is the whole point of effect rows.
        const PURE: &[&str] = &[
            "len", "str", "int", "float", "bool", "abs", "min", "max", "chan", "assert",
            "assert_eq", "panic", "anthropic", "mock_model", "local_memory",
            // ---- Stage 11 stdlib: std.string ----
            "str_upper", "str_lower", "str_trim", "str_trim_start", "str_trim_end",
            "str_split", "str_join", "str_contains", "str_starts_with", "str_ends_with",
            "str_replace", "str_repeat", "str_len", "str_chars", "str_index_of",
            "str_substring",
            // ---- Stage 11 stdlib: std.list ----
            "list_new", "list_len", "list_push", "list_pop", "list_get", "list_set",
            "list_first", "list_last", "list_contains", "list_reverse", "list_sort",
            "list_take", "list_drop", "list_concat", "list_index_of", "list_remove_at",
            // ---- Stage 11 stdlib: std.map ----
            "map_new", "map_len", "map_get", "map_get_or", "map_set", "map_remove",
            "map_contains", "map_keys", "map_values", "map_merge",
            // ---- Stage 11 stdlib: std.set ----
            "set_new", "set_len", "set_add", "set_remove", "set_contains", "set_union",
            "set_intersection", "set_difference", "set_to_list",
            // ---- Stage 11 stdlib: std.option ----
            "opt_some", "opt_none", "opt_is_some", "opt_is_none", "opt_unwrap_or", "opt_or",
            // ---- Stage 11 stdlib: std.result ----
            "result_ok", "result_err", "result_is_ok", "result_is_err", "result_unwrap_or",
            "result_value", "result_error",
            // ---- Stage 11 stdlib: std.math ----
            "math_pow", "math_sqrt", "math_floor", "math_ceil", "math_round", "math_sin",
            "math_cos", "math_tan", "math_log", "math_log2", "math_exp", "math_pi",
            "math_e", "math_gcd",
            // ---- Stage 11 stdlib: std.time (pure helpers; clock is in EFFECTFUL) ----
            "dur_seconds", "dur_millis", "dur_from_seconds", "dur_from_millis",
            "date_year", "date_month", "date_day", "date_make", "date_is_leap",
            // ---- Stage 11 memory: host-installed kv store ----
            "mem_open_file", "mem_open_ephemeral", "mem_set", "mem_get", "mem_remove",
            "mem_keys", "mem_len", "mem_contains",
            // ---- Stage 12 rag: retrieval index ----
            "rag_index_new", "rag_index_len", "rag_chunk", "rag_ingest", "rag_retrieve",
            "rag_save", "rag_load",
            // ---- Stage 12 media: typed multimodal primitives ----
            "media_image_load", "media_audio_load", "media_document_load", "media_sniff",
            // ---- Stage 13 flow: orchestration + reasoning combinators ----
            "flow_seq", "flow_parallel", "flow_refine",
        ];
        const EFFECTFUL: &[(&str, &[&str])] = &[
            ("print", &["Console"]),
            ("println", &["Console"]),
            ("eprint", &["Console"]),
            ("print_int", &["Console"]),
            ("read_file", &["Fs.Read"]),
            ("write_file", &["Fs.Write"]),
            ("time_now", &["Time"]),
            ("random_int", &["Random"]),
            ("random_float", &["Random"]),
            ("http_fetch", &["Net"]),
        ];
        for name in PURE {
            self.ctx.register(ItemSig {
                name: (*name).to_string(),
                span: Span::DUMMY,
                kind: ItemSigKind::Const(Ty::Dyn),
                generics: Vec::new(),
            });
        }
        for (name, effects) in EFFECTFUL {
            self.ctx.register(ItemSig {
                name: (*name).to_string(),
                span: Span::DUMMY,
                kind: ItemSigKind::Const(Ty::Dyn),
                generics: Vec::new(),
            });
            let mut row = axon_types::EffectRow::pure();
            for e in *effects {
                row.add(*e);
            }
            self.ctx.register_builtin_effects(*name, row);
        }

        // Run twice: the first pass enters *placeholder* signatures so that
        // mutually-recursive items can refer to each other; the second pass
        // fills in real signatures using a populated ctx. This keeps the
        // lowering of types (`lower_type`) able to look up agent/actor
        // handles even when they are forward references.
        for item in &program.items {
            self.register_placeholder(item);
        }
        for item in &program.items {
            self.register_full(item);
        }
        // Use declarations bring names into scope. We don't have a module
        // system yet (lands in stage 8), so we register imported names as
        // opaque constants of type `Dyn`. This lets calls and references
        // type-check without us pretending to know the real signatures.
        for item in &program.items {
            if let Item::Use(u) = item {
                let names: Vec<&axon_ast::Ident> = match (&u.items, &u.alias) {
                    (Some(items), _) => items.iter().collect(),
                    (None, Some(alias)) => vec![alias],
                    (None, None) => match u.path.segments.last() {
                        Some(last) => vec![last],
                        None => continue,
                    },
                };
                for n in names {
                    if self.ctx.lookup(&n.name).is_none() {
                        self.ctx.register(ItemSig {
                            name: n.name.clone(),
                            span: n.span,
                            kind: ItemSigKind::Const(Ty::Dyn),
                            generics: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    fn register_placeholder(&mut self, item: &Item) {
        let (name, span) = match item {
            Item::Use(_) => return,
            Item::Fn(f) => (f.name.name.clone(), f.span),
            Item::Type(t) => (t.name.name.clone(), t.span),
            Item::Schema(s) => (s.name.name.clone(), s.span),
            Item::Agent(a) => (a.name.name.clone(), a.span),
            Item::Actor(a) => (a.name.name.clone(), a.span),
            Item::Supervisor(s) => (s.name.name.clone(), s.span),
            Item::Graph(g) => (g.name.name.clone(), g.span),
            Item::Network(n) => (n.name.name.clone(), n.span),
            Item::Orchestrate(o) => (o.name.name.clone(), o.span),
            Item::Policy(p) => (p.name.name.clone(), p.span),
            Item::MemPolicy(p) => (p.name.name.clone(), p.span),
            Item::Model(m) => (m.name.name.clone(), m.span),
            Item::Tool(t) => (t.name.name.clone(), t.span),
            Item::Memory(m) => (m.name.name.clone(), m.span),
            Item::Prompt(p) => (p.name.name.clone(), p.span),
            Item::Trait(t) => (t.name.name.clone(), t.span),
            Item::Impl(_) => return,
            Item::Const(c) => (c.name.name.clone(), c.span),
            Item::Effect(e) => (e.name.name.clone(), e.span),
            Item::Test(t) => (t.name.clone(), t.span),
            Item::Eval(e) => (e.name.clone(), e.span),
            Item::Config(c) => (c.name.name.clone(), c.span),
        };
        let (_id, dup) = self.ctx.register(ItemSig {
            name: name.clone(),
            span,
            kind: ItemSigKind::Opaque,
            generics: Vec::new(),
        });
        if let Some(prev_id) = dup {
            let prev_span = self.ctx.get(prev_id).map(|s| s.span).unwrap_or(Span::DUMMY);
            self.report(errors::duplicate_definition(span, &name, prev_span));
        }
    }

    fn register_full(&mut self, item: &Item) {
        let (name, span, generics_ast) = match item {
            Item::Fn(f) => (f.name.name.clone(), f.span, &f.generics),
            Item::Type(t) => (t.name.name.clone(), t.span, &t.generics),
            Item::Schema(s) => (s.name.name.clone(), s.span, &empty_generics()),
            Item::Agent(a) => (a.name.name.clone(), a.span, &empty_generics()),
            Item::Actor(a) => (a.name.name.clone(), a.span, &empty_generics()),
            Item::Model(m) => (m.name.name.clone(), m.span, &empty_generics()),
            Item::Tool(t) => (t.name.name.clone(), t.span, &empty_generics()),
            Item::Memory(m) => (m.name.name.clone(), m.span, &empty_generics()),
            Item::Prompt(p) => (p.name.name.clone(), p.span, &empty_generics()),
            Item::Const(c) => (c.name.name.clone(), c.span, &empty_generics()),
            // Items we don't yet lower in full keep their placeholder.
            _ => return,
        };
        let generics = lower_generics(generics_ast);
        let mut param_env = ParamEnv::default();
        for (i, g) in generics.iter().enumerate() {
            param_env.add(g.name.clone(), axon_types::ParamId(i as u32));
        }
        let kind = match item {
            Item::Fn(f) => ItemSigKind::Fn(self.fn_sig(&f.params, &f.return_type, &f.effect_row, &param_env)),
            Item::Type(t) => match &t.body {
                TypeDeclBody::Record(fields) => {
                    ItemSigKind::Record(self.field_sigs(fields, &param_env))
                }
                TypeDeclBody::Sum(variants) => {
                    let vs = variants
                        .iter()
                        .map(|v| VariantSig {
                            name: v.name.name.clone(),
                            fields: v
                                .fields
                                .iter()
                                .filter_map(|vf| match vf {
                                    axon_ast::VariantField::Named(f) => Some(FieldSig {
                                        name: f.name.name.clone(),
                                        ty: self.lower_type(&f.ty, &param_env),
                                        has_default: f.default.is_some(),
                                        refinements: f
                                            .refinements
                                            .iter()
                                            .map(|r| r.name.name.clone())
                                            .collect(),
                                    }),
                                    axon_ast::VariantField::Anonymous(_) => None,
                                })
                                .collect(),
                        })
                        .collect();
                    ItemSigKind::Sum(vs)
                }
                TypeDeclBody::Alias(t) => ItemSigKind::Alias(self.lower_type(t, &param_env)),
                TypeDeclBody::Newtype { inner, .. } => {
                    ItemSigKind::Newtype(self.lower_type(inner, &param_env))
                }
            },
            Item::Schema(s) => ItemSigKind::Schema {
                version: s.version,
                fields: self.field_sigs(&s.fields, &param_env),
            },
            Item::Agent(a) => {
                let params = self.param_sigs(&a.params, &param_env);
                let state_fields = self.state_field_sigs(&a.members, &param_env);
                let handlers = self.handler_sigs(&a.members, &param_env);
                ItemSigKind::Agent {
                    params,
                    state_fields,
                    handlers,
                }
            }
            Item::Actor(a) => {
                let params = self.param_sigs(&a.params, &param_env);
                let state_fields = self.state_field_sigs(&a.members, &param_env);
                let handlers = self.handler_sigs(&a.members, &param_env);
                ItemSigKind::Actor {
                    params,
                    state_fields,
                    handlers,
                }
            }
            Item::Model(_) => ItemSigKind::Model,
            Item::Tool(t) => {
                let row = t
                    .effect_row
                    .as_ref()
                    .map(|r| self.lower_effect_row(r))
                    .unwrap_or_default();
                let params = self.param_sigs(&t.params, &param_env);
                let ret = self.lower_type(&t.return_type, &param_env);
                ItemSigKind::Tool(FnSig {
                    params,
                    ret,
                    effects: row,
                })
            }
            Item::Memory(_) => ItemSigKind::Memory,
            Item::Prompt(p) => {
                let params = self.param_sigs(&p.params, &param_env);
                let ret = self.lower_type(&p.return_type, &param_env);
                ItemSigKind::Prompt(FnSig {
                    params,
                    ret,
                    effects: EffectRow::singleton("LLM"),
                })
            }
            Item::Const(c) => {
                let ty = match &c.ty {
                    Some(t) => self.lower_type(t, &param_env),
                    None => Ty::Dyn,
                };
                ItemSigKind::Const(ty)
            }
            _ => return,
        };
        // Replace the placeholder.
        let id = self.ctx.lookup(&name).expect("placeholder must exist");
        let sig = ItemSig {
            name,
            span,
            kind,
            generics,
        };
        self.ctx.replace(id, sig);
    }

    fn fn_sig(
        &mut self,
        params: &[axon_ast::Param],
        return_type: &Option<axon_ast::Type>,
        effect_row: &Option<axon_ast::EffectRow>,
        param_env: &ParamEnv,
    ) -> FnSig {
        FnSig {
            params: self.param_sigs(params, param_env),
            ret: match return_type {
                Some(t) => self.lower_type(t, param_env),
                None => Ty::Unit,
            },
            effects: effect_row
                .as_ref()
                .map(|r| self.lower_effect_row(r))
                .unwrap_or_default(),
        }
    }

    fn param_sigs(&mut self, params: &[axon_ast::Param], param_env: &ParamEnv) -> Vec<ParamSig> {
        params
            .iter()
            .map(|p| ParamSig {
                name: p.name.name.clone(),
                ty: self.lower_type(&p.ty, param_env),
                has_default: p.default.is_some(),
            })
            .collect()
    }

    fn field_sigs(&mut self, fields: &[axon_ast::Field], param_env: &ParamEnv) -> Vec<FieldSig> {
        fields
            .iter()
            .map(|f| FieldSig {
                name: f.name.name.clone(),
                ty: self.lower_type(&f.ty, param_env),
                has_default: f.default.is_some(),
                refinements: f.refinements.iter().map(|r| r.name.name.clone()).collect(),
            })
            .collect()
    }

    fn state_field_sigs(
        &mut self,
        members: &[AgentMember],
        param_env: &ParamEnv,
    ) -> Vec<FieldSig> {
        let mut out = Vec::new();
        for m in members {
            if let AgentMember::State { name, ty, init, .. } = m {
                out.push(FieldSig {
                    name: name.name.clone(),
                    ty: self.lower_type(ty, param_env),
                    has_default: init.is_some(),
                    refinements: Vec::new(),
                });
            }
        }
        out
    }

    fn handler_sigs(
        &mut self,
        members: &[AgentMember],
        param_env: &ParamEnv,
    ) -> Vec<HandlerSig> {
        let mut out = Vec::new();
        for m in members {
            if let AgentMember::Handler(h) = m {
                out.push(HandlerSig {
                    name: h.name.name.clone(),
                    params: self.param_sigs(&h.params, param_env),
                    ret: match &h.return_type {
                        Some(t) => self.lower_type(t, param_env),
                        None => Ty::Unit,
                    },
                    effects: h
                        .effect_row
                        .as_ref()
                        .map(|r| self.lower_effect_row(r))
                        .unwrap_or_default(),
                });
            }
        }
        out
    }
}

fn lower_generics(gens: &axon_ast::Generics) -> Vec<GenericParam> {
    gens.params
        .iter()
        .map(|g| match g {
            AstGen::Type { name, .. } => GenericParam {
                name: name.name.clone(),
                kind: GenericParamKind::Type,
            },
            AstGen::Covariant { name, .. } => GenericParam {
                name: name.name.clone(),
                kind: GenericParamKind::Covariant,
            },
            AstGen::Contravariant { name, .. } => GenericParam {
                name: name.name.clone(),
                kind: GenericParamKind::Contravariant,
            },
            AstGen::Effect { name, .. } => GenericParam {
                name: name.name.clone(),
                kind: GenericParamKind::Effect,
            },
        })
        .collect()
}

fn empty_generics() -> axon_ast::Generics {
    axon_ast::Generics::default()
}

use axon_diag::Span;
