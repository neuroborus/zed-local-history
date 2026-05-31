# local-history-core

Core domain and storage layer for `zed-local-history`.

## Responsibility

`local-history-core` owns the durable recovery model:

- project identity and external storage layout;
- default ignore policy;
- snapshot metadata and compressed content-addressed blobs;
- restore safety snapshots and restore-operation records;
- raw, safety, hour, and segment history queries;
- Markdown view generation and rebuild;
- retention and prune behavior;
- unified text diff from snapshot content to the current live file (newline-aware lines; bounded exact LCS with replace fallback on very large changes);
- local timezone formatting for human-facing timestamp display, including explicit `UTC` / `+HH:MM` suffixes.

## Owns

- SQLite schema and storage migrations in the current single-schema model.
- Storage open modes: write/open-or-create for mutating workflows and read-only open for inspection paths.
- Snapshot ID, project ID, content hash, and restore domain types.
- Shared snapshot ID contracts: 12-character display prefixes for CLI, MCP, and Markdown surfaces, and a 6-character minimum for prefix lookup.
- Filesystem-browsable Markdown rendering from stored snapshots.
- Safety-first restore behavior.
- Shared human timestamp formatting (`format_timestamp_local`) for CLI and MCP display boundaries.

## Does Not Own

- CLI argument parsing or terminal formatting.
- Zed extension API behavior.
- Long-running watcher process management.
- MCP JSON-RPC protocol handling.
- Shell environment, process spawning, or release bootstrap.

## Used By

- `local-history-cli` for user-facing terminal workflows.
- `local-history-sidecar` for watcher, Zed-facing JSON commands, and render wrappers.
- `local-history-mcp` for agent-facing tools.

## Validation

Core tests should cover storage, restore safety, prune behavior, ignore rules, history grouping, Markdown rendering, snapshot-to-live unified diff behavior, and timestamp display formatting without depending on editor or process behavior.
