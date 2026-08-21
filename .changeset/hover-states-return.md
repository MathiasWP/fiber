---
"fiber": patch
---

Hover states that never were. The section cog, the two add-buttons in the sidebar header, header/param delete buttons, and a handful of others were written with UnoCSS variant-group syntax — `hover:(bg-border text-text)` — which the PostCSS pipeline never expands: it scans class names but does not rewrite source, so the browser received split-by-space junk and no rule matched. Every one is now written out in full, and the transformer that was quietly doing nothing is gone from the config, with a comment explaining the trap.
