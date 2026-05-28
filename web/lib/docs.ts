import fs from "node:fs/promises";
import path from "node:path";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import remarkRehype from "remark-rehype";
import rehypeRaw from "rehype-raw";
import rehypeSlug from "rehype-slug";
import rehypeAutolinkHeadings from "rehype-autolink-headings";
import rehypeStringify from "rehype-stringify";
import { highlightAs } from "./highlight";

/**
 * Where the workspace root lives, relative to the Next.js working
 * directory (`<repo>/web`). Resolved at module load.
 */
const REPO_ROOT = path.resolve(process.cwd(), "..");

export const DOC_PAGES = {
  overview: {
    slug: "",
    title: "Overview",
    description: "What Axon is, why it exists, and how to read these docs.",
    kind: "static" as const,
  },
  spec: {
    slug: "spec",
    title: "Language spec",
    description:
      "PLAN.md — the normative reference for syntax, types, agents, and the runtime.",
    kind: "markdown" as const,
    file: "PLAN.md",
  },
  features: {
    slug: "features",
    title: "Implemented features",
    description:
      "FEATURES.md — a stage-by-stage account of what ships today, with crate links + test counts.",
    kind: "markdown" as const,
    file: "FEATURES.md",
  },
  examples: {
    slug: "examples",
    title: "Examples",
    description: "Runnable Axon programs that exercise each major feature.",
    kind: "examples" as const,
  },
};

export type DocSlug = keyof typeof DOC_PAGES;

export function listDocSlugs(): DocSlug[] {
  return Object.keys(DOC_PAGES) as DocSlug[];
}

/**
 * Render a Markdown file from disk into HTML, with GFM tables,
 * heading anchors, and Shiki syntax highlighting on fenced blocks.
 */
export async function renderMarkdownFile(rel: string): Promise<string> {
  const abs = path.join(REPO_ROOT, rel);
  const raw = await fs.readFile(abs, "utf8");
  return renderMarkdown(raw);
}

export async function renderMarkdown(raw: string): Promise<string> {
  // First pass — produce HTML without code highlighting; remark-rehype
  // emits `<pre><code class="language-xxx">...</code></pre>` for fenced
  // blocks. Then walk the string and replace each block with Shiki's
  // highlighted version. Doing the highlight here (rather than as a
  // rehype plugin) keeps the dependency tree small.
  const file = await unified()
    .use(remarkParse)
    .use(remarkGfm)
    .use(remarkRehype, { allowDangerousHtml: true })
    .use(rehypeRaw)
    .use(rehypeSlug)
    .use(rehypeAutolinkHeadings, {
      behavior: "wrap",
      properties: { className: ["heading-anchor"] },
    })
    .use(rehypeStringify, { allowDangerousHtml: true })
    .process(raw);
  let html = String(file);
  html = await highlightFencedBlocks(html);
  return html;
}

/**
 * Replace `<pre><code class="language-foo">...</code></pre>` blocks
 * with Shiki-highlighted equivalents.
 */
async function highlightFencedBlocks(html: string): Promise<string> {
  const blockRe =
    /<pre><code class="language-([a-zA-Z0-9_-]+)">([\s\S]*?)<\/code><\/pre>/g;
  const matches: { full: string; lang: string; code: string }[] = [];
  let m: RegExpExecArray | null;
  while ((m = blockRe.exec(html)) !== null) {
    matches.push({ full: m[0], lang: m[1], code: decodeHtml(m[2]) });
  }
  if (matches.length === 0) return html;
  const rendered = await Promise.all(
    matches.map(({ lang, code }) => highlightAs(code, lang))
  );
  let out = html;
  for (let i = 0; i < matches.length; i++) {
    out = out.replace(matches[i].full, rendered[i]);
  }
  return out;
}

function decodeHtml(s: string): string {
  return s
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, "&");
}

export interface ExampleEntry {
  name: string;
  body: string;
  html: string;
}

/** Read every `.ax` file under `<repo>/examples/` and render it as a code block. */
export async function loadExamples(): Promise<ExampleEntry[]> {
  const dir = path.join(REPO_ROOT, "examples");
  const out: ExampleEntry[] = [];
  await walkExamples(dir, dir, out);
  out.sort((a, b) => a.name.localeCompare(b.name));
  return out;
}

async function walkExamples(
  root: string,
  cur: string,
  out: ExampleEntry[]
): Promise<void> {
  let entries;
  try {
    entries = await fs.readdir(cur, { withFileTypes: true });
  } catch {
    return;
  }
  for (const e of entries) {
    const p = path.join(cur, e.name);
    if (e.isDirectory()) {
      await walkExamples(root, p, out);
    } else if (e.isFile() && e.name.endsWith(".ax")) {
      const rel = path.relative(root, p);
      const body = await fs.readFile(p, "utf8");
      const html = await highlightAs(body, "axon");
      out.push({ name: rel, body, html });
    }
  }
}
