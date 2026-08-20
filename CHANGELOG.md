# fiber

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
