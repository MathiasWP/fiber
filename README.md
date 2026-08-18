# Fetch

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
  lib/history.svelte.ts           session history (SQLite in step 3)
  lib/components/Sidebar.svelte   sections, search, history
  lib/components/CommandPalette.svelte   ⌘K search across every endpoint
  lib/components/Editor.svelte    CodeMirror 6 wrapper
src-tauri/
  src/http.rs                     the HTTP core — reqwest, streaming, cancellation
  src/store.rs                    collections as TOML on disk, URL resolution
  src/lib.rs                      Tauri commands
```

## Keys

| | |
|---|---|
| `⌘↵` | send |
| `⌘K` | search endpoints |
| right-click | context menu — on sections, requests, history and responses |
| double-click | rename a section or request |

Panes are draggable and their sizes persist. Theme is system/light/dark, toggled
bottom-left of the sidebar.

## Collections

One TOML file per section, in
`~/Library/Application Support/dev.fetch.app/sections/`. Plain text and stably
ordered, so it diffs cleanly if you ever want it in a repo. Edits autosave.

A section owns the base URL; requests inside it hold just a path. There is no
variable system — an absolute URL in a request's path overrides the section,
which is the only escape hatch.

## Status

Steps 1–3 of the build order in the design doc: send requests from Rust, write a
JSON body, read the response, organise requests into sections that live on disk,
and keep history that survives restarts.

History is SQLite in the app data dir, bucketed per request — the newest 50 per
request, nothing older than 30 days. Bodies over 256KB spill to files.

Not built yet: auth, loaders, MCP server.
