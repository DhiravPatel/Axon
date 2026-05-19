//! Static documentation generator for Axon projects.
//!
//! `axon doc <project> -o site/` walks every loaded module, pairs each
//! top-level item with any `///` doc comments immediately preceding it,
//! and emits one HTML page per module plus an `index.html` listing them.
//!
//! Doc comments are interpreted as CommonMark via `pulldown-cmark`. The
//! HTML is intentionally framework-free — a single embedded stylesheet,
//! semantic markup, no JavaScript. Drop the output directory behind any
//! static host (or open it locally).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use axon_ast::{Item, Visibility};
use axon_lexer::{TokenKind};
use axon_project::{LoadedProject, ModuleFile};

const STYLESHEET: &str = r#"
* { box-sizing: border-box; }
body {
    font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
    line-height: 1.55;
    max-width: 920px;
    margin: 2rem auto;
    padding: 0 1.25rem;
    color: #1f2328;
}
h1, h2, h3 { font-weight: 600; line-height: 1.25; }
h1 { font-size: 2rem; border-bottom: 1px solid #d0d7de; padding-bottom: 0.4rem; }
h2 { font-size: 1.4rem; margin-top: 2.2rem; }
h3 { font-size: 1.1rem; margin-top: 1.5rem; }
nav.crumbs { font-size: 0.92rem; color: #57606a; margin-bottom: 0.6rem; }
nav.crumbs a { color: #0969da; text-decoration: none; }
nav.crumbs a:hover { text-decoration: underline; }
code, pre { font-family: ui-monospace, SFMono-Regular, Consolas, monospace; }
code { background: #f6f8fa; padding: 0.1rem 0.35rem; border-radius: 4px; font-size: 0.9em; }
pre { background: #f6f8fa; padding: 0.9rem 1rem; border-radius: 6px; overflow-x: auto; }
pre code { background: transparent; padding: 0; }
.item { border: 1px solid #d0d7de; border-radius: 8px; padding: 1rem 1.2rem; margin: 1rem 0; }
.item h3 { margin-top: 0; }
.kind {
    display: inline-block; font-size: 0.78rem; font-weight: 600;
    background: #ddf4ff; color: #0969da; padding: 0.1rem 0.55rem;
    border-radius: 999px; margin-left: 0.5rem; vertical-align: middle;
}
.kind.private { background: #f6f8fa; color: #57606a; }
ul.modules { list-style: none; padding: 0; }
ul.modules li { padding: 0.45rem 0; border-bottom: 1px solid #eaeef2; }
ul.modules li:last-child { border-bottom: none; }
ul.modules a { color: #0969da; text-decoration: none; font-weight: 500; }
footer { color: #57606a; font-size: 0.85rem; margin-top: 3rem; border-top: 1px solid #d0d7de; padding-top: 1rem; }
"#;

#[derive(Debug)]
pub struct GeneratedSite {
    pub files: HashMap<PathBuf, String>,
}

impl GeneratedSite {
    /// Write every file to disk under `output_dir`. Creates parent
    /// directories as needed and overwrites existing files.
    pub fn write_to(&self, output_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(output_dir)?;
        for (rel, contents) in &self.files {
            let path = output_dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, contents)?;
        }
        Ok(())
    }
}

/// Build a documentation site for an already-loaded project. Returns the
/// generated files in memory so callers can either write them to disk via
/// [`GeneratedSite::write_to`] or feed them to a different sink (tests
/// inspect them directly).
pub fn generate(project: &LoadedProject) -> GeneratedSite {
    let mut files: HashMap<PathBuf, String> = HashMap::new();
    files.insert(PathBuf::from("style.css"), STYLESHEET.to_string());

    let mut modules: Vec<&ModuleFile> = project.modules.iter().collect();
    modules.sort_by(|a, b| a.module_path.cmp(&b.module_path));

    let project_name = if project.manifest.package.name.is_empty() {
        "Project"
    } else {
        &project.manifest.package.name
    };

    files.insert(
        PathBuf::from("index.html"),
        render_index(project_name, &modules),
    );

    for m in &modules {
        let doc_pairs = extract_doc_pairs(m);
        let rel = module_path_to_html(&m.module_path);
        files.insert(rel, render_module(project_name, m, &doc_pairs));
    }
    GeneratedSite { files }
}

// ---------------------------------------------------------------------------
// Doc-comment association
// ---------------------------------------------------------------------------

/// One item + the `///` comments that preceded it.
#[derive(Clone, Debug)]
pub struct ItemDoc {
    pub item_index: usize,
    pub doc: String,
}

/// Walk `module`'s tokens and pair each top-level item with the doc text
/// from any `///` comments that immediately preceded it (separated only
/// by newlines and `//`/`/* */` comments — but not by any non-trivia
/// token).
pub fn extract_doc_pairs(module: &ModuleFile) -> Vec<ItemDoc> {
    let (tokens, _) = axon_lexer::tokenize(&module.source);
    let mut pending: Vec<String> = Vec::new();
    let mut pairs: Vec<ItemDoc> = Vec::new();
    let mut item_iter = module.program.items.iter().enumerate().peekable();
    let mut next_item_start = item_iter
        .peek()
        .map(|(_, it)| it.span().start as usize)
        .unwrap_or(u32::MAX as usize);

    for tok in &tokens {
        let pos = tok.span.start as usize;
        // Once a token reaches the next item's start, dump pending docs
        // onto that item and advance.
        if pos >= next_item_start {
            if let Some((idx, _)) = item_iter.next() {
                if !pending.is_empty() {
                    pairs.push(ItemDoc {
                        item_index: idx,
                        doc: pending.join("\n").trim().to_string(),
                    });
                    pending.clear();
                }
                next_item_start = item_iter
                    .peek()
                    .map(|(_, it)| it.span().start as usize)
                    .unwrap_or(u32::MAX as usize);
            }
        }
        match &tok.kind {
            TokenKind::DocComment(text) => pending.push(text.clone()),
            TokenKind::ModDocComment(_)
            | TokenKind::LineComment
            | TokenKind::BlockComment
            | TokenKind::Newline => {}
            // Item-prefix tokens (`pub`, `async`, `@attribute`, `#[...]`)
            // come *between* the doc comments and the item's recorded
            // span start. They don't reset the pending docs — they're
            // part of the upcoming item.
            TokenKind::Keyword(axon_lexer::Keyword::Pub)
            | TokenKind::Keyword(axon_lexer::Keyword::Async)
            | TokenKind::At
            | TokenKind::HashLBracket => {}
            _ => {
                // Any other token before the item resets the pending docs
                // (they belonged to whatever's behind us, not the next
                // item).
                pending.clear();
            }
        }
    }
    pairs
}

// ---------------------------------------------------------------------------
// HTML rendering
// ---------------------------------------------------------------------------

fn render_index(project_name: &str, modules: &[&ModuleFile]) -> String {
    let mut s = String::new();
    push_header(&mut s, project_name, project_name, &[]);
    s.push_str(&format!("<h1>{project_name}</h1>\n"));
    s.push_str("<p class=\"intro\">Module index:</p>\n<ul class=\"modules\">\n");
    for m in modules {
        let name = if m.module_path.is_empty() {
            "(root)".to_string()
        } else {
            m.module_path.clone()
        };
        let href = module_path_to_html(&m.module_path);
        let href = href.to_string_lossy();
        s.push_str(&format!(
            "<li><a href=\"{}\">{}</a> &mdash; <code>{}</code></li>\n",
            html_escape(&href),
            html_escape(&name),
            html_escape(&m.file_path.display().to_string()),
        ));
    }
    s.push_str("</ul>\n");
    push_footer(&mut s);
    s
}

fn render_module(project_name: &str, module: &ModuleFile, docs: &[ItemDoc]) -> String {
    let display_name = if module.module_path.is_empty() {
        "(root)".to_string()
    } else {
        module.module_path.clone()
    };
    let mut s = String::new();
    push_header(
        &mut s,
        &format!("{display_name} — {project_name}"),
        project_name,
        &[("../index.html", project_name), ("", &display_name)],
    );
    s.push_str(&format!(
        "<h1>{} <span class=\"kind\">module</span></h1>\n",
        html_escape(&display_name)
    ));
    s.push_str(&format!(
        "<p><small><code>{}</code></small></p>\n",
        html_escape(&module.file_path.display().to_string())
    ));

    let doc_by_idx: HashMap<usize, String> =
        docs.iter().map(|d| (d.item_index, d.doc.clone())).collect();

    for (i, item) in module.program.items.iter().enumerate() {
        if let Some(rendered) = render_item(item, doc_by_idx.get(&i)) {
            s.push_str(&rendered);
        }
    }
    push_footer(&mut s);
    s
}

fn render_item(item: &Item, doc: Option<&String>) -> Option<String> {
    let (name, kind, signature, vis) = describe(item)?;
    let body_md = doc.cloned().unwrap_or_default();
    let body_html = markdown_to_html(&body_md);
    let kind_class = match vis {
        Visibility::Public => "kind",
        Visibility::Private => "kind private",
    };
    let mut out = String::new();
    out.push_str("<section class=\"item\">\n");
    out.push_str(&format!(
        "<h3><code>{}</code> <span class=\"{kind_class}\">{}</span></h3>\n",
        html_escape(&name),
        html_escape(kind),
    ));
    out.push_str(&format!(
        "<pre><code>{}</code></pre>\n",
        html_escape(&signature)
    ));
    if !body_html.trim().is_empty() {
        out.push_str(&body_html);
    }
    out.push_str("</section>\n");
    Some(out)
}

/// Return `(name, kind label, source-style signature, visibility)` for
/// each documentable item kind. Items we don't yet render (use, impl)
/// return None and get filtered out.
fn describe(item: &Item) -> Option<(String, &'static str, String, Visibility)> {
    Some(match item {
        Item::Fn(f) => (
            f.name.name.clone(),
            "fn",
            render_fn_signature(f),
            f.vis,
        ),
        Item::Type(t) => (
            t.name.name.clone(),
            "type",
            render_type_signature(t),
            t.vis,
        ),
        Item::Schema(s) => (
            s.name.name.clone(),
            "schema",
            render_schema_signature(s),
            s.vis,
        ),
        Item::Agent(a) => (
            a.name.name.clone(),
            "agent",
            render_agent_signature(&a.name.name, &a.params, &a.members),
            Visibility::Public,
        ),
        Item::Actor(a) => (
            a.name.name.clone(),
            "actor",
            render_agent_signature(&a.name.name, &a.params, &a.members),
            Visibility::Public,
        ),
        Item::Tool(t) => (
            t.name.name.clone(),
            "tool",
            render_tool_signature(t),
            Visibility::Public,
        ),
        Item::Model(m) => (
            m.name.name.clone(),
            "model",
            format!("model {} = ...", m.name.name),
            Visibility::Public,
        ),
        Item::Memory(m) => (
            m.name.name.clone(),
            "memory",
            format!("memory {} = ...", m.name.name),
            Visibility::Public,
        ),
        Item::Prompt(p) => (
            p.name.name.clone(),
            "prompt",
            format!("prompt {}(...) -> ...", p.name.name),
            Visibility::Public,
        ),
        Item::Trait(t) => (
            t.name.name.clone(),
            "trait",
            format!("trait {}", t.name.name),
            t.vis,
        ),
        Item::Const(c) => (
            c.name.name.clone(),
            "const",
            format!("const {} = ...", c.name.name),
            c.vis,
        ),
        _ => return None,
    })
}

fn render_fn_signature(f: &axon_ast::FnDecl) -> String {
    let mut s = String::new();
    if matches!(f.vis, Visibility::Public) {
        s.push_str("pub ");
    }
    s.push_str("fn ");
    s.push_str(&f.name.name);
    s.push('(');
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name.name, render_type(&p.ty)))
        .collect();
    s.push_str(&params.join(", "));
    s.push(')');
    if let Some(rt) = &f.return_type {
        s.push_str(" -> ");
        s.push_str(&render_type(rt));
    }
    if let Some(row) = &f.effect_row {
        s.push_str(" uses ");
        s.push_str(&render_effect_row(row));
    }
    s
}

fn render_type_signature(t: &axon_ast::TypeDecl) -> String {
    let pub_prefix = if matches!(t.vis, Visibility::Public) {
        "pub "
    } else {
        ""
    };
    match &t.body {
        axon_ast::TypeDeclBody::Record(fields) => {
            let body: Vec<String> = fields
                .iter()
                .map(|f| format!("    {}: {}", f.name.name, render_type(&f.ty)))
                .collect();
            format!(
                "{pub_prefix}type {} {{\n{}\n}}",
                t.name.name,
                body.join(",\n")
            )
        }
        axon_ast::TypeDeclBody::Sum(variants) => {
            let body: Vec<String> = variants.iter().map(|v| v.name.name.clone()).collect();
            format!(
                "{pub_prefix}type {} = {}",
                t.name.name,
                body.join(" | ")
            )
        }
        axon_ast::TypeDeclBody::Alias(inner) => {
            format!(
                "{pub_prefix}type {} = {}",
                t.name.name,
                render_type(inner)
            )
        }
        axon_ast::TypeDeclBody::Newtype { inner, .. } => format!(
            "{pub_prefix}type {} = {} @nominal",
            t.name.name,
            render_type(inner)
        ),
    }
}

fn render_schema_signature(s: &axon_ast::SchemaDecl) -> String {
    let pub_prefix = if matches!(s.vis, Visibility::Public) {
        "pub "
    } else {
        ""
    };
    let body: Vec<String> = s
        .fields
        .iter()
        .map(|f| format!("    {}: {}", f.name.name, render_type(&f.ty)))
        .collect();
    let version = s
        .version
        .map(|v| format!(" @version({v})"))
        .unwrap_or_default();
    format!(
        "{pub_prefix}schema {}{} {{\n{}\n}}",
        s.name.name,
        version,
        body.join(",\n")
    )
}

fn render_agent_signature(
    name: &str,
    params: &[axon_ast::Param],
    members: &[axon_ast::AgentMember],
) -> String {
    let p: Vec<String> = params
        .iter()
        .map(|p| format!("{}: {}", p.name.name, render_type(&p.ty)))
        .collect();
    let handlers: Vec<String> = members
        .iter()
        .filter_map(|m| match m {
            axon_ast::AgentMember::Handler(h) => Some(format!("    on {}(...)", h.name.name)),
            _ => None,
        })
        .collect();
    let mut body = String::new();
    if !handlers.is_empty() {
        body.push_str(" {\n");
        body.push_str(&handlers.join("\n"));
        body.push_str("\n}");
    }
    format!("agent {}({}){}", name, p.join(", "), body)
}

fn render_tool_signature(t: &axon_ast::ToolDecl) -> String {
    let p: Vec<String> = t
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name.name, render_type(&p.ty)))
        .collect();
    let mut s = format!("tool {}({}) -> {}", t.name.name, p.join(", "), render_type(&t.return_type));
    if let Some(row) = &t.effect_row {
        s.push_str(" uses ");
        s.push_str(&render_effect_row(row));
    }
    s
}

fn render_type(t: &axon_ast::Type) -> String {
    use axon_ast::TypeKind::*;
    match &t.kind {
        Path { path, generics } => {
            let name = path
                .segments
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(".");
            if generics.is_empty() {
                name
            } else {
                let g: Vec<String> = generics.iter().map(render_type).collect();
                format!("{}<{}>", name, g.join(", "))
            }
        }
        List(inner) => format!("[{}]", render_type(inner)),
        Map { key, value } => format!("{{{}: {}}}", render_type(key), render_type(value)),
        Set(inner) => format!("{{{}}}", render_type(inner)),
        Tuple(xs) => {
            let parts: Vec<String> = xs.iter().map(render_type).collect();
            format!("({})", parts.join(", "))
        }
        Unit => "()".to_string(),
        Ref { is_mut, inner } => {
            let m = if *is_mut { "mut " } else { "" };
            format!("&{m}{}", render_type(inner))
        }
        Tainted(inner) => format!("Tainted<{}>", render_type(inner)),
        Option(inner) => format!("{}?", render_type(inner)),
        Refined { inner, .. } => render_type(inner),
        Union(a, b) => format!("{} | {}", render_type(a), render_type(b)),
        Fn { params, return_type, effects } => {
            let p: Vec<String> = params.iter().map(|p| render_type(&p.ty)).collect();
            let mut s = format!("({}) -> {}", p.join(", "), render_type(return_type));
            if let Some(row) = effects {
                s.push_str(" uses ");
                s.push_str(&render_effect_row(row));
            }
            s
        }
        WithEffects { inner, effects } => {
            format!("{} uses {}", render_type(inner), render_effect_row(effects))
        }
    }
}

fn render_effect_row(row: &axon_ast::EffectRow) -> String {
    let parts: Vec<String> = row
        .effects
        .iter()
        .map(|e| {
            e.path
                .segments
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect();
    format!("{{ {} }}", parts.join(", "))
}

fn markdown_to_html(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(md, opts);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

fn push_header(out: &mut String, title: &str, _project: &str, crumbs: &[(&str, &str)]) {
    out.push_str("<!doctype html>\n<html lang=\"en\"><head>\n");
    out.push_str(&format!("<title>{}</title>\n", html_escape(title)));
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<link rel=\"stylesheet\" href=\"style.css\">\n");
    out.push_str("</head><body>\n");
    if !crumbs.is_empty() {
        out.push_str("<nav class=\"crumbs\">");
        for (i, (href, label)) in crumbs.iter().enumerate() {
            if i > 0 {
                out.push_str(" / ");
            }
            if href.is_empty() {
                out.push_str(&html_escape(label));
            } else {
                out.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    html_escape(href),
                    html_escape(label)
                ));
            }
        }
        out.push_str("</nav>\n");
    }
}

fn push_footer(out: &mut String) {
    out.push_str("<footer>Generated by <code>axon doc</code>.</footer>\n");
    out.push_str("</body></html>\n");
}

fn module_path_to_html(module_path: &str) -> PathBuf {
    if module_path.is_empty() {
        PathBuf::from("module-root.html")
    } else {
        PathBuf::from(format!("{}.html", module_path.replace('.', "-")))
    }
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}
