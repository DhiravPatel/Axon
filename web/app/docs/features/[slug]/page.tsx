import { notFound } from "next/navigation";
import { DocsPageShell } from "@/components/DocsPageShell";
import { ChapterView } from "@/components/ChapterView";
import { buildSidebar } from "@/lib/sidebar";
import { loadChapter, loadChapters } from "@/lib/sections";

interface Props {
  params: Promise<{ slug: string }>;
}

export async function generateStaticParams() {
  const chapters = await loadChapters("FEATURES.md");
  return chapters.map((c) => ({ slug: c.slug }));
}

export async function generateMetadata({ params }: Props) {
  const { slug } = await params;
  const chapter = await loadChapter("FEATURES.md", slug);
  if (!chapter) return { title: "Not found" };
  return {
    title: chapter.title,
    description: chapter.blurb || `Axon features — ${chapter.title}.`,
  };
}

export default async function FeatureChapterPage({ params }: Props) {
  const { slug } = await params;
  const [sidebar, chapter, all] = await Promise.all([
    buildSidebar(),
    loadChapter("FEATURES.md", slug),
    loadChapters("FEATURES.md"),
  ]);
  if (!chapter) notFound();
  return (
    <DocsPageShell
      sidebar={sidebar}
      activeHref={`/docs/features/${slug}`}
      toc={chapter.toc}
    >
      <ChapterView
        chapter={chapter}
        all={all}
        baseHref="/docs/features"
        eyebrow="Feature"
        crumbs={[
          { href: "/", label: "Home" },
          { href: "/docs", label: "Docs" },
          { href: "/docs/features", label: "Features" },
          { label: chapter.title },
        ]}
      />
    </DocsPageShell>
  );
}
