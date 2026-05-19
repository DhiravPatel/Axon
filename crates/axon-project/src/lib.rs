//! Manifest parsing and multi-file project loading for Axon.
//!
//! A *project* is a directory containing an `axon.toml` and a `src/`
//! directory of `.ax` source files. Each file is a module identified by
//! its path relative to `src/`. Loading produces a [`LoadedProject`]: the
//! manifest, every parsed module, and a unified flat [`axon_ast::Program`]
//! that downstream consumers (the type checker, the runtime, the VM) can
//! treat as a single compilation unit.
//!
//! v0 module semantics are intentionally minimal: items are merged into a
//! single global namespace, with **name-collision detection** across files.
//! Cross-file privacy (`pub`/non-`pub`) lands in 8.5 alongside an explicit
//! resolution pass.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use axon_ast::{Item, Program};
use axon_diag::{Diagnostic, SourceFile, SourceRegistry, Span};
use serde::Deserialize;

pub const MANIFEST_FILENAME: &str = "axon.toml";
pub const DEFAULT_SRC_DIR: &str = "src";
pub const SOURCE_EXTENSION: &str = "ax";

// ===========================================================================
// Manifest
// ===========================================================================

/// In-memory representation of `axon.toml`. Only the fields we *use*
/// today have hard accessors — unrecognized keys are accepted silently so
/// the file is forward-compatible.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub package: Package,
    pub run: Run,
    pub caps: Caps,
    /// Local-path dependencies. `[deps.helpers] path = "../helpers"`.
    /// Network / git deps land in a future stage; for now only local
    /// paths are honored.
    pub deps: HashMap<String, Dependency>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Dependency {
    /// Path to the dep's project directory, relative to the manifest.
    pub path: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Package {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Run {
    /// Name of the entry function. `axon run` calls this; defaults to `main`.
    pub entry: String,
    /// Source directory, relative to the manifest directory. Defaults to
    /// `src`.
    pub src: String,
}

impl Default for Run {
    fn default() -> Self {
        Self {
            entry: "main".into(),
            src: DEFAULT_SRC_DIR.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Caps {
    /// Default capability set granted by `axon run` / `axon test` for this
    /// project. Empty means use the binary's default (`CapSet::standard_default`).
    pub default: Vec<String>,
}

impl Manifest {
    /// Parse a manifest from text. Missing fields are filled by `Default`.
    pub fn parse(toml_text: &str) -> Result<Self, String> {
        toml::from_str(toml_text).map_err(|e| format!("invalid axon.toml: {e}"))
    }

    /// Load `axon.toml` from the directory `dir`. Returns `Default::default()`
    /// if the file is absent — single-file invocations don't require one.
    pub fn from_dir(dir: &Path) -> Result<Self, String> {
        let path = dir.join(MANIFEST_FILENAME);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(_) => Ok(Self::default()),
        }
    }
}

// ===========================================================================
// Module loading
// ===========================================================================

/// One module loaded from disk. The `path` is dotted module form
/// (`foo.bar.baz`), not a filesystem path.
#[derive(Debug, Clone)]
pub struct ModuleFile {
    /// Dotted module path — e.g. `foo.bar` for `src/foo/bar.ax`.
    pub module_path: String,
    /// Filesystem path to the source file.
    pub file_path: PathBuf,
    pub source: SourceFile,
    pub program: Program,
}

/// Result of loading every `.ax` file in a project root.
pub struct LoadedProject {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub modules: Vec<ModuleFile>,
    /// All items from every module concatenated, in stable order
    /// (alphabetical by module path; preserving declaration order within
    /// a file). Suitable for handing to the runtime / type checker
    /// without any further plumbing.
    pub merged: Program,
    pub diagnostics: Vec<Diagnostic>,
    /// Source-file registry. Each module's source has a stable id; spans
    /// inside that module carry the id so multi-file diagnostics render
    /// against the correct file.
    pub sources: SourceRegistry,
}

impl LoadedProject {
    /// Treat `path` as either a single `.ax` file or a project directory
    /// (anything containing `axon.toml` or a `src/` subdir, or any
    /// directory walked as src).
    pub fn load(path: &Path) -> Result<Self, String> {
        if path.is_file() {
            return Self::load_single_file(path);
        }
        if !path.is_dir() {
            return Err(format!(
                "axon: `{}` is neither a file nor a directory",
                path.display()
            ));
        }
        Self::load_directory(path)
    }

    fn load_single_file(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("axon: cannot read `{}`: {e}", path.display()))?;
        let mut sources = SourceRegistry::new();
        let id = sources.register(path.to_string_lossy().into_owned(), text.clone());
        let source = SourceFile::with_id(id, path.to_string_lossy().into_owned(), text);
        let (program, diags) = axon_parser::parse(&source);
        let module = ModuleFile {
            module_path: String::new(),
            file_path: path.to_path_buf(),
            source,
            program: program.clone(),
        };
        let merged = program;
        Ok(LoadedProject {
            root: path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
            manifest: Manifest::default(),
            modules: vec![module],
            merged,
            diagnostics: diags,
            sources,
        })
    }

    fn load_directory(root: &Path) -> Result<Self, String> {
        let manifest = Manifest::from_dir(root)?;
        let mut sources = SourceRegistry::new();

        // Load every dependency first so their modules end up in the
        // registry with stable ids before main-project modules. Deps are
        // namespaced under the dep name (e.g. `helpers.foo` for module
        // `foo` in dep `helpers`).
        let mut modules: Vec<ModuleFile> = Vec::new();
        for (dep_name, dep) in &manifest.deps {
            let dep_root = root.join(&dep.path);
            if !dep_root.is_dir() {
                return Err(format!(
                    "axon: dependency `{dep_name}` path `{}` is not a directory",
                    dep_root.display()
                ));
            }
            let dep_manifest = Manifest::from_dir(&dep_root)?;
            let dep_src = dep_root.join(&dep_manifest.run.src);
            if !dep_src.is_dir() {
                return Err(format!(
                    "axon: dependency `{dep_name}` has no `{}` directory",
                    dep_manifest.run.src
                ));
            }
            let dep_modules =
                collect_modules(&dep_src, &dep_src, Some(dep_name), &mut sources)?;
            modules.extend(dep_modules);
        }

        let src_dir = root.join(&manifest.run.src);
        if !src_dir.is_dir() {
            return Err(format!(
                "axon: project at `{}` has no `{}` directory",
                root.display(),
                manifest.run.src
            ));
        }
        let main_modules = collect_modules(&src_dir, &src_dir, None, &mut sources)?;
        modules.extend(main_modules);
        // Stable order — dep modules first (alphabetical by combined path),
        // main project last.
        modules.sort_by(|a, b| a.module_path.cmp(&b.module_path));

        let (merged, mut diags) = merge_modules(&modules);
        diags.extend(check_privacy(&modules));

        Ok(LoadedProject {
            root: root.to_path_buf(),
            manifest,
            modules,
            merged,
            diagnostics: diags,
            sources,
        })
    }
}

// ===========================================================================
// Privacy
// ===========================================================================

/// Walk every module's `use` declarations and verify that each imported
/// item is declared `pub` in its source module. Within a module all items
/// are visible regardless of `pub`, so a `use` of a same-module name
/// (`use foo` where `foo` lives in the current file) is a no-op.
fn check_privacy(modules: &[ModuleFile]) -> Vec<Diagnostic> {
    // Build a map: module_path -> set of (item_name, is_pub).
    let mut exports: HashMap<String, HashMap<String, bool>> = HashMap::new();
    for m in modules {
        let entry = exports.entry(m.module_path.clone()).or_default();
        for item in &m.program.items {
            if let Some(name) = item_name(item) {
                let is_pub = item_is_pub(item);
                entry.insert(name, is_pub);
            }
        }
    }

    let mut diags = Vec::new();
    for m in modules {
        for item in &m.program.items {
            if let Item::Use(u) = item {
                if u.path.segments.is_empty() {
                    continue;
                }
                let dotted = u
                    .path
                    .segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                let mod_path = &dotted;
                let mod_exports = match exports.get(mod_path) {
                    Some(e) => e,
                    None => {
                        // Unknown module path — fall through silently for
                        // now since stdlib-shaped imports (`std.io.print`)
                        // don't have a real module backing them yet.
                        continue;
                    }
                };
                let names: Vec<String> = match (&u.items, &u.alias) {
                    (Some(items), _) => items.iter().map(|i| i.name.clone()).collect(),
                    (None, Some(alias)) => vec![alias.name.clone()],
                    (None, None) => continue,
                };
                for name in &names {
                    match mod_exports.get(name) {
                        Some(true) => {} // pub — OK
                        Some(false) => {
                            diags.push(
                                Diagnostic::error(
                                    format!(
                                        "item `{name}` in module `{mod_path}` is not `pub` — \
                                         declare it with `pub` to expose it outside the module"
                                    ),
                                    u.span,
                                )
                                .with_code("P0010"),
                            );
                        }
                        None => {
                            diags.push(
                                Diagnostic::error(
                                    format!("module `{mod_path}` has no item `{name}`"),
                                    u.span,
                                )
                                .with_code("P0011"),
                            );
                        }
                    }
                }
            }
        }
    }
    diags
}

fn item_is_pub(item: &Item) -> bool {
    match item {
        Item::Fn(f) => matches!(f.vis, axon_ast::Visibility::Public),
        Item::Type(t) => matches!(t.vis, axon_ast::Visibility::Public),
        Item::Schema(s) => matches!(s.vis, axon_ast::Visibility::Public),
        Item::Trait(t) => matches!(t.vis, axon_ast::Visibility::Public),
        Item::Const(c) => matches!(c.vis, axon_ast::Visibility::Public),
        Item::Effect(e) => matches!(e.vis, axon_ast::Visibility::Public),
        // The other item kinds aren't covered by `pub` in the spec — they
        // are always module-scope-visible. Treat as pub for cross-module
        // checking so a `use mod.Greeter` against an agent declaration
        // succeeds.
        _ => true,
    }
}

/// Walk `dir` recursively, collecting every `.ax` file as a [`ModuleFile`].
/// The module path is the file's location relative to `src_root`, dots in
/// place of slashes, with the `.ax` suffix stripped. If `prefix` is
/// supplied, every module name is prefixed (used for dep namespacing).
fn collect_modules(
    dir: &Path,
    src_root: &Path,
    prefix: Option<&str>,
    sources: &mut SourceRegistry,
) -> Result<Vec<ModuleFile>, String> {
    let mut out = Vec::new();
    walk_dir(dir, src_root, prefix, sources, &mut out)?;
    out.sort_by(|a, b| a.module_path.cmp(&b.module_path));
    Ok(out)
}

fn walk_dir(
    dir: &Path,
    src_root: &Path,
    prefix: Option<&str>,
    sources: &mut SourceRegistry,
    out: &mut Vec<ModuleFile>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| {
        format!("axon: cannot read directory `{}`: {e}", dir.display())
    })?;
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, src_root, prefix, sources, out)?;
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some(SOURCE_EXTENSION) {
            continue;
        }
        let module_tail = module_path_for(&path, src_root)?;
        let module_path = match prefix {
            Some(p) if !module_tail.is_empty() => format!("{p}.{module_tail}"),
            Some(p) => p.to_string(),
            None => module_tail,
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("axon: cannot read `{}`: {e}", path.display()))?;
        // Register the source with the registry so spans across modules
        // attribute back to the right file.
        let id = sources.register(path.to_string_lossy().into_owned(), text.clone());
        let source = SourceFile::with_id(id, path.to_string_lossy().into_owned(), text);
        let (program, _diags) = axon_parser::parse(&source);
        out.push(ModuleFile {
            module_path,
            file_path: path,
            source,
            program,
        });
    }
    Ok(())
}

fn module_path_for(file: &Path, src_root: &Path) -> Result<String, String> {
    let rel = file
        .strip_prefix(src_root)
        .map_err(|_| format!("file `{}` is outside src_root", file.display()))?;
    let mut segs = Vec::new();
    for c in rel.components() {
        match c {
            std::path::Component::Normal(s) => {
                let s = s.to_string_lossy();
                segs.push(s.trim_end_matches(".ax").to_string());
            }
            _ => {} // skip RootDir/CurDir/etc.
        }
    }
    Ok(segs.join("."))
}

// ===========================================================================
// Merging
// ===========================================================================

fn merge_modules(modules: &[ModuleFile]) -> (Program, Vec<Diagnostic>) {
    let mut items: Vec<Item> = Vec::new();
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut seen: std::collections::HashMap<String, (String, Span)> =
        std::collections::HashMap::new();

    // Reproduce each module's parse diagnostics so the user sees them.
    for m in modules {
        let (_, parse_diags) = axon_parser::parse(&m.source);
        diags.extend(parse_diags);
        for item in &m.program.items {
            if let Some(name) = item_name(item) {
                if let Some((prev_mod, prev_span)) = seen.get(&name) {
                    let mut d = Diagnostic::error(
                        format!(
                            "item `{name}` is declared in both module `{}` and module `{}`",
                            prev_mod, m.module_path
                        ),
                        item.span(),
                    )
                    .with_code("P0001");
                    d.secondary.push(axon_diag::Label {
                        span: *prev_span,
                        message: Some("first declaration here".into()),
                    });
                    diags.push(d);
                } else {
                    seen.insert(name, (m.module_path.clone(), item.span()));
                }
            }
            items.push(item.clone());
        }
    }
    let program = Program {
        items,
        span: Span::DUMMY,
    };
    (program, diags)
}

/// Extract the *name* used for collision detection. Anonymous items
/// (`use`, `impl`) return None.
fn item_name(item: &Item) -> Option<String> {
    match item {
        Item::Fn(f) => Some(f.name.name.clone()),
        Item::Type(t) => Some(t.name.name.clone()),
        Item::Schema(s) => Some(s.name.name.clone()),
        Item::Agent(a) => Some(a.name.name.clone()),
        Item::Actor(a) => Some(a.name.name.clone()),
        Item::Tool(t) => Some(t.name.name.clone()),
        Item::Model(m) => Some(m.name.name.clone()),
        Item::Memory(m) => Some(m.name.name.clone()),
        Item::Prompt(p) => Some(p.name.name.clone()),
        Item::Trait(t) => Some(t.name.name.clone()),
        Item::Const(c) => Some(c.name.name.clone()),
        Item::Effect(e) => Some(e.name.name.clone()),
        Item::Config(c) => Some(c.name.name.clone()),
        Item::Supervisor(s) => Some(s.name.name.clone()),
        Item::Graph(g) => Some(g.name.name.clone()),
        Item::Network(n) => Some(n.name.name.clone()),
        Item::Orchestrate(o) => Some(o.name.name.clone()),
        Item::Policy(p) => Some(p.name.name.clone()),
        Item::MemPolicy(p) => Some(p.name.name.clone()),
        // Tests and evals have string names; not subject to collision
        // because two `test "..."` blocks can share a name (they're just
        // labels). Same for `use` and `impl`.
        Item::Test(_) | Item::Eval(_) | Item::Use(_) | Item::Impl(_) => None,
    }
}
