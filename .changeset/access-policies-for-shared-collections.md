---
'fiber': minor
---

Decide what an agent may call per endpoint, not per HTTP method. A shared
collection can carry an access policy — a jq filter answering `"allow"`, `"ask"`
or `"deny"` for each endpoint — which is the only workable guard for an API
where every operation is a POST and the method says nothing about what it does.
The filter reads whatever the manifest publishes: every scalar `x-` extension on
an OpenAPI operation is now carried through the loader, so a rule can be written
against the API's own vocabulary. `"ask"` puts the call in front of you in your
agent's client and sends it only once you approve. Section settings shows the
answers for the collection's real endpoints as you type. Collections without a
policy keep the switch they had, unchanged.
