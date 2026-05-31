# Current Status

Updated: 2026-05-31

Use this file as the first contributor context. It is a current-state index, not a roadmap and not a runtime guide for end-user agents.

For runtime agent behavior, use root `llms.txt`. For user-facing commands and setup, use root `README.md`.

## Implemented Now

- `local-history-core` owns storage, restore safety, retention, pruning, Markdown rendering, snapshot-prefix resolution, unified text diff, and shared display formatting.
- `local-history` CLI supports status, manual snapshot, list/recent/history, show, diff, restore, undo-restore, prune, view-root, and Markdown render/rebuild workflows.
- `local-history-sidecar` owns watcher startup/status, polling reconciliation, daemon reuse, Zed-facing render/restore wrappers, atomic watcher-status writes, and watcher diagnostics.
- `local-history-mcp` exposes the current agent tool surface: guide, status, create snapshot, recent snapshots, view snapshot, diff snapshot, restore snapshot, and prune.
- `editors/zed` provides a thin Zed extension boundary for slash-command flows and extension-managed MCP startup.
- CI covers the native workspace and the Zed extension through `xtask`.

## Current Behavior Contracts

- Raw snapshots store the previous known file state before a save/delete, not the newly saved contents.
- Restore is safety-first: the live file state is snapshotted before restore writes the selected snapshot.
- Storage lives outside the user repository under the platform data directory. Generated Markdown is rebuildable presentation, not the durable source of truth.
- Human CLI/MCP summaries display timestamps in local time with an explicit `UTC`, `+HH:MM`, or `+HH:MM:SS` suffix. JSON and structured MCP snapshot timestamps remain canonical UTC.
- Stored snapshot IDs stay opaque. Human displays use the shared 12-character prefix; restore/show/diff accept full IDs or unique prefixes of at least 6 characters.
- Watcher status reports oversized snapshot skips with `skipped_snapshot_count` and `last_skipped_snapshot`.

## Current Limits

- Ignore behavior is built-in only today. The current built-in policy skips `.git/`, common dependency/build/cache directories, environment/secret files, SQLite/database files, and logs.
- `.gitignore`, `.local-history-ignore`, global ignore config, and user-configurable retention are future work, not current behavior.
- Default retention is currently `250` snapshots per file, `512 MiB` project storage, `4 MiB` per snapshot, and `30` days max snapshot age.
- Large files above the cap are skipped with diagnostics. Chunked storage is intentionally deferred until there is a proven large-file requirement.
- MCP does not start the watcher. Use `local-history-sidecar ensure-daemon` or the Zed extension slash flow where available.
- Native Zed UI panels are not implemented; Markdown, CLI, sidecar commands, and MCP tools are the current recovery surfaces.

## Open Work To Check Before Starting

- `agents/DEVELOPMENT_PLAN.md` contains long historical stages plus open validation work. Treat it as roadmap context, not as the shortest source of truth.
- `agents/GOALS.md` captures product direction and earlier design reasoning. Some sections are intentionally future-looking.
- `RHYTHM.md` records meaningful decisions and behavior changes. Add a short entry when architecture, workflow, or contracts change.

## Recommended Contributor Context

Read in this order:

1. `agents/CURRENT_STATUS.md`
2. `agents/AGENTS.md`
3. The relevant crate README or source module
4. `agents/FINALIZE.md` before closing the change

Use `agents/GOALS.md` and `agents/DEVELOPMENT_PLAN.md` only when you need product intent, milestone history, or roadmap context.
