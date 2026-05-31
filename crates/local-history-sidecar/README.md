# local-history-sidecar

Native process boundary for watcher runtime and Zed-facing commands.

## Responsibility

`local-history-sidecar` owns process-oriented behavior that should not live in the Zed WebAssembly extension:

- project watcher startup and status;
- foreground and daemon watcher modes;
- polling-based file change reconciliation;
- JSON command output used by the Zed extension;
- sidecar render wrappers for current hour, current segment, previous hour, and selected windows;
- restore wrapper for Zed slash-command flows.

## Owns

- Watcher runtime state and heartbeat files.
- Daemon spawning behavior.
- Sidecar command parsing and JSON response shape.
- Read-only status and view-root diagnostics that do not initialize storage.
- Runtime translation between watched filesystem state and `local-history-core` snapshots.

## Does Not Own

- Durable storage schema or restore business rules.
- User-facing CLI ergonomics beyond sidecar diagnostics.
- Zed extension UI or release bootstrap.
- MCP protocol behavior.

## Used By

- The Zed extension slash-command handlers.
- Manual fallback workflows while the friendly CLI shortcuts are still planned.
- Contributors debugging watcher behavior.

## Validation

Sidecar tests should cover command parsing, watcher reconciliation, status freshness, atomic replace saves, deletion capture, and end-to-end watcher/restore/Markdown flows.
