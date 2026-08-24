---
'fiber': patch
---

Select an endpoint in the collection you clicked it in.

Two collections describing the same API — staging and production — give every loaded endpoint the same id, because a loaded id is `METHOD /path` and deliberately carries no section: that is the identity a saved body and a refresh have to agree on, so a re-run re-attaches instead of orphaning.

Selection was keyed on that id alone. So both rows highlighted at once, the pane always resolved to whichever collection sorted first, and the second one could not be opened at all — clicking it set an id the store already held, so nothing changed. The selection now carries the section as well.

Note that response history is still bucketed by request id, so the same endpoint in two collections shares one history. That is the same root cause and is not fixed here.
