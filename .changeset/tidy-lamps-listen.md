---
"fiber": patch
---

The MCP server is easier to reach. The README now names the binary's real path on each OS instead of `/path/to/fiber`, so installing it is one `claude mcp add`. For the containerised server, `scripts/toolhive.sh` sets ToolHive up in a single command, and a new `fiber mcp export-secrets` pipes the credentials for your shared collections into ToolHive's secret store rather than having you copy each one out by hand. The container image itself now builds — it had not, since the Linux keychain backend needs a D-Bus library no container has — and is published for amd64 and arm64 on every release.
