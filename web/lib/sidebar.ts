import type { SidebarGroup } from "@/components/DocsSidebar";
import { loadChapters } from "./sections";

/**
 * Build the docs sidebar once per request. The spec + features groups
 * are filled live from the source markdown so the sidebar never drifts
 * from what's actually published.
 */
export async function buildSidebar(): Promise<SidebarGroup[]> {
  const [spec, features] = await Promise.all([
    loadChapters("PLAN.md"),
    loadChapters("FEATURES.md"),
  ]);
  return [
    {
      heading: "Get started",
      items: [{ href: "/docs", label: "Overview" }],
    },
    {
      heading: "Reference",
      collapsible: false,
      items: [
        { href: "/docs/spec", label: "Language spec — overview" },
        { href: "/docs/features", label: "Implemented features" },
      ],
    },
    {
      heading: "Spec — chapters",
      collapsible: true,
      defaultOpen: false,
      items: spec.map((c) => ({
        href: `/docs/spec/${c.slug}`,
        label: shortenTitle(c.title),
        prefix: chapterPrefix(c.title),
      })),
    },
    {
      heading: "Features — by stage",
      collapsible: true,
      defaultOpen: false,
      items: features.map((c) => ({
        href: `/docs/features/${c.slug}`,
        label: shortenTitle(c.title),
        prefix: featurePrefix(c.title),
      })),
    },
    {
      heading: "Browse",
      items: [{ href: "/docs/examples", label: "Examples" }],
    },
  ];
}

/** Pull a leading section number out of the heading for the sidebar prefix. */
function chapterPrefix(title: string): string | undefined {
  const m = title.match(/^(\d+(?:\.\d+)?)\.?\s+/);
  if (m) return `§${m[1]}`;
  return undefined;
}

/** Pull the stage number out of a Features heading. */
function featurePrefix(title: string): string | undefined {
  const m = title.match(/^Stage\s+(\d+(?:\.\d+)?)/i);
  if (m) return `S${m[1]}`;
  return undefined;
}

function shortenTitle(title: string): string {
  // Drop the leading number for spec chapters so the prefix and label
  // don't repeat.
  const numeric = title.replace(/^\d+(?:\.\d+)?\.?\s+/, "");
  const stage = numeric.replace(/^Stage\s+\d+(?:\.\d+)?\s*[—-]\s*/i, "");
  return stage;
}
