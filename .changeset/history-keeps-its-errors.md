---
"fiber": patch
---

A response body that fails to load, or a history clear that fails, is reported
in the History tab again instead of vanishing the moment the tab opens.

History now catches up with what the MCP server records whenever the tab is
opened, and a catch-up that succeeded cleared every error on the way — including
the one the tab was being opened to show. It now clears only an earlier failure
to list history, which is the one a successful list resolves.

Also moves rustls to 0.23.45 for RUSTSEC-2026-0285, in which TLS 1.3 handshake
messages were accepted across encryption level boundaries.
