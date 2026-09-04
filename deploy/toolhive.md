# Running the Fiber MCP server under ToolHive

If Fiber is installed on the machine you are working on, skip this file. The app
*is* the MCP server — one `claude mcp add` and you are done, see the README. A
container adds a layer that cannot reach your keychain, so it is a step down for
local use.

This is for the other case: collections that live in a git repo rather than on
one laptop, on a server, behind ToolHive's proxy and audit log.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/MathiasWP/fiber/main/scripts/toolhive.sh | bash
```

That is the whole thing, credentials included. To serve a collections repo
rather than the desktop app's own collections, name it:

```sh
curl -fsSL .../toolhive.sh | bash -s -- ~/work/api-collections
```

The script finds your collections directory, sets up the credentials for the
collections you have shared, and starts the server. Signing in again in Fiber
reaches the running server on its own — see [Credentials](#credentials) — so
rerunning this is for changing what you serve, not for refreshing a token.

The image is published by the release workflow for `linux/amd64` and
`linux/arm64`, so there is nothing to build and nothing to push. ToolHive pulls
it, starts it on stdio, and wires it into whichever clients you have registered
(`thv client status`).

Everything below is what the script does, for anyone who would rather do it by
hand or is deploying somewhere it cannot run.

## By hand

```sh
thv run --name fiber --transport stdio -v /path/to/your/collections:/data \
  ghcr.io/mathiaswp/fiber-mcp:latest
```

`--transport stdio` is not optional, whatever the flag's description suggests.
Leave it out and ToolHive assumes the image speaks streamable-HTTP: it starts the
container with no stdin, the server reads EOF and exits, and the workload sits
there as `unhealthy` while `docker logs fiber` repeats

```
mcp server stopped: connection closed: initialize request
```

Nothing more is needed unless a collection has auth on it — see below.

## The directory you mount

The container reads `${FIBER_DATA_DIR}/sections/*.toml` (`/data` by default).
Point it at the desktop app's own sections directory or at a plain folder of
section files. Two things a section needs to be usable over MCP:

- `mcp.enabled = true` in its `[mcp]` table. Sharing is off by default; enable
  it explicitly in Section settings. Add `allowWrites = true` to permit
  anything beyond GET/HEAD/OPTIONS.
- for authenticated sections, a `secretRef` — the app writes `"<sectionId>:auth"`.
  That exact string is the key you provide below.

`/data` is also where the credentials file lives when the app is keeping one
current, which is why that half needs no separate mount.

`/data` should be **writable and persistent**: loader caches, request history and
spilled response bodies are written there, and `query_response` reads a stored
body back, so it needs to survive between tool calls. The image runs as the
distroless nonroot user (uid 65532), so the mount must be writable by it. If it
is not, the server exits at startup with

```
mcp server stopped: history body: Permission denied (os error 13)
```

which is the history database failing to open, and means the mount — not the
image.

## Credentials

The desktop app keeps secrets in the OS keychain and the section file holds only
a reference. A container can reach neither, so the headless build takes them
from the environment instead. There are two ways in, and they differ in one
thing that matters a lot in practice: whether signing in again reaches a server
that is already running.

### The file the app keeps current (what the script sets up)

`FIBER_SECRETS_FILE` points at a file of `reference → value`, and the server
re-reads it whenever it needs a credential. Put that file in the directory you
already mount and the desktop app will keep it up to date as you work: sign in
again, and the next tool call picks the new token up. No re-export, no restart.

That mount is the only channel the two halves share — the app cannot write to
ToolHive's secret store, and the container cannot read the keychain — so the
file is encrypted rather than plain, with `FIBER_SECRETS_KEY`. The key stays out
of the mount: in the keychain on the app's side, in ToolHive's encrypted store
on the container's. A copy of the file on its own is inert, and a tampered one
fails to open rather than decrypting to something else.

```sh
/Applications/Fiber.app/Contents/MacOS/fiber mcp file-key | thv secret set fiber-key
/Applications/Fiber.app/Contents/MacOS/fiber mcp export-secrets --to \
  "$HOME/Library/Application Support/dev.fiber.app/mcp-secrets.enc"

thv run --name fiber --transport stdio -v /path/to/your/collections:/data \
  --secret fiber-key,target=FIBER_SECRETS_KEY \
  --env FIBER_SECRETS_FILE=/data/mcp-secrets.enc \
  ghcr.io/mathiaswp/fiber-mcp:latest
```

`file-key` creates the key on first use and returns the same one thereafter, so
rerunning any of this is safe: the key is long-lived and the values rotate
underneath it.

**The file's existence is the opt-in.** The app writes to it only if it is
already there, so a desktop-only user never has credentials on disk, and
deleting the file opts back out.

Both commands refuse to run into a terminal, so the key and the credentials go
down a pipe or into a `0600` file rather than into your scrollback. Reading
secrets back out of the keychain is the one thing nothing else in Fiber does;
macOS may ask you to approve each one.

### The snapshot (`FIBER_SECRETS`)

`FIBER_SECRETS` is a JSON object of `reference → value` in the environment. It
is simpler, and it is what to use when there is no app on the machine to keep a
file current — a collections repo on a server, say.

```sh
/Applications/Fiber.app/Contents/MacOS/fiber mcp export-secrets |
  thv secret set fiber-secrets

thv run --name fiber --transport stdio -v /path/to/your/collections:/data \
  --secret fiber-secrets,target=FIBER_SECRETS \
  ghcr.io/mathiaswp/fiber-mcp:latest
```

A process's environment cannot change under it, so this is a **snapshot taken
when the workload started**. Sign in again and the container will go on
presenting the old credential until you re-export and replace the workload —
rerunning `toolhive.sh` does both. That is the behaviour the file above exists
to avoid.

Worth being precise about what "old credential" covers, because the sharper case
is easy to miss: a collection you authenticated *after* the workload started is
not in the snapshot at all. It fails with no credential rather than a rejected
one, so there is no 401 and nothing to refresh — see
[Migrating from `FIBER_SECRETS`](#migrating-from-fiber_secrets).

If you have no app on the machine, the same map typed by hand does the same job:

```sh
thv secret set fiber-secrets
# paste, e.g.:  {"acme-api:auth":"eyJhbGciOi...","stripe:auth":"sk_live_..."}
```

For a login-request section the value is the request body (`{"user":"…","password":"…"}`);
for bearer/browser sections it's the token or cookie string — exactly what the
app would have put in the keychain.

`FIBER_SECRETS` wins over the file if you somehow set both. An unencrypted
`FIBER_SECRETS_FILE` still works when `FIBER_SECRETS_KEY` is unset, for a file
you manage yourself; setting the key and pointing it at a plaintext file is an
error rather than a silent downgrade, and so is an encrypted file with no key.

### Migrating from `FIBER_SECRETS`

A workload created before the credentials file existed goes on reading its
snapshot, and nothing announces it: the container is healthy, the collections it
knew about still work, and only a collection you authenticated *after* the
workload started fails — with the credential simply absent rather than expired.
The symptom is "it works in the app but not over MCP".

Rerunning the install migrates it, because the script replaces the workload:

```sh
curl -fsSL https://raw.githubusercontent.com/MathiasWP/fiber/main/scripts/toolhive.sh | bash
```

Afterwards the old snapshot is unused, and it is a stale copy of every
credential you had the day it was taken, so take it away:

```sh
thv secret delete fiber-secrets
```

Two things will tell you which scheme a running server is on. It logs its source
at startup —

```
credentials: FIBER_SECRETS, a snapshot frozen at startup
```

— and warns there about any shared collection whose credential it cannot see.
`list_sections` marks the same collections with `"credential": "missing"`, so an
agent finds out before spending a call rather than after.

### Why this is not as good as the keychain

Inside a container, injected secrets live in the process environment or a
mounted file rather than the OS keychain — that's the unavoidable cost of a
container that can't reach the keychain, and it's the standard container
pattern. Encrypting the file narrows the gap: what is at rest in the mount is
ciphertext, and the key is held by ToolHive's encrypted secret store, which is
why `--secret` is preferable to a plain `--env` for it. The redaction guarantee
still holds either way: `authorization`, `cookie`, `set-cookie`,
`proxy-authorization` and `x-api-key` are stripped from every response the
server returns, so an injected credential can't be laundered back out through a
tool result.

## Building the image yourself

You shouldn't need to, but it is one command and it is what the release workflow
runs:

```sh
docker build -t fiber-mcp .                       # this machine's architecture
docker buildx build --platform linux/amd64,linux/arm64 -t fiber-mcp .
```

It's tiny by construction: reqwest already uses rustls (no OpenSSL), so the
binary is built **fully static against musl** and dropped onto
`gcr.io/distroless/static` — CA roots, a nonroot user, no shell, ~2 MB plus the
binary. rustls verifies against the system trust store at runtime, which is why
the base ships `ca-certificates`; a bare `scratch` base would additionally need
that bundle copied in and `SSL_CERT_FILE` pointed at it.

`--no-default-features` drops Tauri, and with it the Linux keychain backend,
which talks to the Secret Service over D-Bus: it wants libdbus and pkg-config at
build time and a session bus at runtime, and a container has none of them. That
backend is part of the `gui` feature for exactly this reason — see
`src-tauri/Cargo.toml`.
