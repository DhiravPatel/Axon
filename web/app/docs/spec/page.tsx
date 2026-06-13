import Link from "next/link";
import { DocsPageShell } from "@/components/DocsPageShell";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { buildSidebar } from "@/lib/sidebar";
import { loadChapters } from "@/lib/sections";

export const metadata = {
  title: "Language spec",
  description: "Per-chapter navigation across the Axon language reference.",
};

export default async function SpecIndexPage() {
  const [sidebar, chapters] = await Promise.all([
    buildSidebar(),
    loadChapters("PLAN.md"),
  ]);
  return (
    <DocsPageShell sidebar={sidebar} activeHref="/docs/spec">
      <Breadcrumbs
        items={[
          { href: "/", label: "Home" },
          { href: "/docs", label: "Docs" },
          { label: "Spec" },
        ]}
      />
      <header className="mb-10 max-w-3xl">
        <p className="font-mono text-xs uppercase tracking-[0.2em] text-copper-400">
          Reference · PLAN.md
        </p>
        <h1 className="mt-2 font-display text-4xl font-semibold tracking-tight text-white">
          Language spec
        </h1>
        <p className="mt-4 text-lg leading-relaxed text-ink-300">
          The normative reference for Axon, split into focused chapters. Each
          page renders one <code>##</code> section of <code>PLAN.md</code> with
          syntax-highlighted code, anchored sub-headings, and a per-page table
          of contents.
        </p>
      </header>

      <ChapterGrid baseHref="/docs/spec" chapters={chapters} />
    </DocsPageShell>
  );
}

function ChapterGrid({
  baseHref,
  chapters,
}: {
  baseHref: string;
  chapters: { slug: string; title: string; blurb: string; index: number }[];
}) {
  return (
    <ul className="grid gap-3 md:grid-cols-2">
      {chapters.map((c) => (
        <li key={c.slug}>
          <Link
            href={`${baseHref}/${c.slug}`}
            className="card-glow group flex h-full flex-col rounded-2xl border border-ink-800 bg-ink-900/30 p-5"
          >
            <div className="mb-2 flex items-center gap-2">
              <span className="rounded bg-ink-800 px-2 py-0.5 font-mono text-[10px] uppercase tracking-wider text-ink-400 ring-1 ring-inset ring-ink-700/50">
                #{c.index}
              </span>
            </div>
            <h3 className="font-display text-lg font-semibold text-white group-hover:text-copper-200">
              {c.title}
            </h3>
            {c.blurb ? (
              <p className="mt-2 line-clamp-3 text-sm leading-6 text-ink-300">
                {c.blurb}
              </p>
            ) : null}
            <span className="mt-4 inline-flex items-center gap-1 text-sm font-medium text-copper-400">
              Open
              <span className="transition-transform group-hover:translate-x-1">
                →
              </span>
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}
