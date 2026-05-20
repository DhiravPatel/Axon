import type { Metadata } from "next";
import "./globals.css";
import { Header } from "@/components/Header";
import { Footer } from "@/components/Footer";

export const metadata: Metadata = {
  title: {
    default: "Axon — The Programming Language for Autonomous AI Agents",
    template: "%s · Axon",
  },
  description:
    "Axon is a typed, capability-safe, replayable programming language for building production AI agents. Effect rows, agents as first-class values, durable triggers, signed identity, sandboxed tools.",
  metadataBase: new URL("https://axon-lang.org"),
  openGraph: {
    title: "Axon — The Programming Language for Autonomous AI Agents",
    description:
      "Typed, capability-safe, replayable. Agents as first-class values.",
    type: "website",
  },
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" className="dark">
      <body>
        <Header />
        <main>{children}</main>
        <Footer />
      </body>
    </html>
  );
}
