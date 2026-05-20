import { highlightAs } from "@/lib/highlight";

interface Props {
  code: string;
  lang?: string;
  filename?: string;
}

export async function CodeBlock({ code, lang = "axon", filename }: Props) {
  const html = await highlightAs(code.trimEnd(), lang);
  return (
    <div className="overflow-hidden rounded-xl border border-ink-700 bg-ink-900 shadow-2xl shadow-black/30">
      {filename ? (
        <div className="flex items-center justify-between border-b border-ink-800 bg-ink-900/80 px-4 py-2 text-xs">
          <div className="flex items-center gap-2">
            <span className="h-2 w-2 rounded-full bg-copper-500" />
            <span className="font-mono text-ink-400">{filename}</span>
          </div>
          <span className="font-mono uppercase tracking-wider text-ink-500">
            {lang}
          </span>
        </div>
      ) : null}
      {/* `html` is the trusted output of Shiki — no untrusted input flows in. */}
      <div
        className="[&_pre]:!my-0 [&_pre]:!bg-transparent"
        dangerouslySetInnerHTML={{ __html: html }}
      />
    </div>
  );
}
