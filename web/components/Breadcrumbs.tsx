import Link from "next/link";

export interface Crumb {
  href?: string;
  label: string;
}

export function Breadcrumbs({ items }: { items: Crumb[] }) {
  return (
    <nav aria-label="Breadcrumb" className="mb-6 text-xs text-ink-500">
      <ol className="flex flex-wrap items-center gap-1.5">
        {items.map((c, i) => (
          <li key={i} className="flex items-center gap-1.5">
            {c.href ? (
              <Link
                href={c.href}
                className="rounded px-1 transition-colors hover:text-copper-300"
              >
                {c.label}
              </Link>
            ) : (
              <span className="text-ink-300">{c.label}</span>
            )}
            {i < items.length - 1 ? (
              <span aria-hidden className="text-ink-700">
                /
              </span>
            ) : null}
          </li>
        ))}
      </ol>
    </nav>
  );
}
