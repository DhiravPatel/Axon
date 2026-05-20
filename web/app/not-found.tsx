import Link from "next/link";

export default function NotFound() {
  return (
    <section className="mx-auto max-w-3xl px-6 py-24 text-center">
      <p className="font-mono text-xs uppercase tracking-[0.2em] text-copper-400">
        404 — not found
      </p>
      <h1 className="mt-3 font-display text-4xl font-semibold text-white">
        That page doesn't exist.
      </h1>
      <p className="mt-4 text-ink-300">
        It may have moved or never existed. Try the docs index, or head back
        home.
      </p>
      <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
        <Link
          href="/"
          className="rounded-md bg-copper-500 px-5 py-2.5 text-sm font-semibold text-ink-950 hover:bg-copper-400"
        >
          ← Home
        </Link>
        <Link
          href="/docs"
          className="rounded-md border border-ink-700 px-5 py-2.5 text-sm font-semibold text-ink-100 hover:border-ink-500 hover:bg-ink-900"
        >
          Docs
        </Link>
      </div>
    </section>
  );
}
