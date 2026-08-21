---
"fiber": minor
---

A generated body has a way back. Filling in a loader endpoint's request body is destructive to the placeholders that guided it — once `"offset": number` becomes `"offset": 42`, the tabbable gap is gone. A Reset button next to Format now restores the manifest's generated skeleton, placeholders and all. It sits disabled while the body already matches, and Cmd+Z undoes it.

And clicking quickly through requests no longer builds a backlog that drains one slow response pane at a time. Loading a response body was a synchronous command, and synchronous commands share the event-loop thread — every click queued another read behind the last. The reads now run concurrently off that thread, and a body still in flight for an entry you have already left is dropped instead of parked in memory.
