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
    <nav className="scroll-thin sticky top-16 hidden h-[calc(100vh-4rem)] w-64 shrink-0 overflow-y-auto border-r border-ink-800/70 py-8 pr-5 lg:block">
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
    <p className="mb-2.5 px-3 font-mono text-[10.5px] font-semibold uppercase tracking-[0.18em] text-ink-500">
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
      <summary className="mb-2.5 flex cursor-pointer list-none items-center justify-between rounded-md px-3 py-1 font-mono text-[10.5px] font-semibold uppercase tracking-[0.18em] text-ink-500 transition-colors hover:text-ink-300">
        <span>{group.heading}</span>
        <span
          aria-hidden
          className="text-ink-600 transition-transform duration-200 group-open:rotate-90"
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
      aria-current={active ? "page" : undefined}
      className={
        "group/link relative flex items-baseline gap-2 rounded-md py-1.5 pl-3 pr-2 text-sm transition-colors " +
        (active
          ? "bg-copper-500/10 font-medium text-copper-200"
          : "text-ink-300 hover:bg-ink-800/50 hover:text-white")
      }
    >
      <span
        aria-hidden
        className={
          "absolute left-0 top-1/2 h-4 w-0.5 -translate-y-1/2 rounded-full transition-all " +
          (active
            ? "bg-copper-400"
            : "bg-transparent group-hover/link:bg-ink-600")
        }
      />
      {prefix ? (
        <span
          className={
            "shrink-0 font-mono text-[10px] uppercase tracking-wider " +
            (active ? "text-copper-400/80" : "text-ink-500")
          }
        >
          {prefix}
        </span>
      ) : null}
      <span className="min-w-0 truncate">{label}</span>
    </Link>
  );
}
