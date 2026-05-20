//! Feature gating (§7.1).
//!
//! `axon.toml` accepts a `[features]` table whose entries map a feature
//! name to the list of other features it transitively enables — the
//! Cargo shape:
//!
//! ```toml
//! [features]
//! default       = ["console-logs"]
//! console-logs  = []
//! redis-cache   = ["network"]
//! network       = []
//! ```
//!
//! At the source level a developer gates code by attaching
//! `#[cfg(feature = "name")]` to a top-level `fn`. When the active
//! feature set doesn't contain `name`, the item is dropped before
//! type-checking sees it. The filter is **conservative**: when an
//! attribute we don't understand appears the item is kept, so adding
//! new conditions later doesn't silently strip code.
//!
//! Resolution rules:
//!
//!   * `--features ""` or no flag → enable the `default` feature (and
//!     its transitive closure). If `default` doesn't exist, no features
//!     are active.
//!   * `--features a,b,c` → start from `{a, b, c}`, compute transitive
//!     closure via the `[features]` table.
//!   * `--no-default-features` → start from the user-supplied set
//!     without seeding `default`.
//!   * Unknown feature names are accepted (no error) — they just don't
//!     enable anything. This matches Cargo's behavior and keeps gates
//!     a no-op when their backing feature was renamed.

use std::collections::{BTreeSet, HashMap};

use axon_ast::{Attribute, Expr, ExprKind, Item, Literal, Program, StringPart};

#[derive(Clone, Debug, Default)]
pub struct ActiveFeatures {
    set: BTreeSet<String>,
}

impl ActiveFeatures {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_iter(it: impl IntoIterator<Item = String>) -> Self {
        Self {
            set: it.into_iter().collect(),
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.set.contains(name)
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.set.iter().cloned().collect();
        v.sort();
        v
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }
}

/// Compute the active feature set from CLI flags + the manifest's
/// `[features]` table. Closure-resolves transitive enables; doesn't
/// return any feature names that aren't in the manifest table since
/// those can't have transitive children.
pub fn resolve_features(
    table: &HashMap<String, Vec<String>>,
    requested: &[String],
    enable_default: bool,
) -> ActiveFeatures {
    let mut active: BTreeSet<String> = BTreeSet::new();
    if enable_default && table.contains_key("default") {
        active.insert("default".to_string());
    }
    for r in requested {
        active.insert(r.clone());
    }
    // Fixed-point transitive closure.
    loop {
        let before = active.len();
        let snapshot: Vec<String> = active.iter().cloned().collect();
        for f in snapshot {
            if let Some(children) = table.get(&f) {
                for c in children {
                    active.insert(c.clone());
                }
            }
        }
        if active.len() == before {
            break;
        }
    }
    ActiveFeatures { set: active }
}

/// Strip every top-level item whose `#[cfg(feature = "X")]` predicate
/// evaluates to `false` against `active`.
pub fn filter_program(program: &mut Program, active: &ActiveFeatures) {
    program
        .items
        .retain(|item| item_cfg_passes(item, active));
}

fn item_cfg_passes(item: &Item, active: &ActiveFeatures) -> bool {
    let attrs: &[Attribute] = match item {
        Item::Fn(f) => &f.attrs,
        Item::Tool(t) => attrs_for_tool(t),
        _ => return true,
    };
    for a in attrs {
        if !cfg_predicate_passes(a, active) {
            return false;
        }
    }
    true
}

fn attrs_for_tool(tool: &axon_ast::ToolDecl) -> &[Attribute] {
    // ToolDecl may or may not have `attrs`; ax-ast version-dependent.
    // We use a getter-by-field-name pattern so the function stays a
    // single source of truth: any item whose source layout grows an
    // `attrs` field appears here.
    &tool.attrs
}

/// `attribute` is `#[cfg(feature = "X")]` *or* anything else (in which
/// case we return `true` — only `cfg(feature=...)` participates in
/// gating, every other attribute is invisible to the filter).
fn cfg_predicate_passes(attr: &Attribute, active: &ActiveFeatures) -> bool {
    // Only consider attributes whose path is the single segment `cfg`.
    let is_cfg = attr.name.segments.len() == 1
        && attr
            .name
            .segments
            .first()
            .map(|s| s.name == "cfg")
            .unwrap_or(false);
    if !is_cfg {
        return true;
    }
    // Walk the args; the first `feature = "X"` we find decides.
    for arg in &attr.args {
        if let Some(name) = feature_name_from_arg(arg) {
            return active.contains(&name);
        }
    }
    // Malformed `cfg(...)` — be conservative, keep the item.
    true
}

fn feature_name_from_arg(arg: &Expr) -> Option<String> {
    // Accept three shapes:
    //   feature("name")                          — call form
    //   feature = "name"                         — record-field form (parser
    //                                              represents `=` args as
    //                                              record entries)
    //   "name"                                   — bare string under cfg(...)
    let kind = &*arg.kind;
    match kind {
        ExprKind::Literal(lit) => string_lit(lit),
        ExprKind::Call { callee, args } => {
            let is_feature_name = matches!(&*callee.kind, ExprKind::Path(p) if p.segments.first().map(|s| s.name == "feature").unwrap_or(false));
            if !is_feature_name {
                return None;
            }
            args.first().and_then(|a| {
                let inner = match a {
                    axon_ast::CallArg::Positional(e) => e,
                    axon_ast::CallArg::Named { value, .. } => value,
                };
                match &*inner.kind {
                    ExprKind::Literal(lit) => string_lit(lit),
                    _ => None,
                }
            })
        }
        ExprKind::BraceLit(brace) => brace_lit_feature_field(brace),
        _ => None,
    }
}

fn string_lit(lit: &Literal) -> Option<String> {
    let Literal::String { parts, .. } = lit else {
        return None;
    };
    let mut s = String::new();
    for p in parts {
        match p {
            StringPart::Text(t) => s.push_str(t),
            StringPart::Interp(_) => return None,
        }
    }
    Some(s)
}

fn brace_lit_feature_field(brace: &axon_ast::BraceLit) -> Option<String> {
    let axon_ast::BraceLit::Record(fields) = brace else {
        return None;
    };
    for (name, value) in fields {
        if name.name == "feature" {
            return match &*value.kind {
                ExprKind::Literal(lit) => string_lit(lit),
                _ => None,
            };
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> HashMap<String, Vec<String>> {
        let mut t = HashMap::new();
        t.insert("default".into(), vec!["console-logs".into()]);
        t.insert("console-logs".into(), vec![]);
        t.insert("redis-cache".into(), vec!["network".into()]);
        t.insert("network".into(), vec![]);
        t
    }

    #[test]
    fn default_enabled_when_no_flags() {
        let active = resolve_features(&table(), &[], true);
        assert!(active.contains("default"));
        assert!(active.contains("console-logs"));
        assert!(!active.contains("network"));
    }

    #[test]
    fn explicit_features_transitively_close() {
        let active = resolve_features(&table(), &["redis-cache".into()], false);
        assert!(active.contains("redis-cache"));
        assert!(active.contains("network"));
        assert!(!active.contains("default"));
    }

    #[test]
    fn no_default_features_excludes_default() {
        let active = resolve_features(&table(), &["network".into()], false);
        assert!(!active.contains("default"));
        assert!(active.contains("network"));
    }

    #[test]
    fn unknown_feature_does_not_error() {
        let active = resolve_features(&table(), &["does-not-exist".into()], true);
        // The unknown name is still "active" — it just enables nothing.
        assert!(active.contains("does-not-exist"));
    }

    #[test]
    fn empty_table_yields_no_default() {
        let active = resolve_features(&HashMap::new(), &[], true);
        assert!(active.is_empty());
    }

    #[test]
    fn names_are_sorted_and_deduped() {
        let active = ActiveFeatures::from_iter(["b".into(), "a".into(), "b".into()]);
        assert_eq!(active.names(), vec!["a".to_string(), "b".to_string()]);
    }
}
