import Link from "next/link";

const NAV = [
  { href: "/docs", label: "Docs" },
  { href: "/docs/features", label: "Features" },
  { href: "/docs/spec", label: "Spec" },
  { href: "/docs/examples", label: "Examples" },
];

export function Header() {
  return (
    <header className="sticky top-0 z-40 border-b border-ink-800/80 bg-ink-950/70 backdrop-blur-xl">
      <div className="mx-auto flex h-16 max-w-7xl items-center justify-between px-6">
        <Link
          href="/"
          className="flex items-center gap-3 font-display text-lg font-semibold tracking-tight"
        >
          <span
            aria-hidden
            className="inline-flex h-8 w-8 items-center justify-center rounded-md bg-gradient-to-br from-copper-400 to-copper-700 text-ink-950 font-bold"
          >
            ▲
          </span>
          <span className="text-white">Axon</span>
          <span className="hidden text-ink-500 sm:inline">— a language for agents</span>
        </Link>
        <nav className="hidden items-center gap-6 md:flex">
          {NAV.map((item) => (
            <Link
              key={item.href}
              href={item.href}
              className="text-sm text-ink-300 transition-colors hover:text-white"
            >
              {item.label}
            </Link>
          ))}
          <a
            href="https://github.com/axon-lang/axon"
            target="_blank"
            rel="noopener noreferrer"
            className="rounded-md border border-ink-700 px-3 py-1.5 text-sm text-ink-200 transition-colors hover:border-copper-500 hover:text-white"
          >
            GitHub
          </a>
        </nav>
      </div>
    </header>
  );
}
