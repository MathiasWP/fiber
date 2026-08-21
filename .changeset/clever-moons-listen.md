---
"fiber": minor
---

Endpoints refresh themselves when you come back to the window, not only at startup — so a loader left open all day no longer shows yesterday's routes. A collection spins a small icon while it is refreshing, so an automatic refresh is something you can see rather than endpoints changing on their own. New loaders default to a five minute TTL; 0 still means "only when asked".
