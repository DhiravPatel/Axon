"use client";

import { useEffect, useState } from "react";
import type { TocEntry } from "@/lib/sections";

interface Props {
  entries: TocEntry[];
}

export function TableOfContents({ entries }: Props) {
  const [activeId, setActiveId] = useState<string>("");

  useEffect(() => {
    if (entries.length === 0) return;
    const ids = entries.map((e) => e.id);

    let frame = 0;
    const recompute = () => {
      frame = 0;
      // Active heading = the last one whose top has scrolled past the
      // reading line (~140px below the sticky header).
      const line = 140;
      let current = ids[0] ?? "";
      for (const id of ids) {
        const el = document.getElementById(id);
        if (!el) continue;
        if (el.getBoundingClientRect().top <= line) {
          current = id;
        } else {
          break;
        }
      }
      // Pin the last entry once the page is scrolled to the very bottom.
      if (
        window.innerHeight + window.scrollY >=
        document.documentElement.scrollHeight - 4
      ) {
        current = ids[ids.length - 1] ?? current;
      }
      setActiveId(current);
    };

    const onScroll = () => {
      if (frame) return;
      frame = window.requestAnimationFrame(recompute);
    };

    recompute();
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll, { passive: true });
    return () => {
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
      if (frame) window.cancelAnimationFrame(frame);
    };
  }, [entries]);

  if (entries.length === 0) return null;

  return (
    <aside className="scroll-thin sticky top-16 hidden h-[calc(100vh-4rem)] w-60 shrink-0 overflow-y-auto py-8 pl-8 xl:block">
      <p className="mb-3 px-2 font-mono text-[10.5px] font-semibold uppercase tracking-[0.18em] text-ink-500">
        On this page
      </p>
      <div className="relative">
        {/* Track line down the left edge of the TOC list. */}
        <span
          aria-hidden
          className="absolute left-0 top-0 h-full w-px bg-ink-800"
        />
        <ul className="space-y-0.5 text-[13px]">
          {entries.map((e) => {
            const active = e.id === activeId;
            return (
              <li key={e.id} className="relative">
                <span
                  aria-hidden
                  className={
                    "absolute -left-px top-0 h-full w-px transition-colors " +
                    (active ? "bg-copper-400" : "bg-transparent")
                  }
                />
                <a
                  href={`#${e.id}`}
                  data-active={active}
                  className="toc-link truncate"
                  style={{ paddingLeft: (e.depth - 2) * 12 + 12 + "px" }}
                >
                  {e.text}
                </a>
              </li>
            );
          })}
        </ul>
      </div>
      <a
        href="#top"
        onClick={(ev) => {
          ev.preventDefault();
          window.scrollTo({ top: 0, behavior: "smooth" });
        }}
        className="mt-6 inline-flex items-center gap-1.5 px-2 text-[12px] text-ink-500 transition-colors hover:text-copper-300"
      >
        <span aria-hidden>↑</span> Back to top
      </a>
    </aside>
  );
}
