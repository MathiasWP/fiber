---
"fiber": minor
---

A containerised MCP server now says when it has no credential for a collection,
instead of giving desktop advice.

The credentials file landed in 0.15.0, but only for people who migrated onto it.
A workload created before it goes on reading its frozen `FIBER_SECRETS` snapshot,
and a collection authenticated after that workload started is not stale in the
snapshot — it is absent. There is no 401, so nothing refreshes and nothing
retries; the send fails before it is made, with "not signed in — open Section
settings and sign in". In a container there are no Section settings, no window
to sign in through, and usually a user who *is* signed in.

That failure now explains itself. It names the collection and the reference,
says which source the server reads, and — for a snapshot — that the source was
frozen when the workload started and cannot pick a later sign-in up. The server
also reports its credential source at startup and warns there about every shared
collection whose credential it cannot see, and `list_sections` marks those with
`"credential": "missing"` so an agent finds out before spending a call.

Silently re-captured browser credentials reach the file too. On a 401 the app
lifts a fresh one out of a hidden webview and deliberately does not write it to
the keychain, because writing costs a password prompt. None of that reasoning
applies to the credentials file — the sealing key is already cached and the
write is a file write — but it was being skipped all the same, so a container
only ever saw a browser credential change on an explicit sign-in.

Rerunning `scripts/toolhive.sh` migrates a pre-0.15 workload, and now says so
when it does, including that the old snapshot is left behind holding stale
credentials.
