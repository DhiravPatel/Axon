import type { Config } from "tailwindcss";

const config: Config = {
  content: [
    "./app/**/*.{ts,tsx,mdx}",
    "./components/**/*.{ts,tsx}",
    "./lib/**/*.{ts,tsx}",
  ],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        // Brand palette — warm copper "Axon" with a charcoal neutral.
        ink: {
          50: "#f6f7f9",
          100: "#eceef2",
          200: "#d5d9e0",
          300: "#aab1be",
          400: "#7c8492",
          500: "#525a68",
          600: "#3a4250",
          700: "#272e3b",
          800: "#161c28",
          900: "#0b1018",
          950: "#05080d",
        },
        copper: {
          50: "#fdf6f2",
          100: "#faeae0",
          200: "#f3cdb6",
          300: "#e8a786",
          400: "#dd7e58",
          500: "#cf5b34",
          600: "#b4451f",
          700: "#8f361a",
          800: "#6a281a",
          900: "#451c16",
          950: "#260f0c",
        },
      },
      fontFamily: {
        sans: ['"Inter"', "ui-sans-serif", "system-ui", "sans-serif"],
        mono: [
          '"JetBrains Mono"',
          '"Fira Code"',
          "ui-monospace",
          "SFMono-Regular",
          "Menlo",
          "monospace",
        ],
        display: ['"Space Grotesk"', "ui-sans-serif", "system-ui", "sans-serif"],
      },
      typography: ({ theme }: { theme: (path: string) => string }) => ({
        DEFAULT: {
          css: {
            "--tw-prose-body": theme("colors.ink.700"),
            "--tw-prose-headings": theme("colors.ink.900"),
            "--tw-prose-links": theme("colors.copper.600"),
            "--tw-prose-bold": theme("colors.ink.900"),
            "--tw-prose-code": theme("colors.copper.700"),
            "--tw-prose-pre-bg": theme("colors.ink.900"),
            "--tw-prose-pre-code": theme("colors.ink.100"),
          },
        },
      }),
    },
  },
  plugins: [],
};

export default config;
