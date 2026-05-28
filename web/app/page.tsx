import Link from "next/link";
import { CodeBlock } from "@/components/CodeBlock";

const HERO_SAMPLE = `agent Researcher(m: Model, mem: Memory) {
    on inquiry(question: String) -> String uses { LLM, Net, Memory } {
        let ctx = self.mem.recall(question)
        ask self.m {
            system: "Cite every claim. Use the search tool when unsure."
            memory: ctx
            user:   question
            tools:  [search]
        }
    }
}

fn main() uses { Spawn, LLM, Net, Memory, Console } {
    let r = spawn Researcher(m = anthropic("claude-opus-4-7"), mem = local_memory())
    let answer = r.inquiry("What changed in the EU AI Act in 2025?")
    print(answer)
}`;

const PILLARS = [
  {
    title: "Typed effect rows",
    body: "Every function declares what it touches — Net, Fs, LLM, Spawn, Memory. Capability checks run at compile time *and* at runtime; ambient authority is impossible.",
  },
  {
    title: "Agents as first-class values",
    body: "`agent`, `actor`, `tool`, `model`, `memory`, `prompt` are language constructs. Spawn, message, supervise — no framework required.",
  },
  {
    title: "Deterministic by default",
    body: "Every model call, randomness source, and wall clock read is recorded. `axon replay` re-executes a run byte-identically against the captured tape.",
  },
  {
    title: "Production-ready toolchain",
    body: "Multi-file projects, LSP, formatter, doc generator, WASM target, sandboxed FFI, OAuth vault, TLS-terminated serve, OTLP traces, cost ledger.",
  },
  {
    title: "Built-in safety",
    body: "Tainted<T> for untrusted input. Policy blocks with allow/deny/budget/rate around every effect. Red-team eval suites + sandboxed tool subprocesses.",
  },
  {
    title: "Multi-agent orchestration",
    body: "Networks with cycle analysis, workflow graphs with topological scheduling, debate / tree-of-thought / consensus voting — all typed, all traced.",
  },
];

const STATS = [
  { value: "29", label: "Build stages shipped" },
  { value: "859", label: "Tests passing" },
  { value: "30+", label: "Crates" },
  { value: "v1.0", label: "Spec complete" },
];

export default function HomePage() {
  return (
    <>
      <Hero />
      <Pillars />
      <PlanLoop />
      <Stats />
      <CTA />
    </>
  );
}

async function Hero() {
  return (
    <section className="relative overflow-hidden">
      <div className="hero-grid absolute inset-0" aria-hidden />
      <div className="relative mx-auto grid max-w-7xl gap-12 px-6 pt-20 pb-24 lg:grid-cols-[1.1fr_1fr] lg:gap-16 lg:pt-28 lg:pb-32">
        <div className="flex flex-col justify-center">
          <div className="mb-6 inline-flex items-center gap-2 self-start rounded-full border border-copper-700/40 bg-copper-950/40 px-3 py-1 text-xs font-medium text-copper-300">
            <span className="h-1.5 w-1.5 rounded-full bg-copper-400" />
            v0.1 · 29 stages shipped
          </div>
          <h1 className="font-display text-5xl font-semibold leading-tight tracking-tight text-white lg:text-6xl">
            The programming language
            <br />
            <span className="bg-gradient-to-br from-copper-300 to-copper-500 bg-clip-text text-transparent">
              for autonomous AI agents.
            </span>
          </h1>
          <p className="mt-6 max-w-xl text-lg leading-relaxed text-ink-300">
            Axon is typed, capability-safe, and replayable. Agents, tools,
            models, and prompts are first-class — the language itself enforces
            the invariants you used to write framework code for.
          </p>
          <div className="mt-8 flex flex-wrap items-center gap-3">
            <Link
              href="/docs"
              className="inline-flex items-center gap-2 rounded-md bg-copper-500 px-5 py-2.5 text-sm font-semibold text-ink-950 transition-colors hover:bg-copper-400"
            >
              Read the docs
              <span aria-hidden>→</span>
            </Link>
            <a
              href="https://github.com/axon-lang/axon"
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-2 rounded-md border border-ink-700 px-5 py-2.5 text-sm font-semibold text-ink-100 transition-colors hover:border-ink-500 hover:bg-ink-900"
            >
              <GithubIcon />
              View on GitHub
            </a>
          </div>
          <div className="mt-8 rounded-lg border border-ink-800 bg-ink-900/60 px-4 py-3 text-sm font-mono text-ink-300">
            <span className="text-ink-500">$</span>{" "}
            <span className="text-copper-300">curl</span>
            {" -sSf "}
            <span className="text-ink-200">axon-lang.org/install.sh</span>
            {" | sh"}
          </div>
        </div>
        <div className="flex items-start lg:items-stretch">
          <div className="w-full self-stretch">
            <CodeBlock
              code={HERO_SAMPLE}
              filename="researcher.ax"
              lang="axon"
            />
          </div>
        </div>
      </div>
    </section>
  );
}

function Pillars() {
  return (
    <section className="border-t border-ink-800 bg-ink-950">
      <div className="mx-auto max-w-7xl px-6 py-20">
        <div className="mb-12 max-w-3xl">
          <p className="font-mono text-xs uppercase tracking-[0.2em] text-copper-400">
            Why Axon
          </p>
          <h2 className="mt-3 font-display text-3xl font-semibold text-white sm:text-4xl">
            A language designed for the things agents actually do.
          </h2>
          <p className="mt-4 text-ink-300">
            Effect rows, capability tokens, signed identity, durable timers,
            sandboxed FFI — production agent infrastructure built into the
            type system instead of bolted on.
          </p>
        </div>
        <div className="grid gap-px overflow-hidden rounded-2xl border border-ink-800 bg-ink-800 md:grid-cols-2 lg:grid-cols-3">
          {PILLARS.map((p) => (
            <div
              key={p.title}
              className="flex flex-col bg-ink-950 p-8 transition-colors hover:bg-ink-900"
            >
              <h3 className="font-display text-lg font-semibold text-white">
                {p.title}
              </h3>
              <p className="mt-3 text-sm leading-6 text-ink-300">{p.body}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

async function PlanLoop() {
  const sample = `let answer = plan with brain {
    system:   "Solve step by step. Cite every claim."
    user:     question
    tools:    [search, calc]
    output:   Answer
    strategy: TreeOfThought(width = 4, depth = 3, scorer = judge)
    budget:   budget(usd = 0.20, tokens = 60_000)
    on_step_error |e| match e {
        ToolError.RateLimited(..) => Directive.Backoff(2s)
        ValidationError(..)       => Directive.Repair
        BudgetExceeded(..)        => Directive.FinalizeBest
        _                         => Directive.Escalate(to = human)
    }
} await`;

  return (
    <section className="border-t border-ink-800">
      <div className="mx-auto grid max-w-7xl gap-12 px-6 py-20 lg:grid-cols-[1fr_1.2fr] lg:items-center">
        <div>
          <p className="font-mono text-xs uppercase tracking-[0.2em] text-copper-400">
            The plan block
          </p>
          <h2 className="mt-3 font-display text-3xl font-semibold text-white sm:text-4xl">
            Think → act → observe.
            <br />
            With reasoning budgets you can audit.
          </h2>
          <p className="mt-4 text-ink-300">
            `plan` is the built-in agentic loop. Swap the loop shape (ReAct,
            PlanExecute, Reflexion, TreeOfThought, Debate, Custom). Cap
            thinking-token spend separately from output tokens. Route step
            failures through a typed directive set.
          </p>
          <ul className="mt-6 space-y-2 text-sm text-ink-300">
            <li className="flex gap-3">
              <span className="text-copper-400">▸</span>
              Capability checks fire around every tool call
            </li>
            <li className="flex gap-3">
              <span className="text-copper-400">▸</span>
              Budgets compose; deepest scope wins
            </li>
            <li className="flex gap-3">
              <span className="text-copper-400">▸</span>
              `on_step_error` is a value, not a callback hell
            </li>
          </ul>
        </div>
        <CodeBlock code={sample} filename="plan_example.ax" lang="axon" />
      </div>
    </section>
  );
}

function Stats() {
  return (
    <section className="border-t border-ink-800 bg-gradient-to-b from-ink-950 to-ink-900">
      <div className="mx-auto max-w-7xl px-6 py-16">
        <div className="grid grid-cols-2 gap-px overflow-hidden rounded-2xl border border-ink-800 bg-ink-800 sm:grid-cols-4">
          {STATS.map((s) => (
            <div key={s.label} className="bg-ink-950 px-6 py-8 text-center">
              <div className="font-display text-4xl font-semibold text-white">
                {s.value}
              </div>
              <div className="mt-2 font-mono text-xs uppercase tracking-wider text-ink-400">
                {s.label}
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function CTA() {
  return (
    <section className="border-t border-ink-800 bg-ink-950">
      <div className="mx-auto max-w-4xl px-6 py-24 text-center">
        <h2 className="font-display text-3xl font-semibold text-white sm:text-4xl">
          Ready to write your first agent?
        </h2>
        <p className="mt-4 text-ink-300">
          The docs walk you from `hello world` to a multi-agent system with
          policies, durable triggers, and an OTLP trace pipeline.
        </p>
        <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
          <Link
            href="/docs"
            className="inline-flex items-center gap-2 rounded-md bg-copper-500 px-6 py-3 text-sm font-semibold text-ink-950 transition-colors hover:bg-copper-400"
          >
            Start with the overview
            <span aria-hidden>→</span>
          </Link>
          <Link
            href="/docs/examples"
            className="inline-flex items-center gap-2 rounded-md border border-ink-700 px-6 py-3 text-sm font-semibold text-ink-100 transition-colors hover:border-ink-500 hover:bg-ink-900"
          >
            Browse examples
          </Link>
        </div>
      </div>
    </section>
  );
}

function GithubIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <path d="M12 .5C5.65.5.5 5.65.5 12.05c0 5.1 3.3 9.41 7.88 10.94.58.1.79-.25.79-.55v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.34-1.28-1.7-1.28-1.7-1.04-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.76 2.7 1.25 3.36.95.1-.74.4-1.25.73-1.54-2.56-.29-5.26-1.28-5.26-5.7 0-1.26.45-2.29 1.18-3.1-.12-.29-.51-1.46.11-3.05 0 0 .97-.31 3.18 1.18.92-.26 1.91-.39 2.89-.39.98 0 1.97.13 2.89.39 2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.12 3.05.73.81 1.18 1.84 1.18 3.1 0 4.43-2.7 5.41-5.27 5.69.41.36.78 1.06.78 2.15v3.19c0 .31.21.66.8.55C20.21 21.46 23.5 17.15 23.5 12.05 23.5 5.65 18.35.5 12 .5z" />
    </svg>
  );
}
