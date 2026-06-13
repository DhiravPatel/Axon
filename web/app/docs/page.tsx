import Link from "next/link";
import type { ReactNode } from "react";
import { CodeBlock } from "@/components/CodeBlock";
import { DocsPageShell } from "@/components/DocsPageShell";
import { Breadcrumbs } from "@/components/Breadcrumbs";
import { buildSidebar } from "@/lib/sidebar";

const QUICK_START = `// hello.ax — your first agent program
fn main() uses { Console } {
    print("Hello from Axon!")
}`;

const READING_PATHS: {
  href: string;
  title: string;
  body: string;
  badge: string;
  icon: ReactNode;
}[] = [
  {
    href: "/docs/spec",
    title: "Read the spec",
    body: "PLAN.md split into per-chapter pages — types, agents, effects, runtime, deployment.",
    badge: "spec",
    icon: <BookIcon />,
  },
  {
    href: "/docs/features",
    title: "Browse features by stage",
    body: "FEATURES.md as a chapter-per-stage tour — every entry links to source crates and tests.",
    badge: "live",
    icon: <LayersIcon />,
  },
  {
    href: "/docs/examples",
    title: "See runnable examples",
    body: "Real `.ax` programs that exercise the spec end-to-end — copy + run with `axon run`.",
    badge: "code",
    icon: <TerminalIcon />,
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

      <header className="mb-12 max-w-3xl">
        <div className="mb-4 inline-flex items-center gap-2 rounded-full border border-copper-700/40 bg-copper-950/40 px-3 py-1 text-xs font-medium text-copper-300">
          <span className="h-1.5 w-1.5 rounded-full bg-copper-400" />
          Documentation
        </div>
        <h1 className="font-display text-4xl font-semibold tracking-tight text-white sm:text-5xl">
          Welcome to{" "}
          <span className="bg-gradient-to-br from-copper-300 to-copper-500 bg-clip-text text-transparent">
            Axon
          </span>
          .
        </h1>
        <p className="mt-5 text-lg leading-relaxed text-ink-300">
          A typed, capability-safe, replayable language for production AI
          agents. Read the spec one chapter at a time, browse implemented
          features by stage, or open a runnable example.
        </p>
      </header>

      <SectionHeading kicker="Step 1" title="Install" />
      <p className="mb-5 max-w-3xl text-ink-300">
        One-line installer for macOS, Linux, and WSL. Windows is supported via
        the same script under PowerShell.
      </p>
      <div className="mb-14 max-w-3xl">
        <CodeBlock
          code={"curl -sSf axon-lang.org/install.sh | sh"}
          lang="bash"
          filename="install"
        />
      </div>

      <SectionHeading kicker="Step 2" title="Your first program" />
      <p className="mb-5 max-w-3xl text-ink-300">
        Save the snippet below as{" "}
        <code className="rounded-md bg-ink-800/80 px-1.5 py-0.5 font-mono text-[0.88em] text-copper-300 ring-1 ring-inset ring-ink-700/60">
          hello.ax
        </code>
        , then run{" "}
        <code className="rounded-md bg-ink-800/80 px-1.5 py-0.5 font-mono text-[0.88em] text-copper-300 ring-1 ring-inset ring-ink-700/60">
          axon run hello.ax
        </code>
        . The Console effect on the function signature is the only capability
        the program asks for — anything else would be a compile-time error.
      </p>
      <div className="mb-16 max-w-3xl">
        <CodeBlock code={QUICK_START} filename="hello.ax" lang="axon" />
      </div>

      <SectionHeading kicker="Next" title="Where to read next" />
      <p className="mb-6 max-w-3xl text-ink-300">
        The docs are organized into focused chapters. Pick a reading path that
        matches what you need today.
      </p>
      <div className="mb-16 grid gap-4 md:grid-cols-3">
        {READING_PATHS.map((c) => (
          <Link
            key={c.href}
            href={c.href}
            className="card-glow group flex flex-col rounded-2xl border border-ink-800 bg-ink-900/40 p-5"
          >
            <div className="mb-4 flex items-center justify-between">
              <span className="inline-flex h-10 w-10 items-center justify-center rounded-xl bg-copper-500/10 text-copper-300 ring-1 ring-inset ring-copper-500/20">
                {c.icon}
              </span>
              <span className="rounded bg-ink-800 px-2 py-0.5 font-mono text-[10px] uppercase tracking-wider text-ink-400">
                {c.badge}
              </span>
            </div>
            <h3 className="font-display text-lg font-semibold text-white">
              {c.title}
            </h3>
            <p className="mt-2 flex-1 text-sm leading-6 text-ink-300">
              {c.body}
            </p>
            <span className="mt-4 inline-flex items-center gap-1 text-sm font-medium text-copper-400">
              Read
              <span className="transition-transform group-hover:translate-x-1">
                →
              </span>
            </span>
          </Link>
        ))}
      </div>

      <SectionHeading kicker="Under the hood" title="The pipeline at a glance" />
      <p className="mb-8 max-w-3xl text-ink-300">
        An Axon program is parsed, type-checked, and executed by the same
        compiler that produces WASM artifacts. Every stage is observable via the
        CLI —{" "}
        <code className="rounded-md bg-ink-800/80 px-1.5 py-0.5 font-mono text-[0.88em] text-copper-300 ring-1 ring-inset ring-ink-700/60">
          axon tokens
        </code>
        ,{" "}
        <code className="rounded-md bg-ink-800/80 px-1.5 py-0.5 font-mono text-[0.88em] text-copper-300 ring-1 ring-inset ring-ink-700/60">
          axon check
        </code>
        ,{" "}
        <code className="rounded-md bg-ink-800/80 px-1.5 py-0.5 font-mono text-[0.88em] text-copper-300 ring-1 ring-inset ring-ink-700/60">
          axon run --trace
        </code>
        .
      </p>
      <div className="relative mb-12 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {PIPELINE.map((p, i) => (
          <div
            key={p.step}
            className="relative rounded-2xl border border-ink-800 bg-ink-900/40 p-5"
          >
            {i < PIPELINE.length - 1 ? (
              <span
                aria-hidden
                className="absolute -right-3 top-9 z-10 hidden text-ink-700 lg:block"
              >
                →
              </span>
            ) : null}
            <div className="flex items-center gap-3">
              <span className="inline-flex h-8 w-8 items-center justify-center rounded-lg bg-gradient-to-br from-copper-500/25 to-copper-700/10 font-mono text-xs font-bold text-copper-300 ring-1 ring-inset ring-copper-500/20">
                {p.step}
              </span>
              <h4 className="font-display font-semibold text-white">
                {p.title}
              </h4>
            </div>
            <p className="mt-3 text-sm leading-6 text-ink-300">{p.body}</p>
          </div>
        ))}
      </div>
    </DocsPageShell>
  );
}

function SectionHeading({ kicker, title }: { kicker: string; title: string }) {
  return (
    <div className="mb-3 flex items-center gap-3">
      <h2 className="font-display text-2xl font-semibold tracking-tight text-white">
        {title}
      </h2>
      <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-copper-400/70">
        {kicker}
      </span>
    </div>
  );
}

function BookIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M4 5.5A2.5 2.5 0 0 1 6.5 3H20v15H6.5A2.5 2.5 0 0 0 4 20.5z" />
      <path d="M4 20.5A2.5 2.5 0 0 1 6.5 18H20" />
    </svg>
  );
}

function LayersIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="m12 3 9 5-9 5-9-5z" />
      <path d="m3 13 9 5 9-5" />
      <path d="m3 17 9 5 9-5" />
    </svg>
  );
}

function TerminalIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="m5 8 4 4-4 4" />
      <path d="M13 16h6" />
      <rect x="2" y="3" width="20" height="18" rx="2.5" />
    </svg>
  );
}
