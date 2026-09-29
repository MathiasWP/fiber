---
"fiber": patch
---

A collection whose OpenAPI schemas refer to each other in a ring no longer
writes a loader cache of gigabytes, and the MCP server no longer hangs on one.

The cache stored every endpoint's request and response schema with each `$ref`
expanded in place, stopping only where a reference met itself on the current
path. Schemas that refer to each other — an operand that is an object or a
function, each of which holds operands — have a number of such paths that
grows factorially, and every endpoint got its own copy: one real collection's
cache reached 2.5 GB, a single endpoint's schema 252 MB. Every MCP call, down
to `list_sections`, read that file first, so the server answered `initialize`
and then nothing else.

Schemas now live in a file of their own beside the endpoint list, stored as the
document wrote them, with each definition they reach kept once. An endpoint's
schema is put together when it is opened or asked for by `get_endpoint`, with
anything used more than once written a single time under `$defs` — which the
editor's linting already reads — so it can never be larger than the document it
came from. Listing, searching and deciding whether a call is allowed read only
the endpoint list. Request bodies built from such schemas are bounded too,
since a depth limit alone still let them branch into hundreds of kilobytes.

A cache written by an earlier version still lists its endpoints straight away:
reading stops at the end of the endpoint list, before the schemas. Its schemas
are left unread until the next refresh writes both files.
