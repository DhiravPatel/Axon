import { Breadcrumbs, type Crumb } from "./Breadcrumbs";
import { PrevNext } from "./PrevNext";
import { PreEnhancer } from "./PreEnhancer";
import type { Chapter, ChapterMeta } from "@/lib/sections";

interface Props {
  chapter: Chapter;
  /** All chapters in the source so we can compute prev/next + index. */
  all: ChapterMeta[];
  baseHref: string;
  crumbs: Crumb[];
  eyebrow: string;
}

export function ChapterView({
  chapter,
  all,
  baseHref,
  crumbs,
  eyebrow,
}: Props) {
  const idx = all.findIndex((c) => c.slug === chapter.slug);
  const prev = idx > 0 ? all[idx - 1] : null;
  const next = idx >= 0 && idx < all.length - 1 ? all[idx + 1] : null;
  const containerId = `chapter-${baseHref.replace(/[^a-z0-9]/gi, "-")}-${chapter.slug}`;

  return (
    <>
      <Breadcrumbs items={crumbs} />
      <header className="mb-9 max-w-3xl">
        <div className="inline-flex items-center gap-2 rounded-full border border-copper-700/40 bg-copper-950/40 px-3 py-1 font-mono text-[11px] uppercase tracking-[0.14em] text-copper-300">
          <span>{eyebrow}</span>
          <span className="text-copper-500/60">·</span>
          <span className="text-copper-200/80">
            {idx + 1} / {all.length}
          </span>
        </div>
        <h1 className="mt-4 font-display text-3xl font-semibold tracking-tight text-white sm:text-[2.5rem] sm:leading-tight">
          {chapter.title}
        </h1>
        {chapter.blurb ? (
          <p className="mt-4 text-lg leading-relaxed text-ink-300">
            {chapter.blurb}
          </p>
        ) : null}
      </header>

      {/* `dangerouslySetInnerHTML` is fed by remark + Shiki running
       *  server-side at build time — no untrusted input flows in. */}
      <div
        id={containerId}
        className="docs-prose"
        dangerouslySetInnerHTML={{ __html: chapter.html }}
      />

      <PreEnhancer containerId={containerId} />

      <PrevNext
        prev={
          prev
            ? { href: `${baseHref}/${prev.slug}`, title: prev.title }
            : undefined
        }
        next={
          next
            ? { href: `${baseHref}/${next.slug}`, title: next.title }
            : undefined
        }
      />
    </>
  );
}
