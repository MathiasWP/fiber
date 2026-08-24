---
'fiber': patch
---

Validate request bodies with Ajv, and read credential paths with real JSONPath.

Body linting was a hand-written walker over "a deliberately useful subset" of JSON Schema, and the subset was the problem: no `$ref`, no `minimum`, `pattern`, `uniqueItems`, `minLength`, `patternProperties`, `if`/`then` or `dependentSchemas`. All of those passed silently, so a body could be reported clean and still be rejected by the API that published the schema. Ajv is the reference implementation of what that walker was approximating.

What stays hand-written is the part that isn't JSON Schema: OpenAPI 3.0's `nullable`, folded into a union type, and `type` values that don't exist. One real 3.1 document arrives with `"type": "undefined"` 310 times alongside `emoji`, `icon`, `void` and `http` — Ajv rejects those at compile time, which would cost that schema all of its linting rather than one field's, so the invented names are dropped and every valid constraint beside them keeps working. Messages are unchanged, including the "must be number, not string" phrasing Ajv leaves out.

Credential paths now go through `serde_json_path`, so a capture rule can use `$..id_token` to find a token whose nesting depth you don't know, or `$.keys[?(@.active == true)].secret` to pick the entry that is current rather than pinning an index that moves. The dotted form every saved rule already uses keeps working: it isn't valid JSONPath — `$.data.tokens.0.value` needs `[0]` under RFC 9535 — so the query is tried first and the original walk answers for anything it rejects. A query matching several nodes reports nothing rather than picking arbitrarily.
