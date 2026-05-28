"use client";

import { useEffect } from "react";

/**
 * Walk the rendered docs HTML once on mount and add a "Copy" button to
 * every `<pre class="shiki">…</pre>` block. Pure DOM mutation; no
 * children, no re-renders.
 *
 * This belongs on the client because navigator.clipboard isn't
 * available during SSG and because the button itself is interactive.
 */
export function PreEnhancer({ containerId }: { containerId: string }) {
  useEffect(() => {
    const root = document.getElementById(containerId);
    if (!root) return;
    const blocks = root.querySelectorAll<HTMLElement>("pre.shiki");
    const cleanups: Array<() => void> = [];

    blocks.forEach((pre) => {
      // Idempotent — don't double-attach if hydration re-runs.
      if (pre.querySelector(".copy-btn")) return;
      const btn = document.createElement("button");
      btn.className = "copy-btn";
      btn.type = "button";
      btn.textContent = "Copy";
      btn.setAttribute("aria-label", "Copy code to clipboard");
      const onClick = async () => {
        const code = pre.innerText.replace(/Copy$/, "").trim();
        try {
          await navigator.clipboard.writeText(code);
          btn.textContent = "Copied";
          btn.dataset.copied = "true";
          setTimeout(() => {
            btn.textContent = "Copy";
            delete btn.dataset.copied;
          }, 1400);
        } catch {
          btn.textContent = "Copy failed";
          setTimeout(() => {
            btn.textContent = "Copy";
          }, 1400);
        }
      };
      btn.addEventListener("click", onClick);
      pre.appendChild(btn);
      cleanups.push(() => {
        btn.removeEventListener("click", onClick);
        btn.remove();
      });
    });

    return () => cleanups.forEach((fn) => fn());
  }, [containerId]);

  return null;
}
