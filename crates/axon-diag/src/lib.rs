//! Source files, byte spans, and Rust-style diagnostic rendering.
//!
//! The compiler is span-driven: every token and AST node carries a [`Span`] that
//! points back into a [`SourceFile`]. Diagnostics ([`Diagnostic`]) attach a primary
//! span plus any number of secondary spans and notes, and [`render`] turns them
//! into a colored, caret-pointed message ready for stderr.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

pub mod explain;

/// A loaded source file. Cheaply cloneable; the contents live behind `Arc`.
/// Each instance carries a stable `file_id` so spans can be attributed
/// back to the right file in a multi-file project — the [`SourceRegistry`]
/// holds the mapping from id to file.
#[derive(Clone)]
pub struct SourceFile {
    inner: Arc<SourceFileInner>,
}

struct SourceFileInner {
    id: u16,
    path: PathBuf,
    text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    /// Construct a file with file_id 0 — the right default for single-file
    /// callers. Project loaders that manage multiple files should use
    /// [`Self::with_id`] so spans across modules attribute correctly.
    pub fn new(path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        Self::with_id(0, path, text)
    }

    pub fn with_id(
        id: u16,
        path: impl Into<PathBuf>,
        text: impl Into<String>,
    ) -> Self {
        let text = text.into();
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            inner: Arc::new(SourceFileInner {
                id,
                path: path.into(),
                text,
                line_starts,
            }),
        }
    }

    pub fn id(&self) -> u16 {
        self.inner.id
    }

    pub fn path(&self) -> &std::path::Path {
        &self.inner.path
    }

    pub fn text(&self) -> &str {
        &self.inner.text
    }

    /// 1-based line/column for a byte offset. Column counts Unicode scalars.
    /// Out-of-range bytes saturate to the end of the file — callers may
    /// pass a span from a *different* source file (when rendering a
    /// multi-file project diagnostic against the wrong stand-in) and we'd
    /// rather degrade gracefully than panic on every cross-file error.
    pub fn line_col(&self, byte: usize) -> (usize, usize) {
        let len = self.inner.text.len();
        let byte = byte.min(len);
        let line_starts = &self.inner.line_starts;
        let line_idx = match line_starts.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = line_starts[line_idx].min(byte);
        let col = self.inner.text[line_start..byte].chars().count() + 1;
        (line_idx + 1, col)
    }

    pub fn line_text(&self, line_1based: usize) -> &str {
        let idx = line_1based - 1;
        let start = self.inner.line_starts[idx];
        let end = self
            .inner
            .line_starts
            .get(idx + 1)
            .copied()
            .unwrap_or(self.inner.text.len());
        let line = &self.inner.text[start..end];
        line.trim_end_matches(['\n', '\r'])
    }
}

impl fmt::Debug for SourceFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceFile")
            .field("path", &self.inner.path)
            .field("len", &self.inner.text.len())
            .finish()
    }
}

/// A half-open byte range `[start, end)` into a single source file. We do not
/// embed the file handle in `Span` so the type stays `Copy` and small; callers
/// pass the [`SourceFile`] alongside when they need it.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    /// Id of the source file this span lives in. `0` is the default — used
    /// by single-file callers and by spans that don't have a real source
    /// (synthesized literals from the runtime, for instance).
    pub file: u16,
}

impl Span {
    pub const DUMMY: Span = Span {
        start: 0,
        end: 0,
        file: 0,
    };

    pub fn new(start: usize, end: usize) -> Self {
        Self::in_file(start, end, 0)
    }

    pub fn in_file(start: usize, end: usize, file: u16) -> Self {
        debug_assert!(start <= end);
        Self {
            start: start as u32,
            end: end as u32,
            file,
        }
    }

    pub fn join(self, other: Span) -> Span {
        // The merged span belongs to *some* file; pick the first-non-zero
        // so the more useful attribution wins in the common case.
        let file = if self.file != 0 { self.file } else { other.file };
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            file,
        }
    }

    pub fn len(self) -> usize {
        (self.end - self.start) as usize
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.file == 0 {
            write!(f, "{}..{}", self.start, self.end)
        } else {
            write!(f, "{}..{}@{}", self.start, self.end, self.file)
        }
    }
}

// ---------------------------------------------------------------------------
// SourceRegistry
// ---------------------------------------------------------------------------

/// A list of [`SourceFile`]s, indexed by `file_id`. The first file gets id
/// 1 by convention so id 0 means "no file / dummy span". Use
/// [`Self::register`] to add new files; [`Self::get`] looks them up.
#[derive(Clone, Default)]
pub struct SourceRegistry {
    files: Vec<SourceFile>,
}

impl SourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a file's content and return its id. The id matches what
    /// you'd pass to [`SourceFile::with_id`] when materializing it.
    pub fn register(&mut self, path: impl Into<PathBuf>, text: impl Into<String>) -> u16 {
        let id = (self.files.len() + 1) as u16;
        let file = SourceFile::with_id(id, path, text);
        self.files.push(file);
        id
    }

    /// Register an existing file *as-is*. Useful when the parser already
    /// constructed a SourceFile with a known id.
    pub fn push(&mut self, file: SourceFile) {
        self.files.push(file);
    }

    pub fn get(&self, file: u16) -> Option<&SourceFile> {
        if file == 0 {
            return None;
        }
        self.files.iter().find(|f| f.id() == file)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SourceFile> {
        self.files.iter()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Severity {
    Error,
    Warning,
    Note,
    Help,
}


#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Option<&'static str>,
    pub message: String,
    pub primary: Label,
    pub secondary: Vec<Label>,
    pub notes: Vec<String>,
    /// Concrete, applicable rewrites the user could accept. Each fix is
    /// labeled (`"replace `foo` with `bar`"`) and carries one or more
    /// span-keyed text edits — `axon fix` applies them as a unified-diff
    /// dry-run or rewrites the file in place. Empty for diagnostics that
    /// can't be mechanically resolved.
    pub fixes: Vec<Fix>,
}

/// One mechanically applicable rewrite suggested by a diagnostic.
#[derive(Clone, Debug)]
pub struct Fix {
    /// Short human description: `"insert `Net` into the `uses` row"`,
    /// `"replace `foo` with `food`"`.
    pub description: String,
    /// Concrete edits — all applied atomically when the user accepts.
    pub edits: Vec<FixEdit>,
}

/// One span-keyed text replacement. `replacement.is_empty()` is a delete;
/// `span.is_empty()` is an insertion at that offset.
#[derive(Clone, Debug)]
pub struct FixEdit {
    pub span: Span,
    pub replacement: String,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            code: None,
            message: message.into(),
            primary: Label {
                span,
                message: None,
            },
            secondary: Vec::new(),
            notes: Vec::new(),
            fixes: Vec::new(),
        }
    }

    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    pub fn with_primary_label(mut self, label: impl Into<String>) -> Self {
        self.primary.message = Some(label.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Attach one mechanically applicable rewrite. Multiple fixes can be
    /// attached — `axon fix` lets the user pick (or `--only CODE`
    /// auto-applies the first when the code matches).
    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fixes.push(fix);
        self
    }
}

impl Fix {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            edits: Vec::new(),
        }
    }

    pub fn replace(span: Span, replacement: impl Into<String>) -> Self {
        let replacement = replacement.into();
        Self {
            description: format!("replace with `{replacement}`"),
            edits: vec![FixEdit { span, replacement }],
        }
    }

    pub fn insert(at: usize, file: u16, text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            description: format!("insert `{text}`"),
            edits: vec![FixEdit {
                span: Span::in_file(at, at, file),
                replacement: text,
            }],
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_edit(mut self, edit: FixEdit) -> Self {
        self.edits.push(edit);
        self
    }
}

/// Render a diagnostic against a [`SourceRegistry`], picking the right
/// source for the diagnostic's primary span. Falls back to the first
/// file in the registry when the span's `file` id is 0 or unknown.
pub fn render_with_registry(
    diag: &Diagnostic,
    registry: &SourceRegistry,
    use_color: bool,
) -> String {
    let file = registry
        .get(diag.primary.span.file)
        .or_else(|| registry.iter().next());
    match file {
        Some(f) => render(diag, f, use_color),
        None => format!("{}", diag.message),
    }
}

/// Render a diagnostic against its source file in a Rust-like format.
///
/// ```text
/// error[E0001]: unexpected character `§`
///   --> hello.ax:3:5
///    |
///  3 |     §let x = 1
///    |     ^ not a valid token start
///    = note: did you mean `let`?
/// ```
pub fn render(diag: &Diagnostic, file: &SourceFile, use_color: bool) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    let (red, yellow, blue, cyan, bold, reset) = if use_color {
        (
            "\x1b[31m", "\x1b[33m", "\x1b[34m", "\x1b[36m", "\x1b[1m", "\x1b[0m",
        )
    } else {
        ("", "", "", "", "", "")
    };
    let (sev_color, sev_text) = match diag.severity {
        Severity::Error => (red, "error"),
        Severity::Warning => (yellow, "warning"),
        Severity::Note => (blue, "note"),
        Severity::Help => (cyan, "help"),
    };

    write!(out, "{bold}{sev_color}{sev_text}").unwrap();
    if let Some(code) = diag.code {
        write!(out, "[{code}]").unwrap();
    }
    writeln!(out, "{reset}{bold}: {}{reset}", diag.message).unwrap();

    let (line, col) = file.line_col(diag.primary.span.start as usize);
    let path = file.path().display();
    let gutter = line.to_string().len().max(1);
    let pad = " ".repeat(gutter);
    writeln!(out, "{pad}{blue}-->{reset} {path}:{line}:{col}").unwrap();
    writeln!(out, "{pad} {blue}|{reset}").unwrap();
    let line_text = file.line_text(line);
    writeln!(out, "{blue}{line:>gw$} |{reset} {line_text}", gw = gutter).unwrap();

    let caret_pad_chars = col - 1;
    // Bounds-clamp the span when it points into a different source file
    // than the one we're rendering against (multi-file projects: we
    // currently fall back to the first module's source for diagnostics).
    let len = file.text().len();
    let s = (diag.primary.span.start as usize).min(len);
    let e = (diag.primary.span.end as usize).min(len);
    let s = s.min(e);
    let span_chars = file.text()[s..e].chars().count().max(1);
    let caret = "^".repeat(span_chars);
    write!(
        out,
        "{pad} {blue}|{reset} {0}{sev_color}{caret}{reset}",
        " ".repeat(caret_pad_chars)
    )
    .unwrap();
    if let Some(label) = &diag.primary.message {
        write!(out, " {sev_color}{label}{reset}").unwrap();
    }
    writeln!(out).unwrap();

    for note in &diag.notes {
        writeln!(out, "{pad} {blue}= {bold}note{reset}: {note}").unwrap();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_basic() {
        let src = SourceFile::new("t.ax", "ab\ncd\nef");
        assert_eq!(src.line_col(0), (1, 1));
        assert_eq!(src.line_col(2), (1, 3));
        assert_eq!(src.line_col(3), (2, 1));
        assert_eq!(src.line_col(7), (3, 2));
    }

    #[test]
    fn line_text_returns_line_without_newline() {
        let src = SourceFile::new("t.ax", "ab\ncd\nef");
        assert_eq!(src.line_text(1), "ab");
        assert_eq!(src.line_text(2), "cd");
        assert_eq!(src.line_text(3), "ef");
    }

    #[test]
    fn render_produces_caret() {
        let src = SourceFile::new("t.ax", "let x = 1\n");
        let diag = Diagnostic::error("oops", Span::new(4, 5))
            .with_primary_label("here")
            .with_note("for testing");
        let out = render(&diag, &src, false);
        assert!(out.contains("error: oops"));
        assert!(out.contains("--> t.ax:1:5"));
        assert!(out.contains("let x = 1"));
        assert!(out.contains("^ here"));
        assert!(out.contains("= note: for testing"));
    }
}
