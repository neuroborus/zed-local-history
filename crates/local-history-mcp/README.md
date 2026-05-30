# local-history-mcp

MCP stdio adapter for agent-facing local-history tools.

## Responsibility

`local-history-mcp` exposes local-history behavior to MCP clients. Agents without MCP tools should use the CLI workflow documented in root `llms.txt`.

- JSON-RPC stdio server lifecycle;
- MCP initialize, ping, tools/list, tools/call, resources/list, and resources/read handling;
- tool schemas and descriptions;
- server instructions, the `local_history_guide` tool, and the packaged `local-history://guide` agent resource;
- structured tool output for status, snapshot creation, recent snapshots, snapshot view, snapshot diff (including `unchanged`), restore, and prune;
- safety-first restore access through MCP.

`local-history-mcp` exposes the same unified text diff as CLI `local-history diff <snapshot-id-or-unique-prefix>` through `local_history_diff_snapshot`.

## Owns

- MCP protocol boundary.
- Tool names, input schemas, summaries, and structured output shape.
- Agent instructions and MCP resource exposure.
- Read-only agent guide tool exposure.
- Agent-facing error messages.
- Snapshot ID prefix resolution for MCP tool input.

## Does Not Own

- Snapshot storage or restore business logic.
- Watcher process management.
- CLI command parsing or terminal UX.
- Zed extension release bootstrap.
- Canonical product documentation beyond packaging the root `llms.txt` guide for MCP clients.

## Used By

- Zed Agent Panel through extension-managed MCP registration.
- Manual `context_servers` configuration when an explicit binary path is preferred.
- Other MCP clients that can launch a stdio server.

Agents in hosts without MCP should follow the CLI mapping in `llms.txt` instead of expecting these tools.

## Validation

MCP tests should cover initialization, tools/list, tools/call, resources/list, resources/read, schema stability, structured output, and restore safety through the MCP boundary. Core storage invariants remain in `local-history-core`.
