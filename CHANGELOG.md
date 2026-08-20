# fiber

## 0.7.0

### Minor Changes

- [#26](https://github.com/MathiasWP/fiber/pull/26) [`6754934`](https://github.com/MathiasWP/fiber/commit/6754934eafb01fdf98766db1d3a9dd01e49264d6) Thanks [@MathiasWP](https://github.com/MathiasWP)! - ⌘+, ⌘- and ⌘0 change the text size, kept separately for the request and response editors so a response can be smaller than what you type. The sidebar's create buttons get proper tooltips and matching icons in their context menu, and the footer drops the ⌘K hint in favour of the version.

### Patch Changes

- [#26](https://github.com/MathiasWP/fiber/pull/26) [`7f4cdd4`](https://github.com/MathiasWP/fiber/commit/7f4cdd457e0b5078e2c7d1958d56c2faf6a6260f) Thanks [@MathiasWP](https://github.com/MathiasWP)! - A Cookie header typed on a request now joins the collection's captured cookie instead of replacing it, so you can send your own cookies alongside a browser session. Auth stays configured in the collection's settings rather than being echoed into the headers table.

## 0.6.2

### Patch Changes

- [#24](https://github.com/MathiasWP/fiber/pull/24) [`b2c4a48`](https://github.com/MathiasWP/fiber/commit/b2c4a48efe59505abe5c595e6a0e1b87b84cd825) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Even out the spacing around the timestamp in the history list, and shrink the update toast's loader.

- [#24](https://github.com/MathiasWP/fiber/pull/24) [`6d5af82`](https://github.com/MathiasWP/fiber/commit/6d5af829bfd1452a15821b4b4203f0b46c9abc64) Thanks [@MathiasWP](https://github.com/MathiasWP)! - A silent re-authentication no longer steals focus with a sign-in window. When the interface does fall over it now says so, with the error, instead of just looking frozen — and devtools are available in release builds.

- [#24](https://github.com/MathiasWP/fiber/pull/24) [`ae8dc4a`](https://github.com/MathiasWP/fiber/commit/ae8dc4af6958f9b90bcedb9d5b3343c831e01094) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Fix the freeze when sending a request. The waiting message picked a new line by reading the one it was about to replace, which re-triggered itself forever and threw, leaving the window painted but dead.

## 0.6.1

### Patch Changes

- [#22](https://github.com/MathiasWP/fiber/pull/22) [`bde6add`](https://github.com/MathiasWP/fiber/commit/bde6add6f71ba214f9474393427b040b0b1cd35d) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Keychain work no longer runs on the UI thread, which is what froze the window mid-send. Query params can be cleared the same way headers can, and neither shows a delete button beside a single empty row.

## 0.6.0

### Minor Changes

- [#18](https://github.com/MathiasWP/fiber/pull/18) [`0434271`](https://github.com/MathiasWP/fiber/commit/04342714268d8ca4bcd934f4e9f02d3001a4fc17) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Query params replace the dead body tab on GET and HEAD, editing the URL directly. Right-clicking the collection list offers New collection and New request, and the webview's own "Reload" menu no longer appears where the app has nothing to offer.

- [#20](https://github.com/MathiasWP/fiber/pull/20) [`d98f6a1`](https://github.com/MathiasWP/fiber/commit/d98f6a1c4fc4305d6fe889599da6d56c458dccbf) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Headers can be removed with an X, and the collection's auth header is shown in the table where it lands. Its value stays write-only — pasting a new token replaces it for the whole collection.

### Patch Changes

- [#19](https://github.com/MathiasWP/fiber/pull/19) [`7d514b8`](https://github.com/MathiasWP/fiber/commit/7d514b86d837041730dc1f5235eb8258648cded4) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Loaders are white everywhere, the waiting line is drawn fresh per request rather than cycling, history rows lead with the name and keep method, status and time together on the right, and the sidebar's create buttons use a matched icon pair.

- [#20](https://github.com/MathiasWP/fiber/pull/20) [`695c0ad`](https://github.com/MathiasWP/fiber/commit/695c0add4929cfded9eb564c14b185c34a7efd1b) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Read a collection's credential from the keychain once per app run instead of once per request, so macOS stops asking for your password on every send.

- [#17](https://github.com/MathiasWP/fiber/pull/17) [`48c61c2`](https://github.com/MathiasWP/fiber/commit/48c61c2bcc6b5d9d903837efda29f4f2d349e557) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Drop the trailing slash from a collection's base URL when you leave the field, so there is one obvious way for it to look rather than two that behave identically.

## 0.5.0

### Minor Changes

- [#15](https://github.com/MathiasWP/fiber/pull/15) [`1067f31`](https://github.com/MathiasWP/fiber/commit/1067f3147952de912089be74cff96e77a6619f47) Thanks [@MathiasWP](https://github.com/MathiasWP)! - The waiting state reads better: a white loader, larger label, and messages that change as a slow request drags on. Updates can now be deferred to the next launch instead of restarting immediately, and the window comes back where you left it — in front — after an update restarts the app.

## 0.4.2

### Patch Changes

- [#13](https://github.com/MathiasWP/fiber/pull/13) [`fa6ffda`](https://github.com/MathiasWP/fiber/commit/fa6ffda384a5d0f7ebde51d4ea438bd7660b8de0) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Sidebar and response-pane polish: a much larger loader while a request is in flight, matched optical heights for the new-request and new-collection icons, a base URL chip that sizes to its own text, request names shown and searchable in the History list, and the redundant response-pane history strip removed.

## 0.4.1

### Patch Changes

- [#11](https://github.com/MathiasWP/fiber/pull/11) [`cc3f0eb`](https://github.com/MathiasWP/fiber/commit/cc3f0ebb7107aa031e26b84960c83482f8a492b6) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Add a macOS install script that sidesteps the Gatekeeper prompt entirely, by fetching the release with curl — which, unlike a browser, attaches no quarantine attribute for macOS to ask about.

## 0.4.0

### Minor Changes

- [#9](https://github.com/MathiasWP/fiber/pull/9) [`d60c4e5`](https://github.com/MathiasWP/fiber/commit/d60c4e55b6acb9675d147ab9c88f14a3230a1877) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Update in place. The toast now downloads the new version with a progress bar, verifies its signature, installs it and restarts — instead of sending you to a browser to find a .dmg.

### Patch Changes

- [#9](https://github.com/MathiasWP/fiber/pull/9) [`d60c4e5`](https://github.com/MathiasWP/fiber/commit/d60c4e55b6acb9675d147ab9c88f14a3230a1877) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Show the running version in the sidebar footer, between the theme toggle and the ⌘K hint.

## 0.3.0

### Minor Changes

- [#6](https://github.com/MathiasWP/fiber/pull/6) [`2506f7d`](https://github.com/MathiasWP/fiber/commit/2506f7db83db16c8dbb1737acc7f15f43b47edc6) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Notice new releases: the app checks GitHub on launch, on focus and every six hours, and offers a toast linking to the release page. It notices rather than installs — Fiber is only ad-hoc signed, and a self-replacing bundle is what Gatekeeper re-examines on next launch.

### Patch Changes

- [#6](https://github.com/MathiasWP/fiber/pull/6) [`2506f7d`](https://github.com/MathiasWP/fiber/commit/2506f7db83db16c8dbb1737acc7f15f43b47edc6) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Ad-hoc sign the macOS bundle, so first launch shows the ordinary "unidentified developer" dialog with its Open Anyway button rather than "damaged and can't be opened".

- [#8](https://github.com/MathiasWP/fiber/pull/8) [`4f1272c`](https://github.com/MathiasWP/fiber/commit/4f1272ca8b526d3374578670c2ed1d9ebea0ec73) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Name an unnamed request after the endpoint you type into it, until you name it yourself.

- [#8](https://github.com/MathiasWP/fiber/pull/8) [`041c98b`](https://github.com/MathiasWP/fiber/commit/041c98b8e54e8856ba51643aa8c73b0ecdabadaf) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Replace the spinning-circle loading indicators with a glowing dot-matrix loader.

- [#8](https://github.com/MathiasWP/fiber/pull/8) [`98036f2`](https://github.com/MathiasWP/fiber/commit/98036f2b80d71e4c7eeca238acac20ced0997d50) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Stop the History tab changing which response a request is showing. Opening an entry in History is a look, not a choice — leaving History puts every request back on the response it had.

- [#8](https://github.com/MathiasWP/fiber/pull/8) [`041c98b`](https://github.com/MathiasWP/fiber/commit/041c98b8e54e8856ba51643aa8c73b0ecdabadaf) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Start with an empty URL bar rather than a leftover httpbin.org address.

## 0.2.0

### Minor Changes

- [#1](https://github.com/MathiasWP/fiber/pull/1) [`9b96af2`](https://github.com/MathiasWP/fiber/commit/9b96af27d131e7ac3a58ae25c6696284a6518598) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Release pipeline: changesets keeps a Version Packages PR open, and merging it builds Fiber for macOS, Windows and Linux and publishes them as a GitHub release.
