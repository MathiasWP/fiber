<div align="center">
  <img src="src-tauri/icons/128x128@2x.png" alt="Fiber" width="116" />
  <h1>Fiber</h1>
  <p><b>A modern &amp; secure local alternative to Postman, Insomnia and Hoppscotch.</b></p>
</div>

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/MathiasWP/fiber/main/scripts/install-macos.sh | bash
```

Windows and Linux: [Releases](https://github.com/MathiasWP/fiber/releases).

## Collections

A collection is one TOML file in
`~/Library/Application Support/dev.fiber.app/sections/`. It holds a base URL and
a list of requests. A request is a method and a path.

The files are plain text in a stable order, so they diff cleanly if you keep
them in git. Edits save themselves.

Each collection has its own cookie jar, timeout, redirect policy, optional
proxy and TLS setting — so a session cookie from one API never rides along
with another.

There are no variables. If one request needs a different host, type a full URL
in its path. That is the only way around the base URL.

A request can also belong to no collection at all — the **new request** button
makes a loose one that takes a full URL.

Bodies are JSON by default. A request can also send plain text, form-urlencoded
fields, multipart (including files, by path) or a raw file. Query parameters
are a tab on every method, not only GET. Path parameters from `{name}`
placeholders are filled in when you send.

You get endpoints into a collection in three ways, and they mix:

1. **Type them.**
2. **Import an OpenAPI or Swagger file** — JSON or YAML, 3.x or 2.0. Operations
   become ordinary requests, so nothing goes stale and it works offline. Tags
   become folders, path and query parameters are filled in from the spec, and
   each operation's description is kept (handy for agents talking to Fiber over
   MCP).
3. **Point a loader at the API** — see below. When the manifest is OpenAPI,
   Fiber also derives the same folders, parameters, example bodies and response
   schemas a file import would.

## Auth

Set auth once per collection, under *Section settings → Auth*. Fiber then keeps
the token fresh by itself. That is the point of it: no pasting a new token every
hour.

There are three kinds.

**Bearer** — a fixed token. You paste it once.

**Login request** — Fiber fetches the token by making a request. You say where
the token sits in the reply, like `$.data.access_token`. This is the one for
machine-to-machine APIs.

**Browser session** — you sign in in a real browser window, and Fiber lifts the
credential out afterwards. Use it when a plain request cannot do the sign-in:
a code sent by email, a token an SDK makes inside the page, or a session cookie
the server marks HttpOnly.

### It fixes its own 401s

When the API answers 401, Fiber throws the token away, gets a new one, and sends
the request again. **Once.** If the second try fails too, you get the 401 — there
is no retry loop.

A fixed Bearer token is never retried. It has not changed, so there is nothing
to refresh.

For a browser session, the sign-in page opens hidden. If your login provider
still thinks you are signed in, a new credential arrives and you never see a
window.

### Picking a credential

Sign in, then press *Pick credential*. You can close the sign-in window first —
the session is remembered. Fiber lists everything that session holds:

- every **cookie**, HttpOnly included, from every domain
- every **localStorage** entry, flattened to leaf paths, with MSAL v4's
  encrypted ones decrypted first
- every **IndexedDB** record, as `database/store/key` — where the Firebase SDK
  keeps its session

Anything that matches a known provider's format is labelled and sorted to the
top: Auth0, Supabase, Firebase, MSAL, Cognito, Okta, Clerk, Keycloak,
Auth.js/NextAuth, Better Auth and more.

A collection with auth set up shows a small shield: filled when a credential is
stored, amber when there is none. It only tells you one is there — whether it
still works cannot be known without sending something, since an expired cookie
looks fine until the server says otherwise.

### Where secrets live

In the OS keychain. The collection file holds only a reference to one, so the
file stays safe to share or commit.

The app can write a secret and ask whether one exists. It cannot show you one,
and nothing in the UI reads one back. The keychain is read once per run and kept
in memory from then on.

The single exception is `fiber mcp export-secrets`, which exists so that a
containerised copy can be given the credentials it cannot fetch itself — see
[`deploy/toolhive.md`](deploy/toolhive.md). It covers only the collections you
have shared over MCP, it writes to a pipe or an encrypted file and refuses a
terminal, and you have to run it deliberately. Once you have set that up, the
app keeps the file current as you sign in, so a container stops going stale;
without it, nothing is ever written to disk.

A header you type on a request beats the collection's auth — for "just this
once, use a different token". `Cookie` is the exception, and has to be: cookies
are a `;`-joined list, so one you type is added to the session's rather than
replacing it.

## Loaders

A loader keeps a collection's endpoints up to date with the API. Point it at the
URL that lists your routes, and write a small [jq](https://jqlang.org/) filter
that turns that JSON into endpoints.

```jq
.routes | map({method: .verb, path: .url, name: .handler})
```

Each result needs a `method` and a `path`. `name`, `description` and `tag` are
optional. The editor ships starting points for common shapes, OpenAPI included.

**Why a jq filter and not a script.** A filter cannot do anything except turn
JSON into other JSON. No file access, no network, no way to reach the host. So a
collection with a loader in it is safe to open, and safe to let an agent write.
(`env` is removed as well, so a filter cannot read your environment either.)

And because it only transforms, the editor can run it against a fetched sample
while you type and show you the result. You never run a script and read a stack
trace.

**Paging.** jq cannot make a second request, so paging is its own field: another
filter that returns the next page's URL, or null when there are no more. Up to
50 pages, and 30 seconds for the whole run.

**Your work survives a refresh.** Loaded endpoints are a cache, never the source
of truth. The body and headers you write are stored beside them and matched by
`METHOD /path`, so a refresh finds them again and puts them back. If the API
stops listing an endpoint, it is marked *missing* rather than deleted. Losing a
body to a refresh is the one thing this design exists to prevent.

## MCP

The app you already installed *is* the MCP server — `fiber mcp` is the same
binary with a different first argument. There is nothing else to install, and
nothing to build.

The **MCP** tab, beside Collections and History, is the short way in. It knows
Claude Code, Claude Desktop, Cursor, VS Code, Windsurf, Codex CLI and Gemini
CLI, and lists the ones it can find on this machine with the config file each
uses. **Add** writes the entry — pointing at wherever this copy of Fiber
actually lives, which is the part that is tedious to find by hand. The clients
it didn't find are one line away, under the list, since that check is a guess
at a directory rather than a fact. An entry left behind by a copy that moved shows as
*Update*. Fiber only ever adds or removes its own key: other servers, other
settings and (in Codex's TOML) every comment stay exactly as they were, and a
config file that doesn't parse is left alone with the snippet offered instead.

By hand, it is one command for Claude Code:

```sh
# macOS
claude mcp add fiber -- /Applications/Fiber.app/Contents/MacOS/fiber mcp
```

and the same two things in any other client's config file:

```jsonc
{ "fiber": { "command": "/Applications/Fiber.app/Contents/MacOS/fiber", "args": ["mcp"] } }
```

The binary lives at `/Applications/Fiber.app/Contents/MacOS/fiber` on macOS,
`/usr/bin/fiber` on Linux, and `Fiber.exe` in the install directory on Windows.

It reads the same files and needs no running app. Search results include each
operation's description and tag, so an agent can tell endpoints apart the same
way a person reading the spec would. A new collection is **shared
read-only** — visible and callable with GET/HEAD/OPTIONS — with a second switch
for anything more. Turn sharing off and the collection is hidden completely, not
just read-only. Credentials are applied on the way out and stripped from
everything that comes back.

### When the method says nothing

That switch assumes GET means read and POST means write. For an API where every
call is a POST it is useless: off, and the API can't be used; on, and there is no
guard left.

So a collection can carry an **access policy** instead — a jq filter answering
`"allow"`, `"ask"` or `"deny"` per endpoint, reading what the manifest already
publishes about each one. Every scalar `x-` extension on an OpenAPI operation
comes through the loader, so a rule can be written against the API's own words:

```jq
if   .meta["x-kind"] == "query"   then "allow"
elif .meta["x-kind"] == "command" then "ask"
else "deny" end
```

Fiber knows nothing about `x-kind`, or any other key. The filter is where meaning
is attached, in one place you can read and change. Section settings runs it
against the collection's real endpoints as you type and shows what each one gets.

`"ask"` puts the call in front of you — in your agent's client, which is where
you are sitting when an agent is working — and sends it only if you approve.
One approval covers one call. If nobody answers within ten minutes, or the
client has no way to show a prompt, the call is refused rather than sent.

A policy replaces the switch entirely while it is set, GET included, so a read
that returns your whole customer table can say so. Anything it cannot answer
for — including a path the collection doesn't list, which is how an agent would
reach an endpoint nobody reviewed — is denied, as is everything in a collection
whose policy has a mistake in it.

Two tools exist for jq specifically: `loader_manifest` fetches your raw
manifest, and `try_loader_filter` tests a filter against it. So you can ask an
agent to write a loader filter instead of learning jq first.

### Without the app

The MCP tab links to this, and hands over the command below — it needs ToolHive
and a container runtime, which is a terminal's job rather than a button's.

It also builds without the desktop app at all, as a ~30MB static image for a
container manager — for a collections repo on a server, or to put ToolHive's
proxy and audit log in front of your agents. That install is one command too:

```sh
curl -fsSL https://raw.githubusercontent.com/MathiasWP/fiber/main/scripts/toolhive.sh | bash
```

It finds your collections, copies the credentials for the collections you have
shared straight from the keychain into ToolHive's encrypted store, and starts
the server. Nothing is typed twice and nothing is pasted.

See [`deploy/toolhive.md`](deploy/toolhive.md) for what each step does, and for
running it by hand.

## History

SQLite in the app data dir, kept per request: the newest 50 of each, nothing
older than 30 days. Bodies over 256KB are stored as files.

## Keys

| | |
|---|---|
| `⌘↵` | send |
| `⌘K` | search endpoints |
| `⌘A` | select the whole response, when focused in it |
| `⌘+` `⌘-` `⌘0` | text size, and back to default — applies to whichever editor you last used, so the response can be smaller than the body |
| right-click | context menu — rename, duplicate, delete, refresh, copy |
| drag | reorder requests and collections, or move a request between them |

Panes are draggable and their sizes persist. The theme follows the system until
you pick one, bottom-left of the sidebar; right-click there to follow it again.

## Building it

Running, releasing and signing: [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE).
