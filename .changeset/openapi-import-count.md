---
"fiber": patch
---

Importing an OpenAPI spec now reports how many endpoints were actually added. The count used to re-read a live list after those endpoints had already been pushed, so it always said zero.
