import Link from "next/link";

export interface SidebarChapter {
  href: string;
  label: string;
  /** Optional short numeric / stage prefix shown before the label. */
  prefix?: string;
}

export interface SidebarGroup {
  heading: string;
  /** When true, items live behind an expandable disclosure summary. */
  collapsible?: boolean;
  /** When `collapsible`, whether to start expanded. */
  defaultOpen?: boolean;
  items: SidebarChapter[];
}

interface Props {
  activeHref: string;
  groups: SidebarGroup[];
}

export function DocsSidebar({ activeHref, groups }: Props) {
  return (
    <nav className="sticky top-20 hidden h-[calc(100vh-5rem)] w-72 shrink-0 overflow-y-auto border-r border-ink-800 py-8 pr-6 lg:block">
      <ul className="space-y-7">
        {groups.map((g) => (
          <li key={g.heading}>
            <SidebarSection group={g} activeHref={activeHref} />
          </li>
        ))}
      </ul>
    </nav>
  );
}

function SidebarSection({
  group,
  activeHref,
}: {
  group: SidebarGroup;
  activeHref: string;
}) {
  const isActiveInside = group.items.some((it) => it.href === activeHref);
  const heading = (
    <p className="mb-2 font-mono text-[11px] font-semibold uppercase tracking-[0.18em] text-ink-500">
      {group.heading}
    </p>
  );
  const list = (
    <ul className="space-y-0.5">
      {group.items.map((it) => (
        <li key={it.href}>
          <SidebarLink
            href={it.href}
            label={it.label}
            prefix={it.prefix}
            active={it.href === activeHref}
          />
        </li>
      ))}
    </ul>
  );
  if (!group.collapsible) {
    return (
      <>
        {heading}
        {list}
      </>
    );
  }
  return (
    <details className="group" open={group.defaultOpen ?? isActiveInside}>
      <summary className="mb-2 flex cursor-pointer list-none items-center justify-between font-mono text-[11px] font-semibold uppercase tracking-[0.18em] text-ink-500 hover:text-ink-300">
        <span>{group.heading}</span>
        <span
          aria-hidden
          className="text-ink-600 transition-transform group-open:rotate-90"
        >
          ›
        </span>
      </summary>
      {list}
    </details>
  );
}

function SidebarLink({
  href,
  label,
  prefix,
  active,
}: {
  href: string;
  label: string;
  prefix?: string;
  active: boolean;
}) {
  return (
    <Link
      href={href}
      className={
        "flex items-baseline gap-2 rounded-md px-3 py-1.5 text-sm transition-colors " +
        (active
          ? "bg-copper-950/40 text-copper-200"
          : "text-ink-300 hover:bg-ink-900 hover:text-white")
      }
    >
      {prefix ? (
        <span className="shrink-0 font-mono text-[10px] uppercase tracking-wider text-ink-500">
          {prefix}
        </span>
      ) : null}
      <span className="min-w-0 truncate">{label}</span>
    </Link>
  );
}
