---
"fiber": minor
---

Add an MCP tab beside Collections and History that installs Fiber into an AI client for you. It lists Claude Code, Claude Desktop, Cursor, VS Code, Windsurf, Codex CLI and Gemini CLI with the config file each one uses, and Add writes the entry pointing at wherever this copy of the app actually lives. An entry left behind by a copy that has moved shows as Update. The edit only ever adds or removes Fiber's own key: other servers and settings survive, Codex's hand-written TOML keeps its comments and key order, and a config file that doesn't parse is left untouched with the snippet offered to paste instead. Below the list, the ToolHive route is offered as a copyable command with a link to its guide, for collections served from a repo rather than this machine.
