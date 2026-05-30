# local-history-cli

User-facing terminal interface for `zed-local-history`.

## Responsibility

`local-history-cli` owns human and JSON command-line workflows:

- manual snapshot creation;
- recent, list, show, and browse commands;
- restore by full snapshot ID, unique prefix, or recent-list number;
- safety-list, undo-restore, and restore-last-safety commands;
- hour and segment history queries;
- Markdown view commands;
- status and prune commands.

## Owns

- CLI command names and argument parsing.
- Human-readable output.
- JSON output at the command boundary.
- Interactive browse behavior.
- Snapshot ID prefix resolution for CLI input.

## Does Not Own

- Snapshot persistence or restore business logic.
- Watcher daemon lifecycle.
- Zed extension command behavior.
- MCP tool schemas or JSON-RPC handling.
- Release bootstrap or binary download behavior.

## Used By

- Users in a terminal.
- Manual testing and recovery workflows when Zed UI surfaces are unavailable.
- Generated Markdown restore command examples.

## Validation

CLI tests should focus on command parsing, output contracts, snapshot ID prefix behavior, and user-facing restore flows. Storage invariants belong in `local-history-core`.
