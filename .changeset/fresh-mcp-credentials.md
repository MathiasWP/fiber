---
"fiber": patch
---

Invalidate cached authentication when saved credentials change across Fiber processes, and refresh once for explicit token-expiry errors returned as HTTP 403. MCP sign-in advice now covers expired-token responses too.
