---
"fiber": minor
---

Signing in again now reaches a running containerised MCP server.

Under ToolHive the server took its credentials from `FIBER_SECRETS`, read once
at startup, so a container held whatever was true when it began: you signed in,
the keychain got the new token, and the server went on presenting the expired
one until someone re-exported the secrets and replaced the workload.

Credentials can now travel through the collections directory the container
already mounts. The app rewrites that file whenever a credential changes, the
server re-reads it, and the 401 retry that was already there picks the new value
up — no re-export, no restart.

The file is sealed with XChaCha20-Poly1305 and the key stays out of the mount:
in the keychain on the app's side, in ToolHive's encrypted store on the
container's. Its existence is the opt-in, so a desktop-only install never has
credentials on disk. New: `fiber mcp file-key` and `fiber mcp export-secrets
--to <path>`; `scripts/toolhive.sh` wires both up for you.

Bearer collections needed a second fix to benefit: a static token cannot be
refreshed by replaying a request, so a 401 never dropped it, and a zero-TTL
cache entry has nothing else to expire it — a container would have presented
the token it started with for the life of the workload. A rejected credential
is now dropped from the cache whenever it came from a source that can change
underneath the process, so the next call reads the new one. The desktop app is
unaffected: it has no such source, and the same line there would have cost a
keychain prompt per 401.
