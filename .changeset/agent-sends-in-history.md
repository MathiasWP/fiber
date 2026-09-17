---
"fiber": patch
---

Requests an agent sends through the MCP server now show up in History as they
happen, wearing an `MCP` badge, and every entry in the list has a name.

The server has always recorded what it sends — it writes to the same database
as the window, so `query_response` can read a body back — but the window only
read that database at launch. A send made from an agent's window appeared in
History after the next relaunch, and then as a bare URL: the server files its
requests under one synthetic request per collection rather than an endpoint's
id, so the name lookup came up empty. The same was true of anything sent from
scratch or whose request has since been deleted, which made a row without a
name look like a rendering fault rather than what it was.

History now catches up when the window regains focus, when the tab is opened,
and every few seconds while it stays open; the list is merged rather than
replaced, so a body you have loaded or a send still streaming is untouched. An
entry with no request to be named after takes the name of the endpoint its URL
hits — the loader's or a saved request's — and failing that its path, the way
endpoint rows read in Collections. `mcp` matches in the search box, so the badge
is something you can filter by.
