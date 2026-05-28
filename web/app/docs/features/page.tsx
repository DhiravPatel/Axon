import Link from "next/link";
import { DocsPageShell } from "@/components/DocsPageShell";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { buildSidebar } from "@/lib/sidebar";
import { loadChapters } from "@/lib/sections";

export const metadata = {
  title: "Implemented features",
  description:
    "Browse what ships today, stage by stage, with crate links and test counts.",
};

export default async function FeaturesIndexPage() {
  const [sidebar, chapters] = await Promise.all([
    buildSidebar(),
    loadChapters("FEATURES.md"),
  ]);
  return (
    <DocsPageShell sidebar={sidebar} activeHref="/docs/features">
      <Breadcrumbs
        items={[
          { href: "/", label: "Home" },
          { href: "/docs", label: "Docs" },
          { label: "Features" },
        ]}
      />
      <header className="mb-10 max-w-3xl">
        <p className="font-mono text-xs uppercase tracking-[0.2em] text-copper-400">
          Reference · FEATURES.md
        </p>
        <h1 className="mt-2 font-display text-4xl font-semibold tracking-tight text-white">
          Implemented features
        </h1>
        <p className="mt-4 text-lg leading-relaxed text-ink-300">
          Every entry below maps a build stage to its shipped surface — source
          crates, host bindings, test counts, and a CLI transcript. Pulled
          live from <code>FEATURES.md</code> so the docs never drift from the
          implementation.
        </p>
      </header>

      <ul className="grid gap-3 md:grid-cols-2">
        {chapters.map((c) => (
          <li key={c.slug}>
            <Link
              href={`/docs/features/${c.slug}`}
              className="group flex h-full flex-col rounded-xl border border-ink-800 bg-ink-900/30 p-5 transition-colors hover:border-copper-700/60 hover:bg-ink-900/60"
            >
              <div className="mb-2 flex items-center gap-2">
                <span className="rounded bg-copper-950/40 px-2 py-0.5 font-mono text-[10px] uppercase tracking-wider text-copper-300">
                  {c.title.match(/^Stage\s+\d+(?:\.\d+)?/i)?.[0] ?? "entry"}
                </span>
              </div>
              <h3 className="font-display text-lg font-semibold text-white group-hover:text-copper-200">
                {c.title.replace(/^Stage\s+\d+(?:\.\d+)?\s*[—-]\s*/i, "")}
              </h3>
              {c.blurb ? (
                <p className="mt-2 line-clamp-3 text-sm leading-6 text-ink-300">
                  {c.blurb}
                </p>
              ) : null}
              <span className="mt-4 text-sm text-copper-400 group-hover:underline">
                Open →
              </span>
            </Link>
          </li>
        ))}
      </ul>
    </DocsPageShell>
  );
}
