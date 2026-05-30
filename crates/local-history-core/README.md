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
- retention and prune behavior.

## Owns

- SQLite schema and storage migrations in the current single-schema model.
- Snapshot ID, project ID, content hash, and restore domain types.
- Filesystem-browsable Markdown rendering from stored snapshots.
- Safety-first restore behavior.

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

Core tests should cover storage, restore safety, prune behavior, ignore rules, history grouping, and Markdown rendering without depending on editor or process behavior.
