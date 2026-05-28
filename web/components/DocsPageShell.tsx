import type { ReactNode } from "react";
import { DocsSidebar, type SidebarGroup } from "./DocsSidebar";
import { TableOfContents } from "./TableOfContents";
import type { TocEntry } from "@/lib/sections";

interface Props {
  sidebar: SidebarGroup[];
  activeHref: string;
  toc?: TocEntry[];
  children: ReactNode;
}

export function DocsPageShell({
  sidebar,
  activeHref,
  toc,
  children,
}: Props) {
  return (
    <>
      <DocsSidebar groups={sidebar} activeHref={activeHref} />
      <article className="flex-1 min-w-0 py-8 lg:px-8">{children}</article>
      <TableOfContents entries={toc ?? []} />
    </>
  );
}
