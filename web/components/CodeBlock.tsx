import { highlightAs } from "@/lib/highlight";

interface Props {
  code: string;
  lang?: string;
  filename?: string;
}

export async function CodeBlock({ code, lang = "axon", filename }: Props) {
  const html = await highlightAs(code.trimEnd(), lang);
  return (
    <div className="group/code overflow-hidden rounded-xl border border-ink-700/70 bg-ink-900 shadow-2xl shadow-black/40 ring-1 ring-inset ring-white/[0.03]">
      {filename ? (
        <div className="flex items-center justify-between border-b border-ink-800 bg-ink-900/80 px-4 py-2.5 text-xs">
          <div className="flex items-center gap-3">
            <span className="flex items-center gap-1.5" aria-hidden>
              <span className="h-2.5 w-2.5 rounded-full bg-[#ff5f57]/80" />
              <span className="h-2.5 w-2.5 rounded-full bg-[#febc2e]/80" />
              <span className="h-2.5 w-2.5 rounded-full bg-[#28c840]/80" />
            </span>
            <span className="font-mono text-ink-400">{filename}</span>
          </div>
          <span className="rounded bg-ink-800/70 px-2 py-0.5 font-mono text-[10px] uppercase tracking-wider text-ink-400">
            {lang}
          </span>
        </div>
      ) : null}
      {/* `html` is the trusted output of Shiki — no untrusted input flows in. */}
      <div
        className="[&_pre]:!my-0 [&_pre]:!rounded-none [&_pre]:!border-0 [&_pre]:!bg-transparent"
        dangerouslySetInnerHTML={{ __html: html }}
      />
    </div>
  );
}
