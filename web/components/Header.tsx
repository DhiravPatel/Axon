import Link from "next/link";

const NAV = [
  { href: "/docs", label: "Docs" },
  { href: "/docs/features", label: "Features" },
  { href: "/docs/spec", label: "Spec" },
  { href: "/docs/examples", label: "Examples" },
];

export function Header() {
  return (
    <header className="sticky top-0 z-40 border-b border-ink-800/70 bg-ink-950/60 backdrop-blur-xl">
      <div className="mx-auto flex h-16 max-w-[88rem] items-center justify-between px-4 sm:px-6 lg:px-8">
        <Link
          href="/"
          className="group flex items-center gap-2.5 font-display text-lg font-semibold tracking-tight"
        >
          <AxonMark />
          <span className="text-white">Axon</span>
          <span className="hidden text-ink-500 sm:inline">
            — a language for agents
          </span>
        </Link>
        <nav className="flex items-center gap-1 md:gap-2">
          <div className="hidden items-center md:flex">
            {NAV.map((item) => (
              <Link
                key={item.href}
                href={item.href}
                className="rounded-md px-3 py-1.5 text-sm text-ink-300 transition-colors hover:bg-ink-800/60 hover:text-white"
              >
                {item.label}
              </Link>
            ))}
          </div>
          <span aria-hidden className="mx-1 hidden h-5 w-px bg-ink-800 md:inline-block" />
          <a
            href="https://github.com/axon-lang/axon"
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-2 rounded-md border border-ink-700/70 bg-ink-900/40 px-3 py-1.5 text-sm text-ink-200 transition-colors hover:border-copper-600/70 hover:text-white"
          >
            <GithubIcon />
            <span className="hidden sm:inline">GitHub</span>
          </a>
        </nav>
      </div>
    </header>
  );
}

function AxonMark() {
  return (
    <span
      aria-hidden
      className="relative inline-flex h-8 w-8 items-center justify-center rounded-lg bg-gradient-to-br from-copper-400 to-copper-700 shadow-lg shadow-copper-900/40 ring-1 ring-inset ring-white/10 transition-transform group-hover:scale-105"
    >
      <svg width="17" height="17" viewBox="0 0 24 24" fill="none" aria-hidden>
        <path
          d="M12 3 4 20h3.4l1.5-3.4h6.2L16.6 20H20L12 3Zm-1.7 10.6L12 9.4l1.7 4.2h-3.4Z"
          fill="#fff"
        />
      </svg>
    </span>
  );
}

function GithubIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M12 .5C5.65.5.5 5.65.5 12.05c0 5.1 3.3 9.41 7.88 10.94.58.1.79-.25.79-.55v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.34-1.28-1.7-1.28-1.7-1.04-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.76 2.7 1.25 3.36.95.1-.74.4-1.25.73-1.54-2.56-.29-5.26-1.28-5.26-5.7 0-1.26.45-2.29 1.18-3.1-.12-.29-.51-1.46.11-3.05 0 0 .97-.31 3.18 1.18.92-.26 1.91-.39 2.89-.39.98 0 1.97.13 2.89.39 2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.12 3.05.73.81 1.18 1.84 1.18 3.1 0 4.43-2.7 5.41-5.27 5.69.41.36.78 1.06.78 2.15v3.19c0 .31.21.66.8.55C20.21 21.46 23.5 17.15 23.5 12.05 23.5 5.65 18.35.5 12 .5z" />
    </svg>
  );
}
