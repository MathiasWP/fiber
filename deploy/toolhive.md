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

The script finds your collections directory, moves the credentials for the
collections you have shared into ToolHive's secret store, and starts the server.
Rerunning it replaces the workload and refreshes the credentials, which is what
you want after signing in again.

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
from the environment instead: `FIBER_SECRETS` is a JSON object of
`reference → value`, and `FIBER_SECRETS_FILE` is a path to a file holding the
same. Both are unset in the desktop app, which still uses only the keychain.

Building that map by hand is the one genuinely tedious part, so the app will
write it for you:

```sh
/Applications/Fiber.app/Contents/MacOS/fiber mcp export-secrets |
  thv secret set fiber-secrets
```

It emits `{"<secretRef>": "<value>"}` for every collection you have shared over
MCP — and only those, so it hands out nothing an agent could not already use. It
writes to stdout and refuses to run into a terminal, so the credentials go down
the pipe into ToolHive's encrypted store without touching a file, a shell
variable or your scrollback. It is the only thing in Fiber that reads a secret
back out of the keychain; macOS may ask you to approve each one.

If you have no app on the machine, the same map typed by hand does the same job:

```sh
thv secret set fiber-secrets
# paste, e.g.:  {"acme-api:auth":"eyJhbGciOi...","stripe:auth":"sk_live_..."}
```

Either way, one flag on the run command uses it:

```sh
thv run --name fiber --transport stdio -v /path/to/your/collections:/data \
  --secret fiber-secrets,target=FIBER_SECRETS \
  ghcr.io/mathiaswp/fiber-mcp:latest
```

For a login-request section the value is the request body (`{"user":"…","password":"…"}`);
for bearer/browser sections it's the token or cookie string — exactly what the
app would have put in the keychain.

If you'd rather not manage a JSON blob, mount a file and set
`--env FIBER_SECRETS_FILE=/run/secrets/fiber.json` instead of the `--secret`
line.

### Why this is not as good as the keychain

Inside a container, injected secrets live in the process environment (or a
mounted file) rather than the OS keychain — that's the unavoidable cost of a
container that can't reach the keychain, and it's the standard container
pattern. ToolHive's encrypted secret store decrypts and injects them at runtime,
which is why `--secret` is preferable to a plain `--env`. The redaction guarantee
still holds: `authorization`, `cookie`, `set-cookie`, `proxy-authorization` and
`x-api-key` are stripped from every response the server returns, so an injected
credential can't be laundered back out through a tool result.

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
