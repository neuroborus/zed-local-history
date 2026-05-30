# zed-local-history

Filesystem-first local history for Zed.

This project stores previous saved states of files outside the user repository, exposes them through a native CLI, a native sidecar watcher, generated Markdown history views, a thin Zed extension, and an additive MCP stdio server. The recovery path does not depend on Git, stash, or a custom editor UI.

## How To CLI

Use `local-history` for normal terminal workflows. Use `local-history-sidecar` only for watcher startup/status and Zed-facing render helpers.

```bash
# Start and inspect the watcher for a project.
local-history-sidecar ensure-daemon /path/to/project
local-history-sidecar status /path/to/project

# List the latest raw snapshots.
local-history recent /path/to/project

# Get full snapshot IDs and machine-readable metadata.
local-history recent /path/to/project --json

# Preview one snapshot before restoring.
local-history show <snapshot-id-or-unique-prefix>

# Show code changes between one snapshot and the current live file.
local-history diff <snapshot-id-or-unique-prefix>

# Browse snapshots interactively in the terminal.
local-history browse /path/to/project

# Restore from the latest list by number.
local-history restore --project-root /path/to/project --recent 1

# Restore by full snapshot ID or any unique snapshot ID prefix.
local-history restore <snapshot-id-or-unique-prefix>

# Undo the latest restore.
local-history undo-restore /path/to/project

# Create a manual snapshot of one file.
local-history snapshot /path/to/project --file relative/path.txt

# Generate Markdown history for a specific UTC hour.
local-history render-markdown hour /path/to/project --hour 2026-05-30T18

# Print the generated Markdown view root.
local-history view-root /path/to/project

# Apply the default retention policy and rebuild the Markdown view.
local-history prune /path/to/project
```

Typical loop:

1. Start the watcher once with `local-history-sidecar ensure-daemon`.
2. Edit and save files normally.
3. Use `local-history recent` to find a recovery point.
4. Use `local-history diff <snapshot-id-or-prefix>` to inspect code changes before restoring.
5. Prefer `local-history restore --project-root <project> --recent <n>` for quick restores from the current list.
6. Use `undo-restore` immediately if the restore was not the one you wanted.

For contributor setup and manual Zed validation, use [agents/ZED_MANUAL_TESTING.md](agents/ZED_MANUAL_TESTING.md).

## What it does

- watches a project and stores the previous on-disk state of a file when that file changes or is deleted;
- keeps snapshot metadata in SQLite and stores snapshot contents as compressed content-addressed blobs;
- groups raw snapshots by hour and fixed 10-minute windows;
- generates filesystem-browsable Markdown history under an external `view/` directory;
- restores an exact snapshot by full ID, unique ID prefix, or recent-list number;
- always creates a safety snapshot before restore;
- can undo the last restore;
- prunes old or excess history while preserving the latest restore/undo chain needed for recovery;
- integrates with Zed through slash commands and sidecar bootstrap;
- exposes MCP tools for agent clients such as the Zed Agent Panel.

## Current surfaces

- `local-history`
  User-facing CLI for snapshotting, browsing, restore, undo, Markdown generation, and pruning.
- `local-history-sidecar`
  Native daemon/process boundary for watcher startup, watcher status, restore, and Markdown render commands used by Zed.
- `editors/zed`
  Thin Zed extension package that resolves or downloads the sidecar, exposes focused slash commands where Zed supports them, and registers the MCP context server for the Zed Agent Panel.
- `local-history-mcp`
  MCP stdio server for agent-facing tool calls.

## Crate Responsibilities

- [local-history-core](crates/local-history-core/README.md)
  Core domain model, storage layout, SQLite metadata, content-addressed blobs, restore safety, retention, and Markdown rendering.
- [local-history-cli](crates/local-history-cli/README.md)
  User-facing terminal commands, human output, JSON output, interactive browse, and restore command ergonomics.
- [local-history-sidecar](crates/local-history-sidecar/README.md)
  Watcher runtime, daemon/status behavior, Zed-facing JSON command boundary, and render/restore wrappers used by the extension.
- [local-history-mcp](crates/local-history-mcp/README.md)
  MCP stdio JSON-RPC adapter, tool schemas, structured agent output, and agent-facing restore/status/listing tools.

## How it works

1. The sidecar scans a project root and keeps an in-memory view of tracked files.
2. When a file changes, the sidecar stores the previous known contents as a raw snapshot.
3. When a file is deleted, the sidecar stores the previous known contents before dropping that file from the cache.
4. Restore writes the chosen snapshot back to the live file only after creating a safety snapshot of the current state.
5. Markdown generation reads from stored raw snapshots and writes history pages into external storage. Generated Markdown is not the source of truth.
6. The MCP server maps tool calls onto the same storage and restore paths instead of becoming a second source of business logic.

## History Storage And Markdown View

Local history has two layers:

- **Storage layer**: the durable source of truth. It lives outside the user repository under the platform data directory. Each project gets a stable project directory containing `metadata.sqlite`, compressed content-addressed blobs, generated `view/` files, and watcher logs.
- **Markdown view**: a generated browsing layer. It is safe to delete and rebuild because it is derived from stored snapshots. It is not the database and it is not where snapshot contents are primarily stored.

The watcher captures the previous known file state on save. For example, if `note.txt` starts as `v1`, then the user saves `v2`, the raw snapshot stores `v1`. If the user then saves `v3`, the next raw snapshot stores `v2`.

Markdown is for browsing and copy/paste recovery:

1. Generate or rebuild the view with `render-markdown` or `rebuild-markdown-view`.
2. Open the returned Markdown path, or run `local-history view-root <project-root>` and open the generated `view/` tree.
3. Navigate from the root index to a day/hour page, then to a fixed 10-minute segment, then to an exact snapshot page.
4. Inspect the timestamp, file path, snapshot ID, and text preview.
5. Restore with the shown restore command or copy the snapshot ID/prefix into `local-history restore`.

Generated Markdown links use absolute local paths under `view/` so they keep working when an editor opens the Markdown outside the original project worktree.

Generated Markdown can become stale after pruning. If a Markdown link no longer restores, confirm the snapshot still exists with `local-history show <snapshot-id-or-unique-prefix>` or `local-history recent <project-root>`.

## Quick start

Start the watcher for the current project:

```bash
local-history-sidecar ensure-daemon .
```

Check watcher and storage status:

```bash
local-history-sidecar status .
```

Create a manual snapshot of one file:

```bash
local-history snapshot . --file README.md
```

List the latest raw snapshots:

```bash
local-history recent .
```

Restore the newest raw snapshot from the recent list:

```bash
local-history restore --project-root . --recent 1
```

Inspect code changes before restoring:

```bash
local-history diff <snapshot-id-or-unique-prefix>
```

Undo that restore:

```bash
local-history undo-restore .
```

Generate an hour Markdown view:

```bash
local-history render-markdown hour . --hour 2026-05-03T14
```

Start the MCP stdio server:

```bash
local-history-mcp
```

## Common workflows

### 1. Watch a project continuously

Start or verify the watcher:

```bash
local-history-sidecar ensure-daemon /absolute/path/to/project
```

Read status:

```bash
local-history-sidecar status /absolute/path/to/project
```

Current watcher behavior:

- initial scan builds state without snapshotting every file immediately;
- only saved on-disk changes are captured;
- polling is used instead of OS-native event subscriptions;
- unchanged contents do not create duplicate snapshots;
- atomic replace save patterns are handled;
- files larger than the snapshot size cap are skipped instead of repeatedly failing the watcher loop.

### 2. Capture a manual snapshot

Manual snapshots are useful for precise experiments or before risky edits:

```bash
local-history snapshot . --file src/lib.rs
```

Manual snapshotting is exact and per-file. It does not create a project-wide checkpoint.

### 3. Browse recent snapshots

Basic recent list:

```bash
local-history recent .
```

Recent list with JSON:

```bash
local-history recent . --json
```

Paginated list with filters:

```bash
local-history list . --page 1 --page-size 20 --file src/lib.rs --from 2026-05-03T10:00:00Z --to 2026-05-03T11:00:00Z
```

Important behavior:

- `recent` shows raw user snapshots only;
- human tables show compact snapshot ID prefixes, while `--json` includes full IDs;
- `show` and `restore` accept a full snapshot ID or any unique snapshot ID prefix;
- safety snapshots are intentionally excluded from normal recent numbering;
- `list` can include filtered or paginated snapshot views;
- `recent`, `list`, `show`, `status`, and `safety-list` support `--json`.

### 4. Inspect one snapshot

Show a stored snapshot:

```bash
local-history show <snapshot-id-or-unique-prefix>
```

This resolves the owning project automatically from external storage.

### 5. Restore safely

Restore by full snapshot ID or unique snapshot ID prefix:

```bash
local-history restore <snapshot-id-or-unique-prefix>
```

Restore by recent-list position:

```bash
local-history restore --project-root . --recent 1
```

What restore guarantees:

- a safety snapshot is always created first;
- the restore operation is recorded;
- the live file is rewritten only after the safety snapshot exists;
- the latest restore can be undone.

Inspect safety snapshots:

```bash
local-history safety-list .
```

Undo the latest restore:

```bash
local-history undo-restore .
```

Explicitly restore the newest safety snapshot:

```bash
local-history restore-last-safety .
```

Inspect code changes between a snapshot and the current live file:

```bash
local-history diff <snapshot-id-or-unique-prefix>
```

The diff direction is snapshot to current file. For a raw snapshot captured before a bad save, removed lines show what the snapshot had and added lines show what the live file has now.

### 6. Use interactive browse mode

```bash
local-history browse .
```

Current browse behavior:

- page through raw snapshots;
- preview one snapshot by number;
- confirm restore explicitly before applying it.

### 7. Query grouped history

Hour history:

```bash
local-history history hour . --hour 2026-05-03T14
```

10-minute segment history:

```bash
local-history history segment . --from 2026-05-03T14:10:00Z --to 2026-05-03T14:20:00Z
```

Grouping rules:

- raw snapshots remain the exact restore targets;
- grouping is additive, not lossy;
- each hour is always divided into six fixed 10-minute windows.

### 8. Generate Markdown history views

Find the generated view root:

```bash
local-history view-root .
```

Generate one hour:

```bash
local-history render-markdown hour . --hour 2026-05-03T14
```

Generate one fixed 10-minute segment:

```bash
local-history render-markdown segment . --from 2026-05-03T14:10:00Z --to 2026-05-03T14:20:00Z
```

Rebuild the entire Markdown tree:

```bash
local-history rebuild-markdown-view .
```

Generated Markdown currently includes:

- root `README.md` with hour links;
- one `README.md` per hour;
- six fixed segment pages per hour;
- exact snapshot pages with metadata, restore command, and text preview when available.

Use the Markdown pages as a filesystem browser for history. The exact snapshot page is the important restore target: it contains the durable snapshot ID and a restore command. The command still goes through `local-history restore`, so restore safety behavior is the same as CLI restore from `recent`.

### 9. Prune history

Apply retention rules:

```bash
local-history prune .
```

Current default retention policy:

- max `250` snapshots per file;
- max `512 MiB` referenced project storage;
- max `4 MiB` snapshot file size;
- max `30` days snapshot age.

Pruning also:

- removes stale restore-operation rows;
- deletes orphaned blobs;
- rebuilds the Markdown view.

## CLI command reference

### Snapshot and restore

```bash
local-history snapshot <project-root> --file <relative-path>
local-history restore <snapshot-id-or-unique-prefix>
local-history restore --project-root <project-root> --recent <index>
local-history undo-restore <project-root>
local-history restore-last-safety <project-root>
local-history safety-list <project-root>
```

### Query and browse

```bash
local-history recent <project-root> [--json]
local-history list <project-root> --page <n> --page-size <n> [--file <relative-path>] [--from <rfc3339>] [--to <rfc3339>] [--hour <YYYY-MM-DDTHH>] [--json]
local-history show <snapshot-id-or-unique-prefix>
local-history diff <snapshot-id-or-unique-prefix>
local-history browse <project-root>
```

### Grouped history and Markdown

```bash
local-history history hour <project-root> --hour <YYYY-MM-DDTHH>
local-history history segment <project-root> --from <rfc3339> --to <rfc3339>
local-history view-root <project-root>
local-history render-markdown hour <project-root> --hour <YYYY-MM-DDTHH>
local-history render-markdown segment <project-root> --from <rfc3339> --to <rfc3339>
local-history rebuild-markdown-view <project-root>
```

### Retention and maintenance

```bash
local-history status <project-root> [--json]
local-history prune <project-root> [--json]
```

## Sidecar command reference

```bash
local-history-sidecar health
local-history-sidecar version
local-history-sidecar status <project-root>
local-history-sidecar ensure-daemon <project-root>
local-history-sidecar watch <project-root>
local-history-sidecar view-root <project-root>
local-history-sidecar render-markdown current-hour <project-root>
local-history-sidecar render-markdown previous-hour <project-root>
local-history-sidecar render-markdown current-segment <project-root>
local-history-sidecar render-markdown hour <project-root> --hour <YYYY-MM-DDTHH>
local-history-sidecar render-markdown segment-at <project-root> --at <rfc3339>
local-history-sidecar restore <snapshot-id-or-unique-prefix>
```

## MCP server

`local-history-mcp` is a newline-delimited JSON-RPC stdio server that exposes local-history tools through the Model Context Protocol.

Run it directly:

```bash
local-history-mcp
```

Local usage help:

```bash
local-history-mcp --help
```

### Current MCP tools

- `local_history_status`
- `local_history_guide`
- `local_history_create_snapshot`
- `local_history_recent_snapshots`
- `local_history_view_snapshot`
- `local_history_restore_snapshot`
- `local_history_prune`

### Current MCP resources

- `local-history://guide`
  Complete agent operating guide, packaged from [llms.txt](llms.txt), covering storage, snapshot semantics, restore safety, Markdown browsing, CLI usage, MCP usage, and Zed integration boundaries.

Current tool contract:

- `local_history_guide` returns the same guide text as the `local-history://guide` resource for MCP clients that expose tools more reliably than resources;
- most tools require explicit `project_root`;
- snapshot view and restore work by full `snapshot_id` or any unique snapshot ID prefix;
- all tools accept optional `data_dir` when you want to use a non-default local-history storage base directory;
- `local_history_restore_snapshot` remains safety-first and creates a safety snapshot before writing the live file;
- `local_history_diff_snapshot` does not exist yet; use CLI `local-history diff <snapshot-id-or-unique-prefix>` for textual diff against the current live file.

### Zed Agent Panel usage

The Zed Agent Panel uses MCP tools, not extension slash commands.

When the extension is installed, it registers the `local-history` context server automatically and resolves the matching MCP binary for Agent Panel use.

Ask the Agent in natural language:

```text
Use local-history to show status for /absolute/path/to/project.
```

If you want to configure the MCP server manually instead, point Zed at an installed or unpacked `local-history-mcp` executable:

```json
{
  "context_servers": {
    "local-history": {
      "command": "/absolute/path/to/local-history-mcp",
      "args": []
    }
  }
}
```

Current release contract:

- platform bundles now include `local-history`, `local-history-sidecar`, `local-history-mcp`, `README.md`, and `LICENSE`;
- fixed-name sidecar-only and MCP-only archives exist separately for Zed extension bootstrap.

### Example MCP usage

Examples of requests that map well to the current tool surface:

- "Show local-history status for `/absolute/path/to/project`."
- "Create a snapshot of `src/lib.rs` under `/absolute/path/to/project`."
- "List the last 10 local-history snapshots for `/absolute/path/to/project`."
- "Show snapshot `<snapshot-id>`."
- "Restore snapshot `<snapshot-id>`."
- "Prune local history for `/absolute/path/to/project`."

For code-level diffs, use the CLI:

```bash
local-history diff <snapshot-id-or-unique-prefix>
```

### Current MCP limitations

- no prompts surface is exposed yet;
- no MCP diff tool exists yet;
- extension-managed MCP release bootstrap still needs live validation against a real tagged GitHub Release.

## Zed usage

### What the extension does

- resolves, downloads, and caches the matching `local-history-sidecar` release asset;
- verifies sidecar version compatibility before use;
- runs focused sidecar commands from slash handlers;
- registers the `local-history` MCP context server for Agent Panel tool use;
- resolves, downloads, and caches the matching `local-history-mcp` release asset;
- verifies MCP binary version compatibility before launching the context server.

### Current slash commands

These are extension slash commands for Zed surfaces that support extension slash commands. They are not commands in the new Zed Agent Panel command menu.

- `/local-history-status`
- `/local-history-start-watcher`
- `/local-history-view`
- `/local-history-current-hour`
- `/local-history-current-segment`
- `/local-history-previous-hour`
- `/local-history-hour <YYYY-MM-DDTHH>`
- `/local-history-segment <YYYY-MM-DDTHH:MM:SSZ>`
- `/local-history-restore <snapshot-id-or-unique-prefix>`

### Important Zed limitation

The current Zed extension API does not provide a direct action for opening an arbitrary external file path. The extension therefore exposes or prints the generated Markdown path instead of pretending it can always open that file automatically.

## Storage, privacy, and safety

### Storage location

Local history is stored outside the user repository:

- macOS: `~/Library/Application Support/local-history`
- Linux: `$XDG_DATA_HOME/local-history` or `~/.local/share/local-history`
- Windows: `%LOCALAPPDATA%\\local-history`

### Per-project layout

```text
projects/<project-id>/
  metadata.sqlite
  blobs/
  view/
  logs/
```

Meaning:

- `metadata.sqlite`
  snapshot metadata, tracked files, restore operations, generated Markdown index
- `blobs/`
  compressed content-addressed snapshot contents
- `view/`
  generated Markdown history tree
- `logs/`
  watcher log and watcher heartbeat/status files

### Privacy and capture rules

- any non-ignored file may be snapshotted if it is saved or manually targeted;
- secrets in normal source files can be captured if they are not covered by ignore rules;
- generated Markdown lives outside the repository and does not recursively snapshot itself;
- restore always creates a safety snapshot before writing to the live file.

### Built-in ignore rules

Current built-in ignores skip:

- `.git/`
- `node_modules/`
- `target/`
- `dist/`
- `build/`
- `.next/`
- `.cache/`
- `coverage/`
- `.env`
- `.env.*`
- `*.pem`
- `*.key`
- `*.p12`
- `*.pfx`
- `*.sqlite`
- `*.db`
- `*.log`

Current nuance:

- `.local-history-ignore` is reserved in the policy model, but project-local custom ignore parsing is not wired yet;
- ignore behavior is built-in only today.

### Deleting history

Delete one project's history by removing its external project directory:

```text
projects/<project-id>/
```

Delete all history by removing the whole base `local-history` directory.

## Troubleshooting

### Sidecar not starting

- run `local-history-sidecar status <project-root>`;
- inspect `projects/<project-id>/logs/watcher.log`;
- inspect `projects/<project-id>/logs/watcher-status.json`.

### Watcher not detecting changes

- confirm you are saving to disk, not only changing in-memory buffers;
- confirm the path is not ignored;
- remember that the watcher is polling-based, not event-driven.

### Storage too large

- run `local-history status <project-root>`;
- run `local-history prune <project-root>`.

### Markdown not updating

- run `local-history rebuild-markdown-view <project-root>`;
- confirm raw snapshots actually exist with `recent` or `list`.

### Restore failure

- confirm the snapshot still exists with `local-history show <snapshot-id-or-unique-prefix>` or `local-history recent <project-root>`;
- inspect the live-file difference with `local-history diff <snapshot-id-or-unique-prefix>`;
- note that a previously generated Markdown link can outlive the snapshot it references after pruning.

### Unsupported platform in Zed bootstrap

Current Zed binary bootstrap contract covers sidecar and MCP assets for:

- macOS `x86_64`
- macOS `aarch64`
- Linux `x86_64`
- Linux `aarch64`
- Windows `x86_64`
- Windows `aarch64`

Current limitation:

- `x86_64-unknown-linux-musl` is not part of the extension bootstrap contract because the current platform mapping distinguishes OS and CPU architecture, not Linux libc family.

## Current limitations

- there is no project-wide checkpoint abstraction yet; snapshots are per-file;
- the watcher is polling-based rather than OS-event-based;
- project-local custom ignore files are not wired yet;
- the Zed extension reveals Markdown paths instead of directly opening arbitrary external files;
- release workflow and extension bootstrap still need live external validation on a real tagged release;
- the MCP server still needs live validation inside a real Zed Agent `context_servers` setup and does not yet expose prompts or diff tools.

## Repository layout

```text
zed-local-history/
  README.md
  RHYTHM.md
  llms.txt
  Cargo.toml
  rust-toolchain.toml
  agents/
  crates/
    local-history-core/
    local-history-cli/
    local-history-sidecar/
    local-history-mcp/
  editors/
    zed/
  xtask/
```

## Example end-to-end session

Start the watcher:

```bash
local-history-sidecar ensure-daemon .
```

Edit and save `src/lib.rs`, then inspect the newest raw snapshot:

```bash
local-history recent .
```

Restore the first item from the recent list:

```bash
local-history restore --project-root . --recent 1
```

If the restore was wrong, undo it:

```bash
local-history undo-restore .
```

Generate an hour view for browsing:

```bash
local-history render-markdown hour . --hour 2026-05-03T14
```

Look up the generated view root:

```bash
local-history view-root .
```
