# Fiber

A local-first API client. Tauri + SvelteKit + Bits UI + UnoCSS.

Design doc: [`.context/DESIGN.md`](.context/DESIGN.md).

## Running

```sh
pnpm install
pnpm app          # tauri dev --no-watch — the app
```

Node 24 (see [`.nvmrc`](.nvmrc); 26 and up also works, odd releases don't).
`.npmrc` sets `engine-strict`, so a wrong version fails the install rather than
something stranger later. CI reads the same file.

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
www/
  index.html                      the landing page — one file, no build step
```

## Keys

| | |
|---|---|
| `⌘↵` | send |
| `⌘K` | search endpoints |
| `⌘A` | select the whole response, when focused in it |
| `⌘+` `⌘-` `⌘0` | text size, and back to default — applies to whichever editor you last used, so the response can be smaller than the body |
| right-click | context menu — rename, duplicate, delete, refresh, copy |
| drag | reorder requests and collections, or move a request between them |

Panes are draggable and their sizes persist. Theme follows the system until you
pick one, bottom-left of the sidebar; right-click there to go back to following
it.

## Endpoints

A request doesn't have to live anywhere: the **new request** button in the
sidebar header makes a loose one that belongs to no collection, takes a full
URL, and persists like any other. Collections are for when a shared base URL and
credentials start earning their keep.

Inside a collection, three ways to get endpoints, and they compose:

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

A collection with auth configured carries a small shield in the sidebar: filled
when a credential is stored, amber when none is. It reports only that — whether
one still *works* cannot be known without sending something, since an expired
cookie is present and correct right up until the server disagrees. What it does
catch is auth set up and then never picked, which is easy to do and otherwise
shows up as a puzzling 401.

Credentials live in the OS keychain; the section file holds only a reference, so
it stays safe to share or commit. There is no command to read a secret back out
— the UI can write one and ask whether one exists, nothing more.

A header typed on a request overrides the section's auth — the escape hatch for
"just this once, use a different token". `Cookie` is the exception, and has to
be: cookies are a `;`-joined list rather than a single value, so one typed on a
request joins the section's rather than replacing it. Replacing would drop the
session, and since a secret cannot be read back out of the keychain there would
be no way to type it in alongside.

## MCP

The same binary is the MCP server:

```jsonc
// e.g. Claude Code's mcp config
{ "fiber": { "command": "/path/to/fiber", "args": ["mcp"] } }
```

It reads the same collections on disk and needs no running app. A new collection
is **shared read-only by default** — visible and callable with GET/HEAD/OPTIONS —
with a second switch for anything beyond that, both under *Section settings →
General*. Turn sharing off to hide a collection entirely: an unshared one is
invisible, not merely read-only. Credentials are applied to outgoing requests and
redacted from everything returned.

Two of the tools exist for jq specifically: `loader_manifest` fetches your raw
manifest and `try_loader_filter` tests a candidate filter against it — so you can
ask an agent to write a loader filter for you rather than learning jq first.

### Headless / containers

The server also runs without the desktop app: `cargo build --no-default-features`
drops Tauri and the webview and builds just `fiber mcp`. That build reads
collections from `FIBER_DATA_DIR` and secrets from `FIBER_SECRETS`
(`{"<sectionId>:auth": "<value>"}`) or `FIBER_SECRETS_FILE`, rather than the app
data dir and the OS keychain — which is what lets it run in a container under a
manager like ToolHive. See [`deploy/toolhive.md`](deploy/toolhive.md) and the
[`Dockerfile`](Dockerfile). These env vars are unset in the desktop app, so its
behaviour is unchanged.

## Updates

The app updates itself. It checks GitHub on launch, when the window regains
focus, and every six hours; a toast in the bottom-right offers the new version,
with three ways out:

- **Update** — downloads with a progress bar, swaps the app in, restarts into it.
- **On next launch** — installs the same way but doesn't restart, so the new
  version is simply what opens next time.
- **Not now** — gone for the rest of this run, offered again next launch. Not
  now means not now, rather than never.

[Tauri's updater](https://v2.tauri.app/plugin/updater/) does the work. Each
release carries a `latest.json` listing every platform's bundle, and each bundle
is signed with a key whose public half is in `tauri.conf.json`. The app refuses
anything that doesn't verify against it — which is what makes downloading an
executable and running it a reasonable thing to do. Nothing about that signature
involves Apple: it protects the update channel, not Gatekeeper.

Every failure before the user clicks *Update* is silent — offline,
rate-limited, no release yet — because there is nothing to act on. Failures
after are shown, because a download that dies halfway is worth knowing about.

> [!IMPORTANT]
> The private signing key lives outside this repo, at `~/.tauri/fiber.key`, and
> in the `TAURI_SIGNING_PRIVATE_KEY` repository secret. **Back it up.** Its
> public half is compiled into every copy of Fiber already installed, so losing
> it means no future release can ever be verified by them — every existing
> install stops updating, permanently, and the only fix is for each user to
> download a new build by hand.

## Releasing

[Changesets](https://github.com/changesets/changesets) drives it. Nothing is
released by hand, and no commit message convention is imposed.

```sh
pnpm changeset      # describe your change; commit the file it writes
```

Pick `patch`, `minor` or `major` and write a line for the changelog. That leaves
a markdown file in `.changeset/`, which you commit alongside the work it
describes — usually in the same PR.

From there it is two steps on GitHub, both automatic:

1. **A "Version Packages" PR keeps itself up to date.** On every push to main,
   [`changesets.yml`](.github/workflows/changesets.yml) rolls the pending
   changesets into it: the version moves, `CHANGELOG.md` is written, and the
   changesets it consumed are deleted. Nothing to trigger — it opens the PR if
   there isn't one and updates it if there is.
2. **Merge it.** That fires [`release.yml`](.github/workflows/release.yml), which
   drafts the release, builds on macOS, Windows and Linux, attaches the bundles,
   and only then publishes — so a half-built release is never downloadable.
   Squash or merge commit, either works: the trigger is the version moving, not
   the commit message.

Changesets only knows about `package.json`, so `pnpm version` is
`changeset version` followed by [`scripts/sync-version.mjs`](scripts/sync-version.mjs),
which chases the new version into `tauri.conf.json`, `Cargo.toml` and
`Cargo.lock`. `tauri.conf.json` is the one that ends up stamped on the bundle.
Don't edit any of those four by hand.

Downloads are a universal `.dmg` (Apple silicon and Intel), `.msi` and `.exe`
for Windows, and `.deb`, `.rpm` and `.AppImage` for Linux.

### Signing

On macOS, install with this and there is no prompt at all:

```sh
curl -fsSL https://raw.githubusercontent.com/MathiasWP/fiber/main/scripts/install-macos.sh | bash
```

The dialog everyone hits is not really about the signature. It is about
`com.apple.quarantine`, an extended attribute attached by the *downloading
application* — browsers set it, `curl` does not, and macOS only consults
Gatekeeper about a file that carries it. So an app that arrives via
[`install-macos.sh`](scripts/install-macos.sh) is never questioned. Nothing is
disabled or worked around; the check simply never applies.

Notarising the app is what would make a browser download quiet too, and that
needs a paid Apple Developer account. The rest of this section is about the
browser route.

### How often it asks

Once per app run, for a collection with credentials — not once per request.

The keychain is read the first time a section's credential is needed and kept in
memory from then on, invalidated by a 401 or by the credential being replaced. A
browser session re-captured after a 401 stays in memory only: writing it back
needs authorization exactly as reading does, and a session that expires this
often will be stale again long before the next launch, so the prompt bought
nothing. Setting one up on purpose, through *Pick credential*, still writes.

Zero prompts needs a stable code signature, for the reason below.

### The keychain asks again after every update

Expected, and not fixable for free. A keychain item's ACL — what "Always Allow"
writes — is bound to the *designated requirement* of the process that stored it.
Ad-hoc signing has no certificate to name, so the requirement falls back to the
code hash, and that changes with every build. macOS therefore sees each update
as a different application and asks again.

A self-signed certificate is a tempting fix and isn't one: it would be stable
across builds, but macOS refuses to sign with it until it has been manually
marked trusted in Keychain Access *on every machine that runs the app*. Trading
one prompt for a worse one.

A Developer ID certificate ($99/yr) gives a requirement based on the team
identifier, which doesn't move between releases — so "Always Allow" would mean
always. That, rather than the install prompt, is the strongest argument for
paying for one.

Nothing here needs a paid certificate, and the three platforms differ in what
that costs the person downloading.

**macOS** would be the harsh one. A bundle with no signature at all reports
*"Fiber is damaged and can't be opened"* — which reads like a corrupt download
rather than a security prompt, so most people just delete it. So the bundle is
**ad-hoc signed** instead: `bundle.macOS.signingIdentity` is `"-"`, which
`codesign` accepts without any Apple account.

That isn't notarisation, so Gatekeeper still stops the first launch — but it
stops with *"Apple could not verify "Fiber" is free of malware"*, which is the
recoverable dialog rather than the dead end.

Installing, in the order that matters:

1. **Drag `Fiber.app` out of the disk image into `/Applications` first.** Opening
   it from the mounted `.dmg` cannot work: the quarantine flag has to be cleared,
   and a disk image is a read-only volume.
2. Then either double-click, hit *Done* on the dialog, and go to System Settings
   → Privacy & Security → *Open Anyway* — the button only appears **after** a
   refused open, which is the part everyone misses — or skip the dialog:

```sh
xattr -dr com.apple.quarantine /Applications/Fiber.app
```

A locally built app — `pnpm app:build` — has no quarantine flag to begin with,
because that is applied by the browser on download. It just runs.

**Windows** is survivable: SmartScreen shows *"Windows protected your PC"*, and
*More info → Run anyway* gets past it. Reputation accrues with downloads.

**Linux** needs no signing at all.

To ship signed and notarised macOS builds instead, two things are needed
together: the `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID`
repository secrets, *and* the `env:` block that is commented out above the
`tauri-action` step in [`release.yml`](.github/workflows/release.yml).

The block can't just sit there waiting for the secrets. A secret that doesn't
exist expands to the empty string, which still *defines* the variable, and Tauri
signs whenever `APPLE_CERTIFICATE` is present rather than when it is non-empty —
so an empty value runs `security import` on an empty certificate and fails the
macOS build outright.

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

## License

[MIT](LICENSE).
