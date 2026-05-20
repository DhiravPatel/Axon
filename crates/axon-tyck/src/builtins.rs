//! Built-in name table.
//!
//! Names that look like ordinary identifiers in source but resolve to
//! primitive [`Ty`] values without an item declaration: `Int`, `Float`,
//! `Bool`, `String`, `Tainted`, `Tool`, `Stream`, etc.

use axon_types::Ty;

/// Resolve a top-level type name to its built-in [`Ty`] if any. Returns
/// `None` for names that must come from the user's item table.
pub fn builtin_type(name: &str) -> Option<Ty> {
    Some(match name {
        "Int" => Ty::Int,
        "Float" => Ty::Float,
        "Decimal" => Ty::Decimal,
        "Money" => Ty::Money,
        "Duration" => Ty::Duration,
        "Date" => Ty::Date,
        "DateTime" => Ty::DateTime,
        "Time" => Ty::Time,
        "Bool" => Ty::Bool,
        "Char" => Ty::Char,
        "String" => Ty::String,
        "Bytes" => Ty::Bytes,
        "Unit" => Ty::Unit,
        "Never" => Ty::Never,
        "dyn" => Ty::Dyn,
        "Model" => Ty::Model,
        "Memory" => Ty::Memory,
        "ContentHash" => Ty::ContentHash,
        "AgentAddr" => Ty::AgentAddr,
        _ => return None,
    })
}

/// True if `name` is the name of a *parametric* built-in container — the
/// type-lowering pass routes these through dedicated arms in `lower.rs`.
pub fn is_builtin_container(name: &str) -> bool {
    matches!(
        name,
        "Option"
            | "List"
            | "Map"
            | "Set"
            | "Tool"
            | "Stream"
            | "Chan"
            | "Secret"
            | "Tainted"
            | "Result"
    )
}

/// True if `name` is a known effect atom in the language's built-in effect
/// set. Right now this is informational only — declarations may use any
/// camel-case identifier in their `uses` row, since §20 leaves the atom set
/// open. We surface a warning when the inferred row includes an effect that
/// isn't built-in *and* isn't in scope as a user-declared effect.
#[allow(dead_code)]
pub fn is_builtin_effect(name: &str) -> bool {
    matches!(
        name,
        "Pure"
            | "Net"
            | "Fs"
            | "Fs.Read"
            | "Fs.Write"
            | "Time"
            | "Random"
            | "Spawn"
            | "Channel"
            | "LLM"
            | "Tool"
            | "Memory"
            | "Console"
            | "Log"
            | "Process"
            | "Env"
            | "Db"
            | "Db.Read"
            | "Db.Write"
            | "Crypto"
            | "Audit"
    )
}
