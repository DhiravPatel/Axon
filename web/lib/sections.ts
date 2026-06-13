import fs from "node:fs/promises";
import path from "node:path";
import { renderMarkdown } from "./docs";

const REPO_ROOT = path.resolve(process.cwd(), "..");

export interface ChapterMeta {
  /** URL-safe identifier — derived from the heading. */
  slug: string;
  /** Original heading text (after `## `). */
  title: string;
  /** First non-blank paragraph from the chapter body — used in TOCs. */
  blurb: string;
  /** 1-based index in the source document (stable across rebuilds). */
  index: number;
  /** Raw markdown body of this chapter (heading included). */
  body: string;
}

export interface Chapter extends ChapterMeta {
  html: string;
  /** Subheadings (H2-level relative to the chapter, i.e. `### `) for the
   *  right-rail table of contents. */
  toc: TocEntry[];
}

export interface TocEntry {
  id: string;
  text: string;
  depth: 2 | 3;
}

export type SourceFile = "PLAN.md" | "FEATURES.md";

const SLUG_OVERRIDES: Record<string, Record<string, string>> = {};

/**
 * Split a `##`-delimited markdown document into chapters. We
 * deliberately ignore `#` (document title) and lower-level headings —
 * each `## Foo` becomes one routable page.
 */
export async function loadChapters(file: SourceFile): Promise<ChapterMeta[]> {
  const abs = path.join(REPO_ROOT, file);
  const raw = await fs.readFile(abs, "utf8");
  return splitChapters(raw, file);
}

export async function loadChapter(
  file: SourceFile,
  slug: string
): Promise<Chapter | null> {
  const chapters = await loadChapters(file);
  const meta = chapters.find((c) => c.slug === slug);
  if (!meta) return null;
  const html = await renderMarkdown(meta.body);
  const toc = extractToc(html);
  return { ...meta, html, toc };
}

export function splitChapters(raw: string, file: SourceFile): ChapterMeta[] {
  const lines = raw.split(/\r?\n/);
  const out: ChapterMeta[] = [];
  let current: { title: string; body: string[] } | null = null;
  let inFence = false;
  let fenceDelim = "";

  const pushCurrent = () => {
    if (!current) return;
    const body = current.body.join("\n").trim();
    const slug = slugify(current.title, file);
    const blurb = firstParagraph(body);
    out.push({
      slug,
      title: current.title,
      blurb,
      index: out.length + 1,
      body: `## ${current.title}\n\n${body}`,
    });
  };

  for (const line of lines) {
    // Track fenced code so a `## foo` *inside* a code block doesn't
    // start a new chapter.
    const trimmed = line.trimStart();
    if (!inFence && (trimmed.startsWith("```") || trimmed.startsWith("~~~"))) {
      inFence = true;
      fenceDelim = trimmed.slice(0, 3);
    } else if (inFence && trimmed.startsWith(fenceDelim)) {
      inFence = false;
      fenceDelim = "";
    }

    if (!inFence && /^##\s+/.test(line) && !/^###/.test(line)) {
      pushCurrent();
      current = { title: line.replace(/^##\s+/, "").trim(), body: [] };
      continue;
    }
    if (current) {
      current.body.push(line);
    }
  }
  pushCurrent();
  return out;
}

/**
 * Slugify a chapter heading. Honors a small override table per file
 * so PLAN.md sections keep their canonical numeric prefix (`§22` →
 * `22-agents`).
 */
function slugify(title: string, file: SourceFile): string {
  const overrides = SLUG_OVERRIDES[file];
  if (overrides && overrides[title]) return overrides[title];
  // Pull a leading section number if the heading starts with it: e.g.
  // `22. Agents — the core abstraction` → `22-agents-the-core-abstraction`.
  const numericMatch = title.match(/^(\d+(?:\.\d+)?)\.?\s+(.*)$/);
  if (numericMatch) {
    const num = numericMatch[1].replace(".", "-");
    return `${num}-${kebab(numericMatch[2])}`;
  }
  // Stage entries in FEATURES.md look like `Stage 27 — @approval (§25.6)`.
  const stageMatch = title.match(/^Stage\s+(\d+(?:\.\d+)?)\s*[—-]\s*(.*)$/i);
  if (stageMatch) {
    return `stage-${stageMatch[1].replace(".", "-")}`;
  }
  return kebab(title);
}

function kebab(s: string): string {
  return s
    .toLowerCase()
    .replace(/[`'"]/g, "")
    // Replace any non-alphanumeric with a single dash.
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-+|-+$)/g, "")
    // Collapse multiple dashes.
    .replace(/-{2,}/g, "-")
    // Cap length so URLs don't get absurd.
    .slice(0, 80);
}

function firstParagraph(body: string): string {
  const stripped = body.replace(/^##\s+.+$/m, "").trim();
  // First non-empty, non-fenced block.
  const paras = stripped.split(/\n\s*\n/);
  for (const p of paras) {
    const t = p.trim();
    if (!t) continue;
    if (t.startsWith("```")) continue;
    if (t.startsWith("|")) continue; // skip tables
    if (t.startsWith("**")) continue; // skip bold-only opener lines
    // Strip markdown formatting for a clean blurb.
    return t
      .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
      .replace(/[*_`]/g, "")
      .replace(/\s+/g, " ")
      .slice(0, 220);
  }
  return "";
}

/**
 * Parse the rendered HTML and return every `<h3>` heading as a TOC
 * entry. (We skip H1/H2 — the chapter heading is already the page
 * title.) Anchor IDs come from `rehype-slug`.
 */
function extractToc(html: string): TocEntry[] {
  const out: TocEntry[] = [];
  const re = /<h(3)[^>]*id="([^"]+)"[^>]*>([\s\S]*?)<\/h\1>/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(html)) !== null) {
    const text = decodeEntities(stripTags(m[3])).trim();
    if (!text) continue;
    out.push({ id: m[2], text, depth: 3 });
  }
  return out;
}

function stripTags(html: string): string {
  return html.replace(/<[^>]+>/g, "");
}

/**
 * Decode the small set of HTML entities the markdown renderer emits in
 * heading text (`&`, `<`, `>`, quotes) plus any numeric escape, so TOC
 * labels read as plain text instead of `&#x26;` / `&#x3C;`.
 */
function decodeEntities(s: string): string {
  return s
    .replace(/&#x([0-9a-f]+);/gi, (_, hex) =>
      String.fromCodePoint(parseInt(hex, 16))
    )
    .replace(/&#(\d+);/g, (_, dec) => String.fromCodePoint(parseInt(dec, 10)))
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&nbsp;/g, " ");
}
