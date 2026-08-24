---
'fiber': patch
---

Open where you left off.

Collections, requests and responses were already durable. Everything about *looking* at them was not: which endpoint was open, which tab of it, which sidebar tab, which loader folders were expanded, and whatever had been typed into the scratch request. Every launch started on an empty scratch request with the sidebar back on Collections — which is a small thing after a deliberate quit, and a rude one after an auto-update restart nobody asked for.

All of it is now stored, and restored before the first frame rather than a moment after it, so the window opens as it was rather than visibly rearranging itself. A stored endpoint that no longer exists — deleted elsewhere, or dropped by a loader — falls back to the scratch request instead of leaving the pane blank.

An update restart also flushes the debounced section writes now. It never did: Rust holds an exit long enough for the frontend to save, but only for an exit it did not ask for itself, and a restart carries a code that sails straight past that. The last few hundred milliseconds of edits went with the old process. The same flush runs on `pagehide`, which covers a webview torn down and brought back without going through either quit path.

Window size and position needed nothing — `tauri-plugin-window-state` already writes those on exit, and the restart path does run through exit.
