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
      <header className="mb-8 max-w-3xl">
        <p className="font-mono text-xs uppercase tracking-[0.2em] text-copper-400">
          {eyebrow} · {idx + 1} of {all.length}
        </p>
        <h1 className="mt-2 font-display text-3xl font-semibold tracking-tight text-white sm:text-4xl">
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
