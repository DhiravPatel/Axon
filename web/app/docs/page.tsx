import Link from "next/link";
import { CodeBlock } from "@/components/CodeBlock";
import { DocsPageShell } from "@/components/DocsPageShell";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { buildSidebar } from "@/lib/sidebar";

const QUICK_START = `// hello.ax — your first agent program
fn main() uses { Console } {
    print("Hello from Axon!")
}`;

const READING_PATHS = [
  {
    href: "/docs/spec",
    title: "Read the spec",
    body: "PLAN.md split into per-chapter pages — types, agents, effects, runtime, deployment.",
    badge: "spec",
  },
  {
    href: "/docs/features",
    title: "Browse features by stage",
    body: "FEATURES.md as a chapter-per-stage tour — every entry links to source crates and tests.",
    badge: "live",
  },
  {
    href: "/docs/examples",
    title: "See runnable examples",
    body: "Real `.ax` programs that exercise the spec end-to-end — copy + run with `axon run`.",
    badge: "code",
  },
];

const PIPELINE = [
  { step: "1", title: "Lex", body: "Unicode-aware tokens; doc comments, prompt strings, money/duration/date literals all first-class." },
  { step: "2", title: "Parse", body: "Recursive-descent + Pratt expressions; every node carries a file-stamped span." },
  { step: "3", title: "Type-check", body: "Bidirectional types, effect rows, trait coherence, Tainted<T> propagation." },
  { step: "4", title: "Run", body: "Tree-walking interpreter or AxVM bytecode; both share the type table." },
];

export default async function DocsIndexPage() {
  const sidebar = await buildSidebar();
  return (
    <DocsPageShell sidebar={sidebar} activeHref="/docs">
      <Breadcrumbs items={[{ href: "/", label: "Home" }, { label: "Docs" }]} />

      <header className="mb-10 max-w-3xl">
        <p className="font-mono text-xs uppercase tracking-[0.2em] text-copper-400">
          Documentation
        </p>
        <h1 className="mt-2 font-display text-4xl font-semibold tracking-tight text-white">
          Welcome to Axon.
        </h1>
        <p className="mt-4 text-lg leading-relaxed text-ink-300">
          A typed, capability-safe, replayable language for production AI
          agents. Read the spec one chapter at a time, browse implemented
          features by stage, or open a runnable example.
        </p>
      </header>

      <div className="docs-prose mb-8">
        <h2>Install</h2>
        <p>
          One-line installer for macOS, Linux, and WSL. Windows is supported
          via the same script under PowerShell.
        </p>
      </div>
      <div className="mb-12 max-w-3xl">
        <CodeBlock
          code={"curl -sSf axon-lang.org/install.sh | sh"}
          lang="bash"
          filename="install"
        />
      </div>

      <div className="docs-prose mb-6">
        <h2>Your first program</h2>
        <p>
          Save the snippet below as <code>hello.ax</code>, then run{" "}
          <code>axon run hello.ax</code>. The <code>Console</code> effect on
          the function signature is the only capability the program asks for —
          anything else would be a compile-time error.
        </p>
      </div>
      <div className="mb-14 max-w-3xl">
        <CodeBlock code={QUICK_START} filename="hello.ax" lang="axon" />
      </div>

      <div className="docs-prose mb-6">
        <h2>Where to read next</h2>
        <p>
          The docs are organized into focused chapters. Pick a reading path
          that matches what you need today.
        </p>
      </div>
      <div className="mb-14 grid gap-4 md:grid-cols-3">
        {READING_PATHS.map((c) => (
          <Link
            key={c.href}
            href={c.href}
            className="group flex flex-col rounded-xl border border-ink-800 bg-ink-900/40 p-5 transition-colors hover:border-copper-700 hover:bg-ink-900"
          >
            <span className="mb-2 self-start rounded bg-ink-800 px-2 py-0.5 font-mono text-[10px] uppercase tracking-wider text-ink-400">
              {c.badge}
            </span>
            <h3 className="font-display text-lg font-semibold text-white">
              {c.title}
            </h3>
            <p className="mt-2 flex-1 text-sm text-ink-300">{c.body}</p>
            <span className="mt-3 text-sm text-copper-400 group-hover:underline">
              Read →
            </span>
          </Link>
        ))}
      </div>

      <div className="docs-prose mb-6">
        <h2>The pipeline at a glance</h2>
        <p>
          An Axon program is parsed, type-checked, and executed by the same
          compiler that produces WASM artifacts. Every stage is observable via
          the CLI — <code>axon tokens</code>, <code>axon parse</code>,{" "}
          <code>axon check</code>, <code>axon run --trace</code>.
        </p>
      </div>
      <div className="mb-12 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {PIPELINE.map((p) => (
          <div
            key={p.step}
            className="rounded-xl border border-ink-800 bg-ink-900/40 p-4"
          >
            <div className="flex items-center gap-3">
              <span className="inline-flex h-7 w-7 items-center justify-center rounded-md bg-copper-500/15 font-mono text-xs font-bold text-copper-300">
                {p.step}
              </span>
              <h4 className="font-display font-semibold text-white">{p.title}</h4>
            </div>
            <p className="mt-2 text-sm text-ink-300">{p.body}</p>
          </div>
        ))}
      </div>
    </DocsPageShell>
  );
}
