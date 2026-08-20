# Running the Fiber MCP server under ToolHive

Fiber's MCP server is normally a local desktop companion: it reads the app's
collections on disk and pulls credentials from the OS keychain. ToolHive runs
MCP servers in **containers**, where neither of those is reachable — so the
headless build takes both from the environment instead:

- **collections** from the directory named by `FIBER_DATA_DIR` (mount it in);
- **secrets** from `FIBER_SECRETS` (a JSON object of `reference → value`) or
  `FIBER_SECRETS_FILE` (a path to the same), which is how ToolHive injects them.

The desktop app is unchanged — it still uses the keychain and its own data dir.
These env vars only take effect when set, which they never are in the app.

## 1. Build the image

```sh
docker build -t fiber-mcp .
# then push wherever ToolHive can pull it, e.g.
# docker tag fiber-mcp ghcr.io/<you>/fiber-mcp:latest && docker push ghcr.io/<you>/fiber-mcp:latest
```

The image is the headless binary only (`cargo build --no-default-features`) — no
Tauri, no webkit.

## 2. Prepare the collections directory

The container reads `${FIBER_DATA_DIR}/sections/*.toml` (default `/data`). Point
it at either the desktop app's own sections directory or a plain folder/repo of
section files. Two things a section needs to be usable over MCP:

- `mcp.enabled = true` in its `[mcp]` table (the app sets this by default now).
  Add `allowWrites = true` to permit anything beyond GET/HEAD/OPTIONS.
- for authenticated sections, a `secretRef` — the app writes `"<sectionId>:auth"`.
  That exact string is the key you provide in `FIBER_SECRETS`.

`/data` should be **writable and persistent**: loader caches, request history and
spilled response bodies are written there, and `query_response` reads a stored
body back, so it needs to survive between tool calls. The image runs as the
distroless nonroot user (uid 65532), so the mount must be writable by it — if the
server exits immediately on start, a read-only or root-owned `/data` is the usual
cause.

## 3. Provide the secrets to ToolHive

Store one secret whose value is the JSON map, then target it at `FIBER_SECRETS`:

```sh
# the value is a JSON object keyed by each section's secretRef
thv secret set fiber-secrets
# paste, e.g.:  {"acme-api:auth":"eyJhbGciOi...","stripe:auth":"sk_live_..."}
```

For a login-request section the value is the request body (`{"user":"…","password":"…"}`);
for bearer/browser sections it's the token or cookie string — exactly what the
app would have put in the keychain.

## 4. Run it

```sh
thv run \
  --name fiber \
  --transport stdio \
  --volume /path/to/your/collections:/data \
  --secret fiber-secrets,target=FIBER_SECRETS \
  ghcr.io/<you>/fiber-mcp:latest
```

ToolHive proxies it to your agents over its own endpoint; point Claude Code (or
any client) at ToolHive as usual. Flag names above match ToolHive's model but
confirm them against `thv run --help` / `thv secret --help` for your version.

If you'd rather not manage a JSON blob, mount a file and set
`--env FIBER_SECRETS_FILE=/run/secrets/fiber.json` instead of the `--secret`
line.

## Security note

Inside a container, injected secrets live in the process environment (or a
mounted file) rather than the OS keychain — that's the unavoidable cost of a
container that can't reach the keychain, and it's the standard container
pattern. ToolHive's encrypted secret store decrypts and injects them at runtime,
which is why `--secret` is preferable to a plain `--env`. The redaction guarantee
still holds: `authorization`, `cookie`, `set-cookie`, `proxy-authorization` and
`x-api-key` are stripped from every response the server returns, so an injected
credential can't be laundered back out through a tool result.

## Note on this image

It's tiny by construction: reqwest already uses rustls (no OpenSSL), so the
binary is built **fully static against musl** and dropped onto
`gcr.io/distroless/static` — CA roots, a nonroot user, no shell, ~2 MB plus the
binary. rustls verifies against the system trust store at runtime, which is why
the base ships `ca-certificates`; a bare `scratch` base would additionally need
that bundle copied in and `SSL_CERT_FILE` pointed at it.

Two things to know:

- The `cargo build --no-default-features` step is verified in this repo, but the
  **musl + aws-lc-rs Docker build hasn't been run here** — treat the first
  `docker build` as the smoke test. aws-lc-rs (rustls' crypto) needs cmake, perl,
  clang and nasm at build time, which the Alpine stage installs. If it gives
  trouble, a `gcr.io/distroless/base-debian12` runtime with a plain
  `rust:1.90-slim` glibc build (no musl, no extra build deps) is the easy
  fallback — larger (~20 MB) but still distroless.
- The Dockerfile targets `x86_64`. For arm64, pass a matching `--target`/base or
  build multi-arch with `docker buildx`.
