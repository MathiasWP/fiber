---
"fiber": patch
---

Fix the freeze when sending a request. The waiting message picked a new line by reading the one it was about to replace, which re-triggered itself forever and threw, leaving the window painted but dead.
