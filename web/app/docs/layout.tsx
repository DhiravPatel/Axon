import type { Metadata } from "next";

export const metadata: Metadata = {
  title: {
    default: "Docs",
    template: "%s · Axon docs",
  },
};

export default function DocsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="mx-auto flex max-w-[88rem] gap-0 px-4 sm:px-6 lg:px-8">
      {children}
    </div>
  );
}
