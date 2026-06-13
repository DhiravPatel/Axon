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
      <article className="min-w-0 flex-1 py-10 lg:px-10">{children}</article>
      <TableOfContents entries={toc ?? []} />
    </>
  );
}
