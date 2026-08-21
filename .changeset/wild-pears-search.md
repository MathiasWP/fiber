---
"fiber": patch
---

The sidebar does much less work per render with a large collection: the loaded-endpoint rows were rebuilt five times over on every update, and matching them against your saved bodies was quadratic. Opening several endpoints in a row now writes the collection once rather than once each.
