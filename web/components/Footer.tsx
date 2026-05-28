import Link from "next/link";

export function Footer() {
  return (
    <footer className="border-t border-ink-800 bg-ink-950">
      <div className="mx-auto grid max-w-7xl grid-cols-2 gap-10 px-6 py-12 md:grid-cols-4">
        <div>
          <div className="flex items-center gap-2 font-display text-lg font-semibold text-white">
            <span
              aria-hidden
              className="inline-flex h-7 w-7 items-center justify-center rounded-md bg-gradient-to-br from-copper-400 to-copper-700 text-ink-950 font-bold"
            >
              ▲
            </span>
            Axon
          </div>
          <p className="mt-4 text-sm text-ink-400">
            A typed, capability-safe, replayable programming language for
            building production AI agents.
          </p>
        </div>
        <div>
          <h4 className="font-display text-sm font-semibold text-white">Docs</h4>
          <ul className="mt-3 space-y-2 text-sm text-ink-400">
            <li>
              <Link className="hover:text-white" href="/docs">
                Overview
              </Link>
            </li>
            <li>
              <Link className="hover:text-white" href="/docs/spec">
                Language spec
              </Link>
            </li>
            <li>
              <Link className="hover:text-white" href="/docs/features">
                Implemented features
              </Link>
            </li>
            <li>
              <Link className="hover:text-white" href="/docs/examples">
                Examples
              </Link>
            </li>
          </ul>
        </div>
        <div>
          <h4 className="font-display text-sm font-semibold text-white">Community</h4>
          <ul className="mt-3 space-y-2 text-sm text-ink-400">
            <li>
              <a
                className="hover:text-white"
                href="https://github.com/axon-lang/axon"
                target="_blank"
                rel="noopener noreferrer"
              >
                GitHub
              </a>
            </li>
            <li>
              <a
                className="hover:text-white"
                href="https://github.com/axon-lang/axon/issues"
                target="_blank"
                rel="noopener noreferrer"
              >
                Issues
              </a>
            </li>
            <li>
              <a
                className="hover:text-white"
                href="https://github.com/axon-lang/axon/discussions"
                target="_blank"
                rel="noopener noreferrer"
              >
                Discussions
              </a>
            </li>
          </ul>
        </div>
        <div>
          <h4 className="font-display text-sm font-semibold text-white">License</h4>
          <ul className="mt-3 space-y-2 text-sm text-ink-400">
            <li>Compiler · Apache-2.0</li>
            <li>Stdlib · Apache-2.0 OR MIT</li>
            <li>Spec · CC-BY-4.0</li>
          </ul>
        </div>
      </div>
      <div className="border-t border-ink-800">
        <div className="mx-auto flex max-w-7xl flex-col items-center justify-between gap-3 px-6 py-6 text-xs text-ink-500 md:flex-row">
          <p>© {new Date().getFullYear()} The Axon Authors.</p>
          <p>
            Built with care for agents that ship.
          </p>
        </div>
      </div>
    </footer>
  );
}
