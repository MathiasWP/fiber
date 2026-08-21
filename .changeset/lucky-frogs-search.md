---
"fiber": patch
---

Searching the endpoints no longer buries what you meant. A term that appears whole in a path now ranks above one whose letters merely appear in order, and when anything matches properly the near-misses are dropped rather than listed alongside — so `/list` stops returning every path containing those five letters somewhere. A typo, which has no proper match to lose to, still guesses as it did before.
