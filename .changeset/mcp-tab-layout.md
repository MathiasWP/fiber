---
"fiber": patch
---

Structure the MCP tab and leave out the clients that aren't there. The tab now lists only the clients it can find on this machine, with the rest one line away — detection is a guess at a directory, not a fact, and a client that already holds an entry is always shown. That check was wrong for Claude Code, whose `~/.claude.json` sits directly in the home directory and so counted as present everywhere; it asks about `~/.claude` now. The tab is in three labelled parts, the copy buttons are bordered controls beside a caption rather than bare text that only appeared on hover, and the snippets wrap instead of scrolling sideways.
