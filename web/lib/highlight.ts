import { createHighlighter, type Highlighter } from "shiki";

/**
 * Server-side Shiki highlighter — initialized lazily so SSG builds
 * pay the language load cost exactly once.
 *
 * We register Axon as a clone of Rust's TextMate grammar with the
 * keyword list extended to cover Axon's agent/effect surface. This
 * is a deliberate shortcut; a hand-rolled grammar can drop in later
 * without changing call sites.
 */
let _highlighter: Promise<Highlighter> | null = null;

function highlighter(): Promise<Highlighter> {
  if (!_highlighter) {
    _highlighter = createHighlighter({
      themes: ["github-dark", "github-light"],
      // Keep the bundle lean — Axon and EBNF fall through to the rust
      // fallback in `highlightAs`.
      langs: ["rust", "typescript", "bash", "json", "toml", "yaml"],
    });
  }
  return _highlighter;
}

const AXON_KEYWORDS = new Set([
  "agent", "actor", "tool", "model", "memory", "prompt", "schema",
  "type", "trait", "impl", "const", "effect", "test", "eval",
  "fn", "let", "var", "if", "else", "match", "while", "for", "in",
  "return", "uses", "pub", "use", "as", "spawn", "select", "await",
  "ask", "generate", "plan", "with", "on", "true", "false", "nil",
  "and", "or", "not", "dyn", "policy", "mempolicy", "config",
  "supervisor", "graph", "network", "orchestrate", "break", "continue",
]);

const AXON_TYPE_NAMES = new Set([
  "Int", "Float", "Bool", "String", "Bytes", "Char", "Unit",
  "Date", "DateTime", "Time", "Money", "Duration", "List", "Map",
  "Set", "Option", "Result", "Tainted", "Stream", "Chan", "Tool",
  "Secret", "Model", "Memory", "AgentAddr",
]);

/**
 * Highlight an Axon snippet. Returns HTML ready to drop into the
 * page; the styling lives in `.shiki` rules in globals.css.
 *
 * For the v0 site we run Shiki over the rust grammar and then
 * patch identifier classes so Axon keywords / type names get the
 * expected coloring even though they aren't in the upstream Rust
 * grammar (e.g. `agent`, `policy`, `uses`).
 */
export async function highlightAxon(code: string): Promise<string> {
  const h = await highlighter();
  const base = h.codeToHtml(code, { lang: "rust", theme: "github-dark" });
  return repaintAxonTokens(base);
}

export async function highlightAs(code: string, lang: string): Promise<string> {
  const h = await highlighter();
  // Normalize shell aliases to `bash` — Shiki loads bash and treats
  // sh/shell as separate grammars that we don't bundle.
  const normalized = lang === "shell" || lang === "sh" ? "bash" : lang;
  const safeLang = SAFE_LANGS.has(normalized) ? normalized : "rust";
  const html = h.codeToHtml(code, { lang: safeLang, theme: "github-dark" });
  return safeLang === "rust" ? repaintAxonTokens(html) : html;
}

// Only languages loaded in `createHighlighter` above are "safe" — any
// other language (axon, ebnf, ...) falls through to the rust grammar
// + Axon-keyword repaint pass.
const SAFE_LANGS = new Set([
  "rust", "typescript", "bash", "shell", "sh", "json", "toml", "yaml",
]);

/**
 * Walk Shiki's rust-flavored HTML and recolor any `<span>` whose text
 * is an Axon keyword/type. Shiki's HTML wraps each token in a
 * `<span style="color:#XXX">word</span>`, so a simple regex replace
 * is enough; we don't need a real HTML parser here.
 */
function repaintAxonTokens(html: string): string {
  return html.replace(
    /<span style="color:#[0-9A-Fa-f]+">([^<]+)<\/span>/g,
    (match, text: string) => {
      const trimmed = text.trim();
      if (AXON_KEYWORDS.has(trimmed)) {
        return `<span style="color:#FF7B72">${text}</span>`; // GitHub Dark keyword red
      }
      if (AXON_TYPE_NAMES.has(trimmed)) {
        return `<span style="color:#FFA657">${text}</span>`; // GitHub Dark type orange
      }
      return match;
    }
  );
}
