import { notFound } from "next/navigation";
import { DocsPageShell } from "@/components/DocsPageShell";
import { ChapterView } from "@/components/ChapterView";
import { buildSidebar } from "@/lib/sidebar";
import { loadChapter, loadChapters } from "@/lib/sections";

interface Props {
  params: Promise<{ slug: string }>;
}

export async function generateStaticParams() {
  const chapters = await loadChapters("PLAN.md");
  return chapters.map((c) => ({ slug: c.slug }));
}

export async function generateMetadata({ params }: Props) {
  const { slug } = await params;
  const chapter = await loadChapter("PLAN.md", slug);
  if (!chapter) return { title: "Not found" };
  return {
    title: chapter.title,
    description: chapter.blurb || `Axon spec — ${chapter.title}.`,
  };
}

export default async function SpecChapterPage({ params }: Props) {
  const { slug } = await params;
  const [sidebar, chapter, all] = await Promise.all([
    buildSidebar(),
    loadChapter("PLAN.md", slug),
    loadChapters("PLAN.md"),
  ]);
  if (!chapter) notFound();
  return (
    <DocsPageShell
      sidebar={sidebar}
      activeHref={`/docs/spec/${slug}`}
      toc={chapter.toc}
    >
      <ChapterView
        chapter={chapter}
        all={all}
        baseHref="/docs/spec"
        eyebrow="Spec"
        crumbs={[
          { href: "/", label: "Home" },
          { href: "/docs", label: "Docs" },
          { href: "/docs/spec", label: "Spec" },
          { label: chapter.title },
        ]}
      />
    </DocsPageShell>
  );
}
