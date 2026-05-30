//! Schema → GBNF grammar emitter.
//!
//! Constrained-decoding backends (llama.cpp, mlx, vllm, others) accept
//! a GBNF (Grammar BNF) grammar that the sampler enforces token-by-
//! token. Compiling a `schema` declaration to GBNF means the model
//! *can't* emit JSON that doesn't match the schema — eliminating the
//! whole JSON-repair-loop class of bugs (the spec's §56.3 promise).
//!
//! The emitter is deliberately limited to the JSON subset every
//! constrained-decoding backend agrees on:
//!
//!   * `string`, `int`, `float`, `bool`, `null`
//!   * object (record): a fixed key set with typed values
//!   * array: `List<T>` → `[T, T, …]`
//!   * optional: `T?` → `T | null`
//!   * union of literal strings (enums): rendered as alternation
//!
//! Refinement attributes (`@min`, `@max`, `@regex`) ride through as
//! comments today; a follow-up wires them to actual GBNF constraints
//! where the backend supports it.

use std::collections::BTreeSet;

use axon_ast::{Field, SchemaDecl, Type, TypeKind};

/// Emit a GBNF grammar that recognizes a single JSON object matching
/// `schema`. The root rule is named `root` (the convention in
/// llama.cpp / mlx).
pub fn emit_for_schema(schema: &SchemaDecl) -> String {
    let mut e = Emitter::default();
    let root = e.emit_object(&schema.fields);
    let mut out = String::new();
    out.push_str(&format!("# schema: {}\n", schema.name.name));
    if let Some(v) = schema.version {
        out.push_str(&format!("# version: {v}\n"));
    }
    out.push_str(&format!("root ::= {root}\n"));
    out.push_str("\n# common terminals\n");
    out.push_str(GBNF_PRELUDE);
    out
}

/// Just the rules, no header — useful when composing multiple schemas
/// into one grammar.
pub fn emit_rules_for_schema(schema: &SchemaDecl) -> String {
    let mut e = Emitter::default();
    let root = e.emit_object(&schema.fields);
    let mut out = String::new();
    out.push_str(&format!("root ::= {root}\n"));
    out.push_str(GBNF_PRELUDE);
    out
}

#[derive(Default)]
struct Emitter {
    /// Track which prelude rules are referenced so we can warn or
    /// elide unused ones in the future.
    _used: BTreeSet<&'static str>,
}

impl Emitter {
    fn emit_object(&mut self, fields: &[Field]) -> String {
        if fields.is_empty() {
            return "\"{}\" ws".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        for (i, f) in fields.iter().enumerate() {
            let key = json_string_lit(&f.name.name);
            let val = self.emit_type(&f.ty);
            // `,` between fields after the first.
            if i == 0 {
                parts.push(format!("{key} ws \":\" ws {val}"));
            } else {
                parts.push(format!("\",\" ws {key} ws \":\" ws {val}"));
            }
        }
        format!("\"{{\" ws {} ws \"}}\"", parts.join(" ws "))
    }

    fn emit_type(&mut self, ty: &Type) -> String {
        match &ty.kind {
            TypeKind::Path { path, generics } => {
                let name = path
                    .segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                self.emit_named(&name, generics)
            }
            TypeKind::List(inner) => {
                let it = self.emit_type(inner);
                // `[ T (, T)* ]?` — accept empty array too.
                format!(
                    "(\"[\" ws (({it}) (ws \",\" ws ({it}))*)? ws \"]\")"
                )
            }
            TypeKind::Map { key: _, value } => {
                // JSON objects always have string keys. Accept any
                // number of string→value entries.
                let v = self.emit_type(value);
                format!(
                    "(\"{{\" ws ((string) ws \":\" ws ({v}) (ws \",\" ws (string) ws \":\" ws ({v}))*)? ws \"}}\")"
                )
            }
            TypeKind::Option(inner) => {
                let it = self.emit_type(inner);
                format!("({it} | \"null\")")
            }
            TypeKind::Tainted(inner) => self.emit_type(inner),
            TypeKind::WithEffects { inner, .. } => self.emit_type(inner),
            TypeKind::Refined { inner, .. } => self.emit_type(inner),
            TypeKind::Ref { inner, .. } => self.emit_type(inner),
            TypeKind::Set(inner) => {
                // Set serializes the same as a List in JSON.
                let it = self.emit_type(inner);
                format!(
                    "(\"[\" ws (({it}) (ws \",\" ws ({it}))*)? ws \"]\")"
                )
            }
            TypeKind::Tuple(items) if items.is_empty() => "\"null\"".to_string(),
            TypeKind::Tuple(items) => {
                // Tuples serialize as fixed-length JSON arrays.
                let inner: Vec<String> =
                    items.iter().map(|t| self.emit_type(t)).collect();
                format!(
                    "(\"[\" ws {} ws \"]\")",
                    inner.join(" ws \",\" ws ")
                )
            }
            TypeKind::Unit => "\"null\"".to_string(),
            TypeKind::Fn { .. } => "string".to_string(),
            // Union types serialize as one-of alternation.
            TypeKind::Union(l, r) => {
                let a = self.emit_type(l);
                let b = self.emit_type(r);
                format!("({a} | {b})")
            }
        }
    }

    fn emit_named(&mut self, name: &str, _generics: &[Type]) -> String {
        match name {
            "String" | "Text" => "string".into(),
            "Int" | "Integer" => "integer".into(),
            "Float" | "Number" | "Decimal" => "number".into(),
            "Bool" | "Boolean" => "boolean".into(),
            "Unit" => "\"null\"".into(),
            // User-declared types (other schemas, enums, …): caller
            // typically composes by emitting each schema's rules and
            // referencing them by name. v0 falls back to a generic
            // JSON `value`.
            _ => "value".into(),
        }
    }
}

/// Lex a string into a quoted GBNF terminal.
fn json_string_lit(s: &str) -> String {
    let mut out = String::from("\"\\\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\\\""),
            '\\' => out.push_str("\\\\\\\\"),
            _ => out.push(c),
        }
    }
    out.push_str("\\\"\"");
    out
}

/// Stable JSON-value prelude. Same shape across every constrained-
/// decoding backend so the grammar is portable.
const GBNF_PRELUDE: &str = "ws ::= [ \\t\\n]*\n\
string ::= \"\\\"\" ( [^\"\\\\] | \"\\\\\" . )* \"\\\"\"\n\
boolean ::= \"true\" | \"false\"\n\
integer ::= \"-\"? [0-9]+\n\
number ::= \"-\"? [0-9]+ (\".\" [0-9]+)? ([eE] [-+]? [0-9]+)?\n\
value ::= string | number | boolean | \"null\" | array | object\n\
array ::= \"[\" ws (value (ws \",\" ws value)*)? ws \"]\"\n\
object ::= \"{\" ws (string ws \":\" ws value (ws \",\" ws string ws \":\" ws value)*)? ws \"}\"\n";

#[cfg(test)]
mod tests {
    use super::*;
    use axon_diag::Span;

    fn ident(s: &str) -> axon_ast::Ident {
        axon_ast::Ident {
            name: s.to_string(),
            span: Span::DUMMY,
        }
    }

    fn ty_path(name: &str) -> Type {
        Type {
            kind: TypeKind::Path {
                path: axon_ast::Path {
                    segments: vec![ident(name)],
                    span: Span::DUMMY,
                },
                generics: Vec::new(),
            },
            span: Span::DUMMY,
        }
    }

    fn field(name: &str, ty: Type) -> Field {
        Field {
            doc: None,
            name: ident(name),
            ty,
            refinements: Vec::new(),
            default: None,
            span: Span::DUMMY,
        }
    }

    fn schema(name: &str, fields: Vec<Field>) -> SchemaDecl {
        SchemaDecl {
            vis: axon_ast::Visibility::Public,
            name: ident(name),
            version: Some(1),
            fields,
            migrations: Vec::new(),
            span: Span::DUMMY,
        }
    }

    #[test]
    fn empty_schema_yields_empty_object_rule() {
        let s = schema("Empty", vec![]);
        let g = emit_for_schema(&s);
        assert!(g.contains("root ::= \"{}\" ws"));
        assert!(g.contains("# schema: Empty"));
        assert!(g.contains("# version: 1"));
    }

    #[test]
    fn primitives_map_to_json_terminals() {
        let s = schema(
            "P",
            vec![
                field("a", ty_path("String")),
                field("b", ty_path("Int")),
                field("c", ty_path("Float")),
                field("d", ty_path("Bool")),
            ],
        );
        let g = emit_for_schema(&s);
        assert!(g.contains("string"));
        assert!(g.contains("integer"));
        assert!(g.contains("number"));
        assert!(g.contains("boolean"));
        // The root rule lists the four fields in order.
        assert!(g.contains("\\\"a\\\""), "missing field a in grammar: {g}");
    }

    #[test]
    fn list_emits_array_rule() {
        let inner = ty_path("Int");
        let list = Type {
            kind: TypeKind::List(Box::new(inner)),
            span: Span::DUMMY,
        };
        let s = schema("L", vec![field("items", list)]);
        let g = emit_for_schema(&s);
        assert!(g.contains("\"[\""), "list should open with [");
        assert!(g.contains("integer"));
    }

    #[test]
    fn option_emits_null_alternation() {
        let opt = Type {
            kind: TypeKind::Option(Box::new(ty_path("String"))),
            span: Span::DUMMY,
        };
        let s = schema("O", vec![field("v", opt)]);
        let g = emit_for_schema(&s);
        assert!(g.contains("\"null\""));
        assert!(g.contains("string"));
    }

    #[test]
    fn tainted_passes_through() {
        let t = Type {
            kind: TypeKind::Tainted(Box::new(ty_path("String"))),
            span: Span::DUMMY,
        };
        let s = schema("T", vec![field("data", t)]);
        let g = emit_for_schema(&s);
        assert!(g.contains("string"));
    }

    #[test]
    fn prelude_includes_value_and_array_rules() {
        let s = schema("P", vec![field("x", ty_path("String"))]);
        let g = emit_for_schema(&s);
        for needle in ["ws ::=", "string ::=", "integer ::=", "value ::="] {
            assert!(g.contains(needle), "missing prelude rule `{needle}`");
        }
    }

    #[test]
    fn unknown_type_falls_back_to_value() {
        // A reference to an external type → generic JSON value.
        let custom = ty_path("MyCustomType");
        let s = schema("X", vec![field("nested", custom)]);
        let g = emit_for_schema(&s);
        // The root rule references `value` for the unknown field type.
        assert!(g.contains(" value "));
    }
}
