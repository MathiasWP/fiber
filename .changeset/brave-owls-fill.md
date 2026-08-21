---
"fiber": minor
---

A request body built from an OpenAPI schema now shows the name of each type where a value goes — `"offset": number` rather than `"offset": 0` — marked in the editor as a field to fill. Tab and Shift-Tab move between the ones still empty and select them so typing replaces them, and a comma carries you on to the next. The body stays invalid until every one is filled, which is the point.
