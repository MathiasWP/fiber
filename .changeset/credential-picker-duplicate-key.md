---
"fiber": patch
---

Fix the credential picker crashing with `each_key_duplicate`. Rows were keyed on the capture rule — source, key and path — which isn't unique: a session holding the same cookie name on two domains (`sid` on `.example.com` and on `api.example.com`) produced two rows with the same key, and Svelte threw instead of rendering the list. Each row now carries its own id.
