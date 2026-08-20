---
"fiber": patch
---

Keychain work no longer runs on the UI thread, which is what froze the window mid-send. Query params can be cleared the same way headers can, and neither shows a delete button beside a single empty row.
