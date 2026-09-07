---
"fiber": patch
---

A collection the API is rejecting no longer re-runs its loader every time you
click back into the window, and editing one no longer reaches for the keychain.

Between them those two made the app unusable after an update. Fiber is signed
ad-hoc, so a new build is a new identity and every keychain item's access list
names the old one: for one launch after every update, each credential read is a
macOS authorization dialog. That is the intended cost, and it should be one
dialog per credential.

It was not, because a failed loader run writes no cache. `loadedAt` never moved,
so the collection stayed stale and the focus trigger re-ran it — and the
authorization dialog is itself a focus event, handing the window back the moment
it is dismissed. Allow, refocus, re-run, prompt again, for as long as the
credential is being refused. A failure now counts as an attempt, so the TTL
means the same thing for both outcomes: don't ask this API again for another
`ttlSeconds`. Refresh and Sign in again are unaffected — those are you asking.

The second one arrived with the credentials file in 0.15.0. Saving a section
reconciled that file, and reconciling it means unsealing it, and unsealing it
means reading the sealing key from the keychain. Fiber saves a section on any
edit, so renaming a request could raise a password prompt. It is now reconciled
only when sharing itself moves — the switch, or which credential the collection
uses — which is what it was always for.
