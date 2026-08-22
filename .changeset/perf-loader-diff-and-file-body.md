---
"fiber": patch
---

A few performance fixes: computing what a loader refresh added or removed was quadratic in the number of endpoints, sending a request whose body comes from a file blocked the async runtime instead of reading it off-thread, and the sidebar's "Move to" submenu recomputed its target list twice per request row.
