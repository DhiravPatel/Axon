import { DocsPageShell } from "@/components/DocsPageShell";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { PreEnhancer } from "@/components/PreEnhancer";
import { buildSidebar } from "@/lib/sidebar";
import { loadExamples } from "@/lib/docs";

export const metadata = {
  title: "Examples",
  description: "Runnable Axon programs that exercise the spec end-to-end.",
};

export default async function ExamplesPage() {
  const [sidebar, examples] = await Promise.all([
    buildSidebar(),
    loadExamples(),
  ]);
  return (
    <DocsPageShell sidebar={sidebar} activeHref="/docs/examples">
      <Breadcrumbs
        items={[
          { href: "/", label: "Home" },
          { href: "/docs", label: "Docs" },
          { label: "Examples" },
        ]}
      />
      <header className="mb-10 max-w-3xl">
        <p className="font-mono text-xs uppercase tracking-[0.2em] text-copper-400">
          Browse · /examples
        </p>
        <h1 className="mt-2 font-display text-4xl font-semibold tracking-tight text-white">
          Examples
        </h1>
        <p className="mt-4 text-lg leading-relaxed text-ink-300">
          Every program below lives in the <code>examples/</code> directory of
          the workspace and is exercised by CI. Run any of them with{" "}
          <code>axon run examples/&lt;file&gt;.ax</code>.
        </p>
      </header>

      {examples.length === 0 ? (
        <p className="text-ink-400">
          No <code>.ax</code> files were found under{" "}
          <code>../examples/</code>.
        </p>
      ) : (
        <div id="examples-container" className="space-y-10">
          {examples.map((ex) => (
            <section key={ex.name} id={slugify(ex.name)}>
              <div className="mb-3 flex items-center justify-between">
                <h2 className="font-display text-lg font-semibold text-white">
                  {ex.name}
                </h2>
                <span className="font-mono text-[11px] uppercase tracking-wider text-ink-500">
                  {ex.body.split("\n").length} lines
                </span>
              </div>
              <div
                className="overflow-hidden rounded-xl border border-ink-800 bg-ink-900 shadow-2xl shadow-black/30"
                dangerouslySetInnerHTML={{ __html: ex.html }}
              />
            </section>
          ))}
          <PreEnhancer containerId="examples-container" />
        </div>
      )}
    </DocsPageShell>
  );
}

function slugify(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, "");
}
