import type { TocEntry } from "@/lib/sections";

interface Props {
  entries: TocEntry[];
}

export function TableOfContents({ entries }: Props) {
  if (entries.length === 0) return null;
  return (
    <aside className="sticky top-20 hidden h-[calc(100vh-5rem)] w-56 shrink-0 overflow-y-auto border-l border-ink-800 py-8 pl-6 xl:block">
      <p className="mb-3 font-mono text-[11px] font-semibold uppercase tracking-[0.18em] text-ink-500">
        On this page
      </p>
      <ul className="space-y-1.5 text-sm">
        {entries.map((e) => (
          <li key={e.id}>
            <a
              href={`#${e.id}`}
              className="block truncate text-ink-400 transition-colors hover:text-copper-300"
              style={{
                paddingLeft: (e.depth - 2) * 12 + "px",
              }}
            >
              {e.text}
            </a>
          </li>
        ))}
      </ul>
    </aside>
  );
}
