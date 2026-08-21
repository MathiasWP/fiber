---
"fiber": patch
---

Filling in a generated body behaves. A comma typed to mean "next field" no longer lands next to the one the body already had, leaving `1,,`. A comma inside a string value stays in the string — typing `"Ada, Lovelace"` used to jump away at the comma and type the rest of the name over the next field. Tabbing to a field now puts the caret at the front of it rather than after it, so it looks like something you are about to replace. And a nullable field in an OpenAPI 3.1 document names its type again instead of coming out as `null`: 3.1 writes `"type": ["string", "null"]`, which read as no type at all, so every such field arrived looking already filled in. A choice with a `null` branch — `anyOf: [{"type": "null"}, {"type": "string"}]`, which is how most specs write it — now names the half that says something, whichever side it sits on. Anything else the importer cannot read is now an `unknown` gap you can tab to, rather than a `null` that claims the API wants null.
