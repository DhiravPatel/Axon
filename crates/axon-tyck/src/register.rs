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
            "assert_eq", "panic", "anthropic", "mock_model", "default_model", "local_memory",
            // ---- Stage 11 stdlib: std.string ----
            "str_upper", "str_lower", "str_trim", "str_trim_start", "str_trim_end",
            "str_split", "str_split_lines", "str_split_once",
            "str_join", "str_contains", "str_starts_with", "str_ends_with",
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
            "dur_seconds", "dur_millis", "dur_micros", "dur_nanos", "dur_seconds_f64",
            "dur_from_seconds", "dur_from_millis",
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
            // ---- Stage 14 triggers: durable scheduling ----
            "trigger_every", "trigger_at", "trigger_cron", "trigger_remove", "trigger_len",
            "trigger_tick", "trigger_save", "trigger_load",
            // ---- Stage 14 skills: .axskill packaging ----
            "skill_pack", "skill_install", "skill_inspect",
            // ---- Stage 14 a2a: agent-to-agent discovery ----
            "a2a_card_load", "a2a_card_fetch", "a2a_card_save", "a2a_card_has_capability",
            // ---- Stage 15 guardrails: PII/injection/policy ----
            "guard_scan_pii", "guard_scan_secrets", "guard_injection_score",
            "guard_policy_evaluate",
            // ---- Stage 15 secrets: redaction-aware vault ----
            "secret_open", "secret_get", "secret_set", "secret_remove", "secret_names",
            "secret_redact",
            // ---- Stage 15 sandbox: resource-limited subprocesses ----
            "sandbox_run",
            // ---- Stage 16 eval: trajectory eval suite runner ----
            "eval_suite_new", "eval_add_scenario", "eval_add_metric",
            "eval_set_latency_budget", "eval_run", "eval_report_junit",
            // ---- Stage 16 cost: cost ledger ----
            "cost_record", "cost_profile_add", "cost_report",
            "cost_save", "cost_load", "cost_reset",
            // ---- Stage 16 ffi: subprocess FFI ----
            "ffi_call",
            // ---- Stage 17 env: environment binding ----
            "env_get", "env_get_or", "env_load_dotenv",
            // ---- Stage 17 deploy: HTTP server + manifest ----
            "serve_run", "deploy_write_manifest",
            // ---- Stage 18 supervisor restart strategies ----
            "super_new", "super_add_child", "super_on_failure", "super_escalated",
            "super_reset",
            // ---- Stage 18 schema migrations ----
            "schema_migrator_new", "schema_add_migration", "schema_migrate",
            "schema_migrate_reset",
            // ---- Stage 20 OTLP exporter ----
            "trace_export_otlp",
            // ---- Stage 21 TLS deploy server ----
            "serve_run_tls",
            // ---- Stage 22 platform sandboxes + Ed25519 identity ----
            "sandbox_run_with_profile",
            "a2a_keypair_generate", "a2a_keypair_from_seed",
            "a2a_sign_card", "a2a_verify_signed_card", "a2a_trust_store_new",
            // ---- Stage 23 dynamic-library FFI + delegated identity ----
            "ffi_dlib_call",
            "a2a_sign_delegation", "a2a_verify_delegation",
            // ---- Stage 24 §29 networks + workflow graphs ----
            "flow_network_new", "flow_network_add_node", "flow_network_add_edge",
            "flow_network_verify", "flow_network_unreachable_from",
            "flow_graph_new", "flow_graph_add_node", "flow_graph_add_edge",
            "flow_graph_verify", "flow_graph_topo", "flow_graph_roots", "flow_graph_leaves",
            "flow_graph_run",
            // ---- Stage 24 §29.8 / §49.2 / §56.3 / §56.4 combinators ----
            "flow_debate", "flow_tree_of_thought", "flow_race", "flow_batch",
            "flow_estimate_difficulty", "flow_route_difficulty",
            // ---- Stage 24 §49.1 reasoning budgets ----
            "reasoning_budget_new", "reasoning_budget_debit", "reasoning_budget_status",
            // ---- Stage 24 §49.2 plan loop drivers ----
            "plan_react_loop",
            // ---- Stage 24 §55.1 trajectory eval ----
            "eval_trajectory_new", "eval_trajectory_add_step", "eval_trajectory_set_answer",
            "eval_trajectory_tool_accuracy", "eval_trajectory_step_efficiency",
            "eval_trajectory_recovered", "eval_trajectory_no_forbidden_tool",
            "eval_trajectory_grounded", "eval_trajectory_no_secret_exposed",
            // ---- Stage 24 §55.2 redteam ----
            "redteam_load", "redteam_refusal_phrases",
            // ---- Stage 24 §55.3 sim.World ----
            "sim_world_new", "sim_world_spawn", "sim_world_script_send",
            "sim_world_script_note", "sim_world_script_settle", "sim_world_send_to",
            "sim_world_advance", "sim_world_run_until_settled", "sim_world_events",
            "sim_world_rand_u64",
            // ---- Stage 24 §56.1 prefix cache ----
            "cost_cache_insert", "cost_cache_lookup", "cost_cache_stats", "cost_cache_clear",
            // ---- Stage 25 §27.3 context policy ----
            "context_policy_plan",
            // ---- Stage 25 §52 saga ----
            "flow_saga_run",
            // ---- Stage 25 §52.2 durable timers ----
            "timer_arm", "timer_cancel", "timer_due", "timer_mark_fired",
            "timer_pending_count", "timer_save", "timer_load",
            // ---- Stage 25 §50.2/§50.3 RAG grounding ----
            "rag_assess_grounding",
            // ---- Stage 25 §51.2/§51.3 media generation ----
            "media_generate_image", "media_generate_audio",
            // ---- Stage 25 §53 skill use ----
            "skill_bind", "skill_narrow_effects",
            // ---- Stage 25 §54.1 agent card auto-publish ----
            "agent_card_derive", "agent_card_well_known_path",
            // ---- Stage 25 §41 metrics + serverless ----
            "metrics_record", "metrics_render_prometheus", "serverless_render",
            // ---- Stage 26 §39.2 deterministic helpers ----
            "clock_freeze", "clock_unfreeze", "rand_seed",
            // ---- Stage 26 §25.5 MCP registry ----
            "mcp_load_from_toml", "mcp_list_tools", "mcp_call_tool",
            "mcp_namespaces", "mcp_deferred_namespaces",
            // ---- Stage 26 §7.1 features ----
            "features_active",
            // ---- Stage 27 §25.6 approval ----
            "approval_open", "approval_approve", "approval_deny", "approval_get",
            "approval_pending_count", "approval_sweep_timeouts", "approval_next_id",
            "approval_purge_terminal",
            // ---- Stage 27 §24.3 prompt @version ----
            "prompt_version_register", "prompt_version_set_default",
            "prompt_version_pick", "prompt_version_versions_for",
            "prompt_version_prompts",
            // ---- Stage 28 §29.5 consensus + spawn_pool ----
            "flow_consensus", "flow_spawn_pool",
            // ---- Stage 36 §36.B.3 majority sugars over flow_consensus ----
            "flow_majority", "flow_majority_with",
            // ---- Stage 28 §29.9 human pseudo-agent ----
            "human_request", "human_resolve", "human_cancel",
            // ---- Stage 28 §30 policy block ----
            "policy_block_new", "policy_block_allow", "policy_block_deny",
            "policy_block_check", "policy_block_charge", "policy_block_add_budget",
            "policy_block_add_rate", "policy_block_audit_summary",
            // ---- Stage 28 §35.2 FFI bridges ----
            "ffi_bridge_call",
            // ---- Stage 28 §35.3 protocol adapters ----
            "serve_protocol_route", "serve_protocol_wrap", "serve_render_grpc_proto",
            // ---- Stage 29 §19 try_recover ----
            "try_recover",
            // ---- Stage 29 §28 streams ----
            "stream_new", "stream_send", "stream_take", "stream_close",
            "stream_is_done", "stream_stats", "for_await",
            // ---- Stage 29 §29.7 @restart variants ----
            "restart_policy_parse", "restart_policy_should_restart",
            // ---- Stage 31 computer-use primitives ----
            "computer_screenshot", "computer_click", "computer_double_click",
            "computer_mouse_move", "computer_drag", "computer_scroll",
            "computer_type", "computer_key", "computer_wait",
            "computer_action_log",
            // ---- Stage 31 GBNF schema emitter ----
            "schema_to_gbnf",
            // ---- Stage 32 async I/O acceptance: sleepy mock model ----
            "mock_model_slow",
            // ---- Stage 36 §36.B.5 call-site resilience combinators ----
            // Effect-row neutral: the inner thunk's row drives the
            // capability check (call_value attenuates as usual).
            "with_retry", "with_timeout",
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
            // §32 — async I/O slice: parallel model calls. Same effect row
            // as a plain `ask`: needs LLM and (for real providers) Net.
            ("flow_parallel_asks", &["LLM", "Net"]),
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
        // (item_span, identifier_span) — the identifier span lets the
        // duplicate-definition fix replace just the name, not the
        // whole item.
        let (name, span, name_span) = match item {
            Item::Use(_) => return,
            Item::Fn(f) => (f.name.name.clone(), f.span, f.name.span),
            Item::Type(t) => (t.name.name.clone(), t.span, t.name.span),
            Item::Schema(s) => (s.name.name.clone(), s.span, s.name.span),
            Item::Agent(a) => (a.name.name.clone(), a.span, a.name.span),
            Item::Actor(a) => (a.name.name.clone(), a.span, a.name.span),
            Item::Supervisor(s) => (s.name.name.clone(), s.span, s.name.span),
            Item::Graph(g) => (g.name.name.clone(), g.span, g.name.span),
            Item::Network(n) => (n.name.name.clone(), n.span, n.name.span),
            Item::Orchestrate(o) => (o.name.name.clone(), o.span, o.name.span),
            Item::Policy(p) => (p.name.name.clone(), p.span, p.name.span),
            Item::MemPolicy(p) => (p.name.name.clone(), p.span, p.name.span),
            Item::Model(m) => (m.name.name.clone(), m.span, m.name.span),
            Item::Tool(t) => (t.name.name.clone(), t.span, t.name.span),
            Item::Memory(m) => (m.name.name.clone(), m.span, m.name.span),
            Item::Prompt(p) => (p.name.name.clone(), p.span, p.name.span),
            Item::Trait(t) => (t.name.name.clone(), t.span, t.name.span),
            Item::Impl(_) => return,
            Item::Const(c) => (c.name.name.clone(), c.span, c.name.span),
            Item::Effect(e) => (e.name.name.clone(), e.span, e.name.span),
            // Test/Eval names are bare strings rather than `Ident`s, so
            // we don't have a sub-span; the fix anchor falls back to the
            // whole item span (the rewrite would be conservative).
            Item::Test(t) => (t.name.clone(), t.span, t.span),
            Item::Eval(e) => (e.name.clone(), e.span, e.span),
            Item::Config(c) => (c.name.name.clone(), c.span, c.name.span),
        };
        let (_id, dup) = self.ctx.register(ItemSig {
            name: name.clone(),
            span,
            kind: ItemSigKind::Opaque,
            generics: Vec::new(),
        });
        if let Some(prev_id) = dup {
            let prev_span = self.ctx.get(prev_id).map(|s| s.span).unwrap_or(Span::DUMMY);
            let existing = self.ctx.item_names();
            self.report(errors::duplicate_definition_with_fix(
                span, &name, prev_span, name_span, &existing,
            ));
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
        // §35.1 — Native agent declaration slots. The parser already
        // accepts `uses_tools: [...]`, `memory: ...`, `policy: ident`,
        // `strategy: ...` as generic `Setting` members today; the
        // runtime evaluates the four well-known ones at spawn time and
        // exposes them on the actor's state as `tools` / `memory` /
        // `policy` / `strategy`. To make `self.tools` etc. type-check
        // inside handler bodies, surface the four as virtual state
        // fields here — typed `[dyn]`, `Memory`, `String`, `String`
        // respectively. This is intentionally permissive (no per-slot
        // type validation) so user code patterns like the test
        // fixture `tools: List<dyn>` stay legal; tighter typing lands
        // when the slot syntax becomes a first-class AST variant.
        for m in members {
            if let AgentMember::Setting { key, .. } = m {
                let virtual_field = match key.name.as_str() {
                    "uses_tools" => Some(("tools", Ty::List(Box::new(Ty::Dyn)))),
                    "memory" => Some(("memory", Ty::Memory)),
                    "policy" => Some(("policy", Ty::String)),
                    "strategy" => Some(("strategy", Ty::String)),
                    _ => None,
                };
                if let Some((name, ty)) = virtual_field {
                    // Don't double-register if the user happens to have
                    // a `state tools: ...` field of the same name.
                    if !out.iter().any(|f| f.name == name) {
                        out.push(FieldSig {
                            name: name.to_string(),
                            ty,
                            has_default: true,
                            refinements: Vec::new(),
                        });
                    }
                }
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
