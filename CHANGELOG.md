# fiber

## 0.14.8

### Patch Changes

- [#87](https://github.com/MathiasWP/fiber/pull/87) [`a703582`](https://github.com/MathiasWP/fiber/commit/a7035828c254598b31d5d0022fc36a4b3d2f5afc) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Keep each collection's response history to itself.
  
  Response history was bucketed by request id alone. A loaded endpoint's id is `METHOD /path` and carries no section, so two collections describing the same API — staging and production — shared one list: opening either showed whichever had been sent last, and clearing one deleted both.
  
  The database has stored `section_id` since the column was added; it was simply never handed back. It is now, so the window can tell the two apart, and clearing is scoped to the collection you cleared.
  
  Entries recorded before this still show for either collection rather than disappearing, since nothing knows which one they came from. A scoped clear takes them too — they are the same request's older entries, and leaving them behind would look like the clear half-worked.

- [#85](https://github.com/MathiasWP/fiber/pull/85) [`ba28ebb`](https://github.com/MathiasWP/fiber/commit/ba28ebbc3e43dd1059b53f97c3e5c0727b2657fd) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Select an endpoint in the collection you clicked it in.
  
  Two collections describing the same API — staging and production — give every loaded endpoint the same id, because a loaded id is `METHOD /path` and deliberately carries no section: that is the identity a saved body and a refresh have to agree on, so a re-run re-attaches instead of orphaning.
  
  Selection was keyed on that id alone. So both rows highlighted at once, the pane always resolved to whichever collection sorted first, and the second one could not be opened at all — clicking it set an id the store already held, so nothing changed. The selection now carries the section as well.
  
  Note that response history is still bucketed by request id, so the same endpoint in two collections shares one history. That is the same root cause and is not fixed here.

## 0.14.7

### Patch Changes

- [#83](https://github.com/MathiasWP/fiber/pull/83) [`5f8fde8`](https://github.com/MathiasWP/fiber/commit/5f8fde84220b5168a9a41ffe4884b9da7e8ba00d) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Validate request bodies with Ajv, and read credential paths with real JSONPath.
  
  Body linting was a hand-written walker over "a deliberately useful subset" of JSON Schema, and the subset was the problem: no `$ref`, no `minimum`, `pattern`, `uniqueItems`, `minLength`, `patternProperties`, `if`/`then` or `dependentSchemas`. All of those passed silently, so a body could be reported clean and still be rejected by the API that published the schema. Ajv is the reference implementation of what that walker was approximating.
  
  What stays hand-written is the part that isn't JSON Schema: OpenAPI 3.0's `nullable`, folded into a union type, and `type` values that don't exist. One real 3.1 document arrives with `"type": "undefined"` 310 times alongside `emoji`, `icon`, `void` and `http` — Ajv rejects those at compile time, which would cost that schema all of its linting rather than one field's, so the invented names are dropped and every valid constraint beside them keeps working. Messages are unchanged, including the "must be number, not string" phrasing Ajv leaves out.
  
  Credential paths now go through `serde_json_path`, so a capture rule can use `$..id_token` to find a token whose nesting depth you don't know, or `$.keys[?(@.active == true)].secret` to pick the entry that is current rather than pinning an index that moves. The dotted form every saved rule already uses keeps working: it isn't valid JSONPath — `$.data.tokens.0.value` needs `[0]` under RFC 9535 — so the query is tried first and the original walk answers for anything it rejects. A query matching several nodes reports nothing rather than picking arbitrarily.

## 0.14.6

### Patch Changes

- [#81](https://github.com/MathiasWP/fiber/pull/81) [`a7def54`](https://github.com/MathiasWP/fiber/commit/a7def5409481d34d62178de1de3f50303ae0fd81) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Read `const` when building a body from a schema.
  
  OpenAPI 3.1 uses JSON Schema 2020-12, where a literal is written as `const` and a literal union as `anyOf: [{const: "once"}, {const: "always"}]`. That is what 3.1 generators emit where 3.0 would have written an `enum`. Fiber read `example`, `default` and `enum`, but not `const` — so it walked into the first branch, found nothing to go on, and printed the `string` placeholder for a field whose only legal values were named right there. Sending that body back got it rejected by the very document it came from.
  
  One real spec this was found against writes 543 `const`s and not a single `enum`, so the existing `enum` handling never fired once across 658 paths. A `const` is now taken as the value it names, in request skeletons and in form fields alike.

## 0.14.5

### Patch Changes

- [#79](https://github.com/MathiasWP/fiber/pull/79) [`5ee2e66`](https://github.com/MathiasWP/fiber/commit/5ee2e6626d849ef0289293b4c8895588723d78f9) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Stop "Pick credential…" timing out when no sign-in window is open yet.
  
  Reading `localStorage` means evaluating script in the page, which fails until the page has loaded. That failure was propagated, discarding the cookies along with it — even though cookies are read from the Rust side, need no script, and are there immediately. So opening the picker straight from a closed window reported "timed out reading the sign-in window" while the session cookie sat in hand; opening the window first, then picking, worked.
  
  Cookies are now kept when the page can't be read, and the timeout is only reported when there is genuinely nothing to show. The retry loop still waits for a complete read before settling, so a credential kept in `localStorage` isn't missed by returning the cookies-only snapshot the instant the window opens.

- [#79](https://github.com/MathiasWP/fiber/pull/79) [`5ee2e66`](https://github.com/MathiasWP/fiber/commit/5ee2e6626d849ef0289293b4c8895588723d78f9) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Stop two loader requests cancelling each other, and say when a rejection came from somewhere else.
  
  Every loader request for a section used the same id, and `HttpState` keys in-flight requests by that id — inserting a second under a key already there drops the first's cancel sender, which *is* the cancel signal. So a "Fetch a sample" while a background refresh was out came back "request cancelled", and which of the two died depended on timing. Loader requests now get a unique handle each; nothing cancels them by id, so there was never a reason for it to be predictable.
  
  A rejected manifest now also reports where the response actually came from, when that left the origin the request was aimed at. A Cookie or Authorization credential is dropped on a cross-host redirect, so "403" and "403, having ended up on a different host" are different problems wearing the same status — and only one of them is your API's fault.
  
  The sidebar's loader error is selectable and has a Copy button, because the first thing anyone does with an error they can't act on is send it to someone who can.

## 0.14.4

### Patch Changes

- [#77](https://github.com/MathiasWP/fiber/pull/77) [`1c149c5`](https://github.com/MathiasWP/fiber/commit/1c149c5db4dc978b2acfa72b9c4404c42e6f259e) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Never capture a cleared cookie, and make "Sign in again" actually sign you in.
  
  Signing out clears a session cookie by setting it to the empty string, and some sign-in flows do it on the way through — often on the identity provider's host while the live cookie lands on another. The capture rule matched by name alone and took whichever came first, so it could store the blank one. That went out as `Cookie: sid=` and came back as the API's own version of "token is empty": a rejection that reads like a server problem and is really an empty header. A cookie now only counts if it has a value, an empty capture reports nothing rather than storing it, and the credential picker no longer offers blanks.
  
  "Sign in again" opened Section settings on the General tab and stopped there, leaving you to find Auth and press Open sign-in yourself. It now opens the Auth tab and starts the sign-in.

## 0.14.3

### Patch Changes

- [#75](https://github.com/MathiasWP/fiber/pull/75) [`cb46eb7`](https://github.com/MathiasWP/fiber/commit/cb46eb77d32150e40a4a956381b3f07181e4cda2) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Stop a background loader refresh from firing at a section you are signing into, and make the failure it left behind actionable.
  
  Opening the sign-in window moves focus off the main window, and focus returning is one of the two triggers for a stale-loader refresh — so the run went out with the credential you were in the middle of replacing, failed, and posted an unattributed "the manifest request returned 403" in the sidebar at the exact moment you opened the window. Signing in never cleared it, because nothing re-ran the loader afterwards.
  
  A section is now skipped while its sign-in window is open, the loader re-runs once a credential is captured, and the error names its section and offers a sign-in button when the API turned the credential down. Rejected manifest requests also carry a snippet of the response body, so a 403 can say which 403 it was — and a 401 or 403 in the response pane offers the same button rather than leaving you to find the drawer.

## 0.14.2

### Patch Changes

- [#72](https://github.com/MathiasWP/fiber/pull/72) [`276d629`](https://github.com/MathiasWP/fiber/commit/276d6297177d43d9e636ba9c24adaf7b6e17bcdd) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Fix the credential picker crashing with `each_key_duplicate`. Rows were keyed on the capture rule — source, key and path — which isn't unique: a session holding the same cookie name on two domains (`sid` on `.example.com` and on `api.example.com`) produced two rows with the same key, and Svelte threw instead of rendering the list. Each row now carries its own id.

- [#72](https://github.com/MathiasWP/fiber/pull/72) [`276d629`](https://github.com/MathiasWP/fiber/commit/276d6297177d43d9e636ba9c24adaf7b6e17bcdd) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Make the crash banner and the update toast usable while a dialog is open. An open modal dialog sets `pointer-events: none` on `<body>`, which both of them inherited: a click on Copy, Hide or Update passed straight through and landed on the dialog, which read it as an outside click and closed itself. With dialogs stacked, the banner stayed out of reach until every one of them had been dismissed. Both now take pointer events of their own and keep the click from reaching the dialog underneath.

## 0.14.1

### Patch Changes

- [#70](https://github.com/MathiasWP/fiber/pull/70) [`a793695`](https://github.com/MathiasWP/fiber/commit/a7936959decd976db06518f22f62972814e4e458) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Structure the MCP tab and leave out the clients that aren't there. The tab now lists only the clients it can find on this machine, with the rest one line away — detection is a guess at a directory, not a fact, and a client that already holds an entry is always shown. That check was wrong for Claude Code, whose `~/.claude.json` sits directly in the home directory and so counted as present everywhere; it asks about `~/.claude` now. The tab is in three labelled parts, the copy buttons are bordered controls beside a caption rather than bare text that only appeared on hover, and the snippets wrap instead of scrolling sideways.

## 0.14.0

### Minor Changes

- [#68](https://github.com/MathiasWP/fiber/pull/68) [`098da51`](https://github.com/MathiasWP/fiber/commit/098da51344090e39c3f0cc6f33489ed4f2c34898) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Add an MCP tab beside Collections and History that installs Fiber into an AI client for you. It lists Claude Code, Claude Desktop, Cursor, VS Code, Windsurf, Codex CLI and Gemini CLI with the config file each one uses, and Add writes the entry pointing at wherever this copy of the app actually lives. An entry left behind by a copy that has moved shows as Update. The edit only ever adds or removes Fiber's own key: other servers and settings survive, Codex's hand-written TOML keeps its comments and key order, and a config file that doesn't parse is left untouched with the snippet offered to paste instead. Below the list, the ToolHive route is offered as a copyable command with a link to its guide, for collections served from a repo rather than this machine.

### Patch Changes

- [#67](https://github.com/MathiasWP/fiber/pull/67) [`3c60cbd`](https://github.com/MathiasWP/fiber/commit/3c60cbdcc60243055e015f029e29c43a3354cfd6) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Loader folders now start collapsed. A spec with hundreds of endpoints opens to
  its list of tags rather than a wall of paths, and each folder header carries the
  full count of what it holds. Endpoints are paged in per open folder, so opening
  one mounts its rows and a closed one costs nothing.

## 0.13.0

### Minor Changes

- [#59](https://github.com/MathiasWP/fiber/pull/59) [`68ab80c`](https://github.com/MathiasWP/fiber/commit/68ab80c6f03dc9265ceb5971ffc18b6f26be1685) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Richer OpenAPI (tags as folders, path and query parameters, operation descriptions, response-schema checks), per-collection HTTP identity (cookie jar, timeout, redirects, proxy, invalid certs), and non-JSON bodies (form, multipart, file). Query parameters are available on every method.

### Patch Changes

- [#66](https://github.com/MathiasWP/fiber/pull/66) [`d24f8d8`](https://github.com/MathiasWP/fiber/commit/d24f8d864bcc3590bf5b9a3c20cded4c46b1745e) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Harden MCP credential boundaries, concurrent request handling, response storage, endpoint discovery, and protocol validation.

- [#60](https://github.com/MathiasWP/fiber/pull/60) [`33fdf99`](https://github.com/MathiasWP/fiber/commit/33fdf994d383cf8a7072ebbecea30706e2f911ab) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Creating a collection now shows its first request immediately. The new section was mutated as a plain object after `$state` had already proxied it, so the sidebar never saw the push.

- [#60](https://github.com/MathiasWP/fiber/pull/60) [`33fdf99`](https://github.com/MathiasWP/fiber/commit/33fdf994d383cf8a7072ebbecea30706e2f911ab) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Importing an OpenAPI spec now reports how many endpoints were actually added. The count used to re-read a live list after those endpoints had already been pushed, so it always said zero.

- [#63](https://github.com/MathiasWP/fiber/pull/63) [`bfbfcd0`](https://github.com/MathiasWP/fiber/commit/bfbfcd0d9a9561d7d61902f1d39f27e0a2977a3f) Thanks [@MathiasWP](https://github.com/MathiasWP)! - A few performance fixes: computing what a loader refresh added or removed was quadratic in the number of endpoints, sending a request whose body comes from a file blocked the async runtime instead of reading it off-thread, and the sidebar's "Move to" submenu recomputed its target list twice per request row.

- [#64](https://github.com/MathiasWP/fiber/pull/64) [`749d148`](https://github.com/MathiasWP/fiber/commit/749d148d542d67a7364ac9359a502d36874f01e3) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Obvious performance wins on both sides of the glass. The URL preview no longer round-trips to Rust on every keystroke. ⌘K and collapsed collections stop rebuilding every loaded endpoint in the background. Schema validation and placeholder highlighting skip bodies too large to be worth it. Loader samples are no longer pretty-printed whole just to show the first 20 KB. Streamed chunks are joined rather than concatenated, and a large response is not shipped over IPC a second time after it has already streamed. Collections stay in memory after the first read so a send does not re-parse every saved body; the MCP server does the same across tool calls. History deletes and section deletes no longer run on the UI event-loop thread. A send with static auth no longer clones the request body just in case a 401 retry needed it.

- [#56](https://github.com/MathiasWP/fiber/pull/56) [`94f8b39`](https://github.com/MathiasWP/fiber/commit/94f8b39c7801f5dcd9f4ee61a990e02aaf707b10) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Svelte 5 best practices: `$state.raw` for wholesale-replaced data (loader caches, OpenAPI samples, browser snapshots), `{@attach}` in place of actions and the CodeMirror mount effect, and window listeners on `<svelte:window>` rather than inside `$effect`.

## 0.12.0

### Minor Changes

- [#54](https://github.com/MathiasWP/fiber/pull/54) [`1af9a51`](https://github.com/MathiasWP/fiber/commit/1af9a51c2836324bc91048faa13c3cbcad468abd) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Large collections keep scrolling instead of asking you to page them. Opening a header still mounts a first screen of endpoints so that click stays quick; reaching the end of the list loads the next screen on its own.
  
  And a loaded OpenAPI body now says when it does not match the operation's schema — under the editor, and in the lint gutter — without dragging every component schema across the bridge at startup. The schema for the open endpoint is fetched when you select it, and again if you refresh the loader while it is still open.

## 0.11.0

### Minor Changes

- [#53](https://github.com/MathiasWP/fiber/pull/53) [`5da1e11`](https://github.com/MathiasWP/fiber/commit/5da1e11479125888553f3fe630ffa31cd40fb955) Thanks [@MathiasWP](https://github.com/MathiasWP)! - A generated body has a way back. Filling in a loader endpoint's request body is destructive to the placeholders that guided it — once `"offset": number` becomes `"offset": 42`, the tabbable gap is gone. A Reset button next to Format now restores the manifest's generated skeleton, placeholders and all. It sits disabled while the body already matches, and Cmd+Z undoes it.
  
  And clicking quickly through requests no longer builds a backlog that drains one slow response pane at a time. Loading a response body was a synchronous command, and synchronous commands share the event-loop thread — every click queued another read behind the last. The reads now run concurrently off that thread, and a body still in flight for an entry you have already left is dropped instead of parked in memory.

### Patch Changes

- [#53](https://github.com/MathiasWP/fiber/pull/53) [`a088263`](https://github.com/MathiasWP/fiber/commit/a0882637e4857483560c103a8cf48c01a398b86c) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Hover states that never were. The section cog, the two add-buttons in the sidebar header, header/param delete buttons, and a handful of others were written with UnoCSS variant-group syntax — `hover:(bg-border text-text)` — which the PostCSS pipeline never expands: it scans class names but does not rewrite source, so the browser received split-by-space junk and no rule matched. Every one is now written out in full, and the transformer that was quietly doing nothing is gone from the config, with a comment explaining the trap.

- [#53](https://github.com/MathiasWP/fiber/pull/53) [`5da1e11`](https://github.com/MathiasWP/fiber/commit/5da1e11479125888553f3fe630ffa31cd40fb955) Thanks [@MathiasWP](https://github.com/MathiasWP)! - A hardening pass across security, reliability, and performance, from a full audit.
  
  Security: the MCP `send_request` tool no longer honors an absolute URL that leaves the section's origin — an agent could previously point `path` at any host and the section's credential went along with it. Custom auth headers (an `X-Api-Key`, say) are now dropped when a redirect leaves the original host, the way reqwest already drops `Authorization`. Inbound credential headers — `Set-Cookie` and friends — are redacted before they reach the history database. The app window has a Content-Security-Policy, the opener capability is scoped to https, and history spill filenames go through the same traversal guard section files always had.
  
  Reliability: a corrupt `history.db` is moved aside and rebuilt instead of panicking on every launch. A collection file that won't parse is now named in the sidebar — it used to vanish silently — and a corrupt file at send time is an error rather than a request quietly sent without auth. Quitting flushes the debounced saves that used to lose the last 400 ms of typing. Saves fsync before the atomic rename. The data-dir migration retries with a copy when the rename fails, and the keychain migration is keyed on a marker so it can't be orphaned. Deleting a history entry or section rolls back in the UI when the disk says no. Requests without an explicit timeout get 60 s instead of forever.
  
  Performance: responses past 1.5 MB skip pretty-printing, JSON parsing, and linting instead of freezing the window. Streaming appends to the editor instead of rewriting the whole document every frame. The loader preview is debounced and no longer ships the manifest across IPC per keystroke. Commands that read or parse files run off the event-loop thread. Typing no longer serializes the whole section per keystroke to ask whether anything changed, and the history tab looks names up in a map instead of scanning every section per row.

- [#51](https://github.com/MathiasWP/fiber/pull/51) [`748226e`](https://github.com/MathiasWP/fiber/commit/748226e1fa6fb75c522db0b80b1e7e72b95a8270) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Filling in a generated body behaves. A comma typed to mean "next field" no longer lands next to the one the body already had, leaving `1,,`. A comma inside a string value stays in the string — typing `"Ada, Lovelace"` used to jump away at the comma and type the rest of the name over the next field. Tabbing to a field now puts the caret at the front of it rather than after it, so it looks like something you are about to replace. And a nullable field in an OpenAPI 3.1 document names its type again instead of coming out as `null`: 3.1 writes `"type": ["string", "null"]`, which read as no type at all, so every such field arrived looking already filled in. A choice with a `null` branch — `anyOf: [{"type": "null"}, {"type": "string"}]`, which is how most specs write it — now names the half that says something, whichever side it sits on. Anything else the importer cannot read is now an `unknown` gap you can tab to, rather than a `null` that claims the API wants null.

## 0.10.1

### Patch Changes

- [#46](https://github.com/MathiasWP/fiber/pull/46) [`3a08ea4`](https://github.com/MathiasWP/fiber/commit/3a08ea42410dadc6cdef50ac81494dfe5676205c) Thanks [@MathiasWP](https://github.com/MathiasWP)! - A collection can be closed while you are searching. A search still opens everything, since a match you cannot see is no use, but closing one now keeps it closed for the rest of that search — and a fresh search opens them again. Closing one this way is about the search rather than the collection, so it is not saved.

- [#47](https://github.com/MathiasWP/fiber/pull/47) [`87a6881`](https://github.com/MathiasWP/fiber/commit/87a68813ba0a0ed08f521f231390c2f98f01258c) Thanks [@MathiasWP](https://github.com/MathiasWP)! - An endpoint you had opened before its manifest carried a request body now picks that body up. Previously the empty one saved against it won, so the schema's body never appeared on exactly the endpoints you had used most. A body you have written is still left alone.

- [#48](https://github.com/MathiasWP/fiber/pull/48) [`2c64519`](https://github.com/MathiasWP/fiber/commit/2c64519df447210f76d6af4d4b341176b56d0071) Thanks [@MathiasWP](https://github.com/MathiasWP)! - The MCP server is easier to reach. The README now names the binary's real path on each OS instead of `/path/to/fiber`, so installing it is one `claude mcp add`. For the containerised server, `scripts/toolhive.sh` sets ToolHive up in a single command, and a new `fiber mcp export-secrets` pipes the credentials for your shared collections into ToolHive's secret store rather than having you copy each one out by hand. The container image itself now builds — it had not, since the Linux keychain backend needs a D-Bus library no container has — and is published for amd64 and arm64 on every release.

- [#49](https://github.com/MathiasWP/fiber/pull/49) [`c4c0638`](https://github.com/MathiasWP/fiber/commit/c4c0638d5ed1f18f16a235fd944bba7253feabf2) Thanks [@MathiasWP](https://github.com/MathiasWP)! - The shield, cog, refresh spinner and endpoint count on a collection now carry proper tooltips saying what they are, and each has an accessible name rather than being an unlabelled icon. The cog and the two buttons in the sidebar header light up more clearly on hover, so it is obvious they can be clicked.

## 0.10.0

### Minor Changes

- [#43](https://github.com/MathiasWP/fiber/pull/43) [`337f6e1`](https://github.com/MathiasWP/fiber/commit/337f6e16964de41d5f58533691d3c88f32e4322d) Thanks [@MathiasWP](https://github.com/MathiasWP)! - A request body built from an OpenAPI schema now shows the name of each type where a value goes — `"offset": number` rather than `"offset": 0` — marked in the editor as a field to fill. Tab and Shift-Tab move between the ones still empty and select them so typing replaces them, and a comma carries you on to the next. The body stays invalid until every one is filled, which is the point.

- [#42](https://github.com/MathiasWP/fiber/pull/42) [`89fd771`](https://github.com/MathiasWP/fiber/commit/89fd77132a4d7c025cd2325533644b0c51a036ca) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Endpoints refresh themselves when you come back to the window, not only at startup — so a loader left open all day no longer shows yesterday's routes. A collection spins a small icon while it is refreshing, so an automatic refresh is something you can see rather than endpoints changing on their own. New loaders default to a five minute TTL; 0 still means "only when asked".

### Patch Changes

- [#44](https://github.com/MathiasWP/fiber/pull/44) [`0e0bb99`](https://github.com/MathiasWP/fiber/commit/0e0bb99be025031330c4c042553a7aec75b559f6) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Searching the endpoints no longer buries what you meant. A term that appears whole in a path now ranks above one whose letters merely appear in order, and when anything matches properly the near-misses are dropped rather than listed alongside — so `/list` stops returning every path containing those five letters somewhere. A typo, which has no proper match to lose to, still guesses as it did before.

- [#42](https://github.com/MathiasWP/fiber/pull/42) [`d01691d`](https://github.com/MathiasWP/fiber/commit/d01691d74211f1e2a3666260f1110bc0205d2cb3) Thanks [@MathiasWP](https://github.com/MathiasWP)! - The sidebar does much less work per render with a large collection: the loaded-endpoint rows were rebuilt five times over on every update, and matching them against your saved bodies was quadratic. Opening several endpoints in a row now writes the collection once rather than once each.

## 0.9.1

### Patch Changes

- [#40](https://github.com/MathiasWP/fiber/pull/40) [`dbd22ff`](https://github.com/MathiasWP/fiber/commit/dbd22ffa31848a52b30b28b0a738f6c2496ac9e4) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Searching the collections now says how many endpoints the filter is hiding, with a button to clear it. The count sits under the results, and pins to the bottom of the pane once they outgrow it.

- [#39](https://github.com/MathiasWP/fiber/pull/39) [`91138a7`](https://github.com/MathiasWP/fiber/commit/91138a755345d787189754c0f5db9f9e256eb598) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Loaders pointed at an OpenAPI document now fill in request bodies from each operation's schema, the way importing the same file already did. The templates dropdown names the template in use and ticks it in the list, and Done runs the loader rather than leaving you to find Refresh.

## 0.9.0

### Minor Changes

- [#36](https://github.com/MathiasWP/fiber/pull/36) [`bd1459c`](https://github.com/MathiasWP/fiber/commit/bd1459c3425f0e88a9fa49552d446e3af073489e) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Response bodies stream into the pane as they arrive, so a slow endpoint shows its answer while it is still being written and an SSE stream shows anything at all. Importing an OpenAPI spec now fills in a request body from the operation's schema. Collections have a settings cog in the sidebar. Loader templates lead with OpenAPI, and a new loader starts on it. Option-arrow in a URL field jumps between dots and slashes, the way a browser's address bar does.

## 0.8.1

### Patch Changes

- [#34](https://github.com/MathiasWP/fiber/pull/34) [`cee1a34`](https://github.com/MathiasWP/fiber/commit/cee1a349de9a8382b46cf9f32a946593fb31f082) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Dialogs open in front of the section settings drawer instead of behind it — picking a credential, searching endpoints, and the new-collection and delete prompts are all usable while the drawer is open.

## 0.8.0

### Minor Changes

- [#30](https://github.com/MathiasWP/fiber/pull/30) [`4bd0066`](https://github.com/MathiasWP/fiber/commit/4bd0066cb000d8cac41b20e5f658aa6602490660) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Section settings is a drawer sliding out from the sidebar's edge rather than a centred dialog that resized on every tab, and CodeMirror's find panel is styled to match the app instead of showing raw browser controls.

### Patch Changes

- [#30](https://github.com/MathiasWP/fiber/pull/30) [`3bb398e`](https://github.com/MathiasWP/fiber/commit/3bb398ea919ba18f9589257878cfa10f6ffc202a) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Text can go down to 7px. Section settings keeps one height across its tabs instead of resizing under you, and a collection with auth shows whether a credential is actually stored.

## 0.7.1

### Patch Changes

- [#28](https://github.com/MathiasWP/fiber/pull/28) [`26e52be`](https://github.com/MathiasWP/fiber/commit/26e52be8622690167182142c2e20e6af8ea32e88) Thanks [@MathiasWP](https://github.com/MathiasWP)! - Creating a collection happens in a dialog with the name field focused, rather than a strip pushed into the sidebar. The section settings tabs now name what they hold — "Auth · browser" — instead of marking it with a bare dot.

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
