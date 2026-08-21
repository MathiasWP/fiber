---
"fiber": patch
---

A hardening pass across security, reliability, and performance, from a full audit.

Security: the MCP `send_request` tool no longer honors an absolute URL that leaves the section's origin — an agent could previously point `path` at any host and the section's credential went along with it. Custom auth headers (an `X-Api-Key`, say) are now dropped when a redirect leaves the original host, the way reqwest already drops `Authorization`. Inbound credential headers — `Set-Cookie` and friends — are redacted before they reach the history database. The app window has a Content-Security-Policy, the opener capability is scoped to https, and history spill filenames go through the same traversal guard section files always had.

Reliability: a corrupt `history.db` is moved aside and rebuilt instead of panicking on every launch. A collection file that won't parse is now named in the sidebar — it used to vanish silently — and a corrupt file at send time is an error rather than a request quietly sent without auth. Quitting flushes the debounced saves that used to lose the last 400 ms of typing. Saves fsync before the atomic rename. The data-dir migration retries with a copy when the rename fails, and the keychain migration is keyed on a marker so it can't be orphaned. Deleting a history entry or section rolls back in the UI when the disk says no. Requests without an explicit timeout get 60 s instead of forever.

Performance: responses past 1.5 MB skip pretty-printing, JSON parsing, and linting instead of freezing the window. Streaming appends to the editor instead of rewriting the whole document every frame. The loader preview is debounced and no longer ships the manifest across IPC per keystroke. Commands that read or parse files run off the event-loop thread. Typing no longer serializes the whole section per keystroke to ask whether anything changed, and the history tab looks names up in a map instead of scanning every section per row.
