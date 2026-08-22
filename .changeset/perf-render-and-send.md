---
"fiber": patch
---

Obvious performance wins on both sides of the glass. The URL preview no longer round-trips to Rust on every keystroke. ⌘K and collapsed collections stop rebuilding every loaded endpoint in the background. Schema validation and placeholder highlighting skip bodies too large to be worth it. Loader samples are no longer pretty-printed whole just to show the first 20 KB. Streamed chunks are joined rather than concatenated, and a large response is not shipped over IPC a second time after it has already streamed. Collections stay in memory after the first read so a send does not re-parse every saved body; the MCP server does the same across tool calls. History deletes and section deletes no longer run on the UI event-loop thread. A send with static auth no longer clones the request body just in case a 401 retry needed it.
