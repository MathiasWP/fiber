---
"fiber": minor
---

Large collections keep scrolling instead of asking you to page them. Opening a header still mounts a first screen of endpoints so that click stays quick; reaching the end of the list loads the next screen on its own.

And a loaded OpenAPI body now says when it does not match the operation's schema — under the editor, and in the lint gutter — without dragging every component schema across the bridge at startup. The schema for the open endpoint is fetched when you select it, and again if you refresh the loader while it is still open.
