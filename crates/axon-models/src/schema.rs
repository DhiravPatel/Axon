//! JSON-Schema lowering for `generate<S>`.
//!
//! The runtime hands us an `axon_ast::Type` representing `S`. We lower it
//! into the JSON Schema shape Anthropic's tool input wants. Today this
//! supports the common cases — primitives, lists, maps, and named schemas
//! whose fields are themselves drawn from this same set.
//!
//! The schema lowering deliberately doesn't need an `axon-ast` dependency
//! here — `axon-runtime` does the resolution and hands us a fully-rendered
//! `serde_json::Value`. This module exposes a tiny helper for the common
//! "look up a name → primitive" decision the runtime needs.

use serde_json::{json, Value};

/// Synthesize a JSON Schema for a *primitive* type by name.
///
/// Returns `None` for names that need additional structure (records,
/// schemas, lists, maps, etc.) — the runtime builds those itself by
/// consulting the user's declarations.
pub fn ast_type_to_json_schema(name: &str) -> Option<Value> {
    Some(match name {
        "Int" => json!({ "type": "integer" }),
        "Float" => json!({ "type": "number" }),
        "Decimal" => json!({ "type": "string", "description": "Exact decimal as a numeric string." }),
        "Bool" => json!({ "type": "boolean" }),
        "String" => json!({ "type": "string" }),
        "Bytes" => json!({ "type": "string", "contentEncoding": "base64" }),
        "Date" => json!({ "type": "string", "format": "date" }),
        "DateTime" => json!({ "type": "string", "format": "date-time" }),
        "Time" => json!({ "type": "string", "format": "time" }),
        "Money" => json!({ "type": "string", "description": "Amount-and-currency as written, e.g. `1.50usd`." }),
        "Duration" => json!({ "type": "string", "description": "Duration as written, e.g. `30s`." }),
        "Unit" => json!({ "type": "null" }),
        _ => return None,
    })
}
