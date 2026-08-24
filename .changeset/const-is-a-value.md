---
'fiber': patch
---

Read `const` when building a body from a schema.

OpenAPI 3.1 uses JSON Schema 2020-12, where a literal is written as `const` and a literal union as `anyOf: [{const: "once"}, {const: "always"}]`. That is what 3.1 generators emit where 3.0 would have written an `enum`. Fiber read `example`, `default` and `enum`, but not `const` — so it walked into the first branch, found nothing to go on, and printed the `string` placeholder for a field whose only legal values were named right there. Sending that body back got it rejected by the very document it came from.

One real spec this was found against writes 543 `const`s and not a single `enum`, so the existing `enum` handling never fired once across 658 paths. A `const` is now taken as the value it names, in request skeletons and in form fields alike.
