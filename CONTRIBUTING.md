# Contributing

How to run, build and release Fiber. For what the app does, see the
[README](README.md).

## Running

```sh
pnpm install
pnpm app          # tauri dev --no-watch — the app
```

Node 24 (see [`.nvmrc`](.nvmrc); 26 and up also works, odd releases don't).
`.npmrc` sets `engine-strict`, so a wrong version fails the install rather than
something stranger later. CI reads the same file.

`--no-watch` is deliberate: the Tauri dev watcher restarts the app in a loop on
macOS. Frontend changes still hot-reload through Vite; changing anything under
`src-tauri/` means restarting `pnpm app`.

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

There is a longer design doc at `.context/DESIGN.md`, but it is a local working
note — `.context/` is excluded from the repo, so it won't be in your clone.

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

The landing page publishes separately: [`pages.yml`](.github/workflows/pages.yml)
uploads `www/` to GitHub Pages on any push to main that touches it. There is no
build step — the directory goes up as-is.

## Signing

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

### How often the keychain asks

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

### What each platform costs the person downloading

Nothing here needs a paid certificate, and the three platforms differ in what
that costs.

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
