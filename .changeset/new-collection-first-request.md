---
"fiber": patch
---

Creating a collection now shows its first request immediately. The new section was mutated as a plain object after `$state` had already proxied it, so the sidebar never saw the push.
