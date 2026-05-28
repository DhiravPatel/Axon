import Link from "next/link";

interface NavTarget {
  href: string;
  title: string;
}

interface Props {
  prev?: NavTarget | null;
  next?: NavTarget | null;
}

export function PrevNext({ prev, next }: Props) {
  if (!prev && !next) return null;
  return (
    <nav className="mt-16 grid gap-3 border-t border-ink-800 pt-8 sm:grid-cols-2">
      {prev ? (
        <Link
          href={prev.href}
          className="group flex flex-col items-start rounded-lg border border-ink-800 bg-ink-950/40 p-4 transition-colors hover:border-copper-700/50 hover:bg-ink-900/40"
        >
          <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-ink-500">
            ← Previous
          </span>
          <span className="mt-1 font-display text-sm font-semibold text-white transition-colors group-hover:text-copper-300">
            {prev.title}
          </span>
        </Link>
      ) : (
        <div aria-hidden />
      )}
      {next ? (
        <Link
          href={next.href}
          className="group flex flex-col items-end rounded-lg border border-ink-800 bg-ink-950/40 p-4 text-right transition-colors hover:border-copper-700/50 hover:bg-ink-900/40"
        >
          <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-ink-500">
            Next →
          </span>
          <span className="mt-1 font-display text-sm font-semibold text-white transition-colors group-hover:text-copper-300">
            {next.title}
          </span>
        </Link>
      ) : (
        <div aria-hidden />
      )}
    </nav>
  );
}
