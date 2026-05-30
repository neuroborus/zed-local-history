# local-history-mcp

MCP stdio adapter for agent-facing local-history tools.

## Responsibility

`local-history-mcp` exposes local-history behavior to MCP clients such as the Zed Agent Panel:

- JSON-RPC stdio server lifecycle;
- MCP initialize, ping, tools/list, tools/call, resources/list, and resources/read handling;
- tool schemas and descriptions;
- server instructions, the `local_history_guide` tool, and the packaged `local-history://guide` agent resource;
- structured tool output for status, snapshot creation, recent snapshots, snapshot view, restore, and prune;
- safety-first restore access through MCP.

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

## Validation

MCP tests should cover initialization, tools/list, tools/call, resources/list, resources/read, schema stability, structured output, and restore safety through the MCP boundary. Core storage invariants remain in `local-history-core`.
