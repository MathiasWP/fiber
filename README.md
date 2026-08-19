# Fiber

A local-first API client. Tauri + SvelteKit + Bits UI + UnoCSS.

Design doc: [`.context/DESIGN.md`](.context/DESIGN.md).

## Running

```sh
pnpm install
pnpm app          # tauri dev --no-watch — the app
```

`--no-watch` is deliberate: the Tauri dev watcher restarts the app in a loop on
macOS (see *Known traps* in the design doc). Frontend changes still hot-reload
through Vite; changing anything under `src-tauri/` means restarting `pnpm app`.

```sh
pnpm check        # svelte-check
pnpm test         # cargo test — HTTP core
pnpm app:build    # bundle the app
```

## Layout

```
src/
  routes/+page.svelte             url bar, body editor, response viewer
  lib/api.ts                      typed wrapper over the Tauri commands
  lib/collections.svelte.ts       sections mirrored from disk, debounced autosave
  lib/history.svelte.ts           history, bucketed per request
  lib/components/LoaderTab.svelte jq filter editor with live preview
  lib/components/Sidebar.svelte   sections, search, history
  lib/components/CommandPalette.svelte   ⌘K search across every endpoint
  lib/components/Editor.svelte    CodeMirror 6 wrapper
src-tauri/
  src/http.rs                     the HTTP core — reqwest, streaming, cancellation
  src/store.rs                    collections as TOML on disk, URL resolution
  src/auth.rs                     token cache, 401 refresh
  src/browser.rs                  browser session capture
  src/loader.rs                   jq-based endpoint discovery
  src/mcp.rs                      the MCP server
  src/lib.rs                      Tauri commands
```

## Keys

| | |
|---|---|
| `⌘↵` | send |
| `⌘K` | search endpoints |
| `⌘A` | select the whole response, when focused in it |
| right-click | context menu — rename, duplicate, delete, refresh, copy |

Panes are draggable and their sizes persist. Theme follows the system until you
pick one, bottom-left of the sidebar; right-click there to go back to following
it.

## Endpoints

Three ways to get them, and they compose — a section can use any mix:

1. **Type them.** A request is a method and a path.
2. **Import an OpenAPI or Swagger file** — *Section settings → General → Import
   OpenAPI*. JSON or YAML, 3.x or 2.0. Operations become ordinary requests, so
   there is nothing to fetch and nothing to go stale: an imported collection
   works offline.
3. **Point a loader at the API's own manifest** — *Section settings → Loader*.
   Best when the API publishes its routes and you want them to stay current,
   but it needs the API reachable.

## Collections

One TOML file per section, in
`~/Library/Application Support/dev.fiber.app/sections/`. Plain text and stably
ordered, so it diffs cleanly if you ever want it in a repo. Edits autosave.

A section owns the base URL; requests inside it hold just a path. There is no
variable system — an absolute URL in a request's path overrides the section,
which is the only escape hatch.

## Auth

Set per section, under *Section settings → Auth*:

- **Bearer token** — a fixed token.
- **Login request** — fetch a token by making a request. Point `tokenPath` at it
  (`$.data.access_token`). For machine-to-machine APIs.
- **Browser session** — sign in in a real browser window, then lift the
  credential out. This is the one for flows a request can't reproduce: emailed
  verification codes, tokens an SDK mints in the page and stores in
  `localStorage`, or a session cookie the server marks HttpOnly.

Any 401 triggers exactly one re-authentication and retry. For a browser session
that means reopening the sign-in page hidden — if your identity provider still
considers you signed in, a fresh credential is captured and you never see a
window.

To set up a browser session: open the sign-in window, log in as you normally
would, then *Pick credential…* — you can close the sign-in window first, the
session is remembered. The app lists everything the session holds, searchable and
ranked with the likeliest first:

- every **cookie** in the session — HttpOnly included, and from every domain, not
  just the API base and login URL
- every **localStorage** entry, flattened to leaf paths, with MSAL v4's encrypted
  entries decrypted in place
- every **IndexedDB** record, addressed as `database/store/key` — this is where
  Firebase's SDK keeps its session by default

Values matching a known provider's documented storage format are labelled and
sorted to the top: Auth0, Supabase, Firebase, MSAL, Cognito, Okta, Clerk,
Keycloak, Auth.js/NextAuth, Better Auth and others (`src/lib/providers.ts`).

Credentials live in the OS keychain; the section file holds only a reference, so
it stays safe to share or commit. There is no command to read a secret back out
— the UI can write one and ask whether one exists, nothing more.

An `Authorization` header typed on a request overrides the section's auth.

## MCP

The same binary is the MCP server:

```jsonc
// e.g. Claude Code's mcp config
{ "fiber": { "command": "/path/to/fiber", "args": ["mcp"] } }
```

It reads the same collections on disk and needs no running app. **Nothing is
exposed until you share it** — per collection, under *Section settings →
General*, with a second switch for anything beyond GET/HEAD/OPTIONS. A collection
you haven't shared is invisible, not merely read-only. Credentials are applied to
outgoing requests and redacted from everything returned.

Two of the tools exist for jq specifically: `loader_manifest` fetches your raw
manifest and `try_loader_filter` tests a candidate filter against it — so you can
ask an agent to write a loader filter for you rather than learning jq first.

## Status

Steps 1–6 of the build order in the design doc: send requests from Rust, write a
JSON body, read the response, organise requests into sections on disk, keep
history that survives restarts, authenticate without babysitting tokens, keep
endpoints in step with the API, and serve the lot over MCP.

History is SQLite in the app data dir, bucketed per request — the newest 50 per
request, nothing older than 30 days. Bodies over 256KB spill to files.

Loaders keep a section's endpoints in step with the API: fetch its route
manifest, map it with a jq filter. Filters are pure, so the editor previews them
against a fetched sample as you type.
