/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // Build a static export so the site can ship to any object store + CDN.
  output: "export",
  trailingSlash: true,
  // The repo lives at the workspace root; allow Next to read source files
  // sitting two levels up (PLAN.md, FEATURES.md, examples/) at build time.
  outputFileTracingRoot: process.cwd().replace(/\/web$/, ""),
  images: {
    unoptimized: true,
  },
};

export default nextConfig;
