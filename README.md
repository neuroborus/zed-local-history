# zed-local-history

Filesystem-first local history for Zed.

This project stores previous saved states of files outside the user repository, exposes them through a native CLI, a native sidecar watcher, generated Markdown history views, a thin Zed extension, and an additive MCP stdio server. The recovery path does not depend on Git, stash, or a custom editor UI.

## Quickstart: local Zed dev run

Use this path when testing the extension from this repository.

1. Use a current Zed build.

   Zed Stable 1.4.4 or newer is the current acceptance baseline. Check:

   ```bash
   zed --version
   ```

2. Prepare Rust for the Zed extension.

   The native workspace uses Rust 1.75.0, but the Zed extension builds with stable Rust and `wasm32-wasip2`.

   ```bash
   rustup +stable target add wasm32-wasip2
   cargo run -p xtask -- full-ci
   ```

3. Build the local binaries used during dev testing.

   ```bash
   cargo build -p local-history-sidecar -p local-history-cli -p local-history-mcp
   ```

4. Create a clean test project and launch Zed from the same shell.

   ```bash
   mkdir -p /tmp/lh-zed-manual
   printf 'v1\n' > /tmp/lh-zed-manual/note.txt

   RUSTUP_TOOLCHAIN=stable \
   PATH="$HOME/.cargo/bin:$PWD/target/debug:$PATH" \
   zed --foreground /tmp/lh-zed-manual
   ```

   Launching from the shell matters: Zed must see `rustup`, the stable extension toolchain, and `target/debug/local-history-sidecar` / `target/debug/local-history-mcp` in `PATH`.

5. Install the extension in Zed.

   Open Extensions, choose `Install Dev Extension`, and select:

   ```text
   editors/zed
   ```

6. Start the watcher.

   From a terminal:

   ```bash
   local-history-sidecar ensure-daemon /tmp/lh-zed-manual
   local-history-sidecar status /tmp/lh-zed-manual
   ```

   If you are using a Zed surface that supports extension slash commands, `/local-history-start-watcher` and `/local-history-status` call the same sidecar paths.

7. Use the Zed Agent Panel through MCP, not slash commands.

   The new Agent Panel treats text starting with `/` as Agent commands. Extension slash commands such as `/local-history-status` are not Agent commands, so the Agent may report that they are unrecognized.

   This extension registers the `local-history` MCP context server for Agent use. Ask in natural language, for example:

   ```text
   Use local-history to show status for /tmp/lh-zed-manual.
   ```

   If Zed does not start the extension-managed context server, add the MCP server manually in Zed settings:

   ```json
   {
     "context_servers": {
       "local-history": {
         "command": "local-history-mcp",
         "args": []
       }
     }
   }
   ```

8. Capture and restore.

   Edit and save `/tmp/lh-zed-manual/note.txt`, then inspect snapshots:

   ```bash
   local-history recent /tmp/lh-zed-manual
   ```

   Restore by full snapshot ID or unique snapshot ID prefix:

   ```bash
   local-history restore <snapshot-id-or-unique-prefix>
   ```

   Or restore directly from the latest list number:

   ```bash
   local-history restore --project-root /tmp/lh-zed-manual --recent 1
   ```

   Undo the latest restore:

   ```bash
   local-history undo-restore /tmp/lh-zed-manual
   ```

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

## How it works

1. The sidecar scans a project root and keeps an in-memory view of tracked files.
2. When a file changes, the sidecar stores the previous known contents as a raw snapshot.
3. When a file is deleted, the sidecar stores the previous known contents before dropping that file from the cache.
4. Restore writes the chosen snapshot back to the live file only after creating a safety snapshot of the current state.
5. Markdown generation reads from stored raw snapshots and writes history pages into external storage. Generated Markdown is not the source of truth.
6. The MCP server maps tool calls onto the same storage and restore paths instead of becoming a second source of business logic.

## Requirements

### Native workspace

- Rust `1.75.0` for the root workspace
- `cargo`, `rustfmt`, and `clippy`

### Zed extension package

- Rust installed through `rustup`
- a newer toolchain for `wasm32-wasip2`
- the extension keeps its own toolchain in `editors/zed/rust-toolchain.toml`

## Build and validation

Run native workspace checks:

```bash
cargo run -p xtask -- ci
```

Run Zed extension checks:

```bash
cargo run -p xtask -- zed-ci
```

Run the full repository checks:

```bash
cargo run -p xtask -- full-ci
```

## Quick start

Start the watcher for the current project:

```bash
cargo run -p local-history-sidecar -- ensure-daemon .
```

Check watcher and storage status:

```bash
cargo run -p local-history-sidecar -- status .
```

Create a manual snapshot of one file:

```bash
cargo run -p local-history-cli -- snapshot . --file README.md
```

List the latest raw snapshots:

```bash
cargo run -p local-history-cli -- recent .
```

Restore the newest raw snapshot from the recent list:

```bash
cargo run -p local-history-cli -- restore --project-root . --recent 1
```

Undo that restore:

```bash
cargo run -p local-history-cli -- undo-restore .
```

Generate the current hour Markdown view:

```bash
cargo run -p local-history-cli -- render-markdown hour . --hour 2026-05-03T14
```

Start the MCP stdio server:

```bash
cargo run -p local-history-mcp
```

## Common workflows

### 1. Watch a project continuously

Start or verify the watcher:

```bash
cargo run -p local-history-sidecar -- ensure-daemon /absolute/path/to/project
```

Read status:

```bash
cargo run -p local-history-sidecar -- status /absolute/path/to/project
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
cargo run -p local-history-cli -- snapshot . --file src/lib.rs
```

Manual snapshotting is exact and per-file. It does not create a project-wide checkpoint.

### 3. Browse recent snapshots

Basic recent list:

```bash
cargo run -p local-history-cli -- recent .
```

Recent list with JSON:

```bash
cargo run -p local-history-cli -- recent . --json
```

Paginated list with filters:

```bash
cargo run -p local-history-cli -- list . --page 1 --page-size 20 --file src/lib.rs --from 2026-05-03T10:00:00Z --to 2026-05-03T11:00:00Z
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
cargo run -p local-history-cli -- show <snapshot-id-or-unique-prefix>
```

This resolves the owning project automatically from external storage.

### 5. Restore safely

Restore by full snapshot ID or unique snapshot ID prefix:

```bash
cargo run -p local-history-cli -- restore <snapshot-id-or-unique-prefix>
```

Restore by recent-list position:

```bash
cargo run -p local-history-cli -- restore --project-root . --recent 1
```

What restore guarantees:

- a safety snapshot is always created first;
- the restore operation is recorded;
- the live file is rewritten only after the safety snapshot exists;
- the latest restore can be undone.

Inspect safety snapshots:

```bash
cargo run -p local-history-cli -- safety-list .
```

Undo the latest restore:

```bash
cargo run -p local-history-cli -- undo-restore .
```

Explicitly restore the newest safety snapshot:

```bash
cargo run -p local-history-cli -- restore-last-safety .
```

### 6. Use interactive browse mode

```bash
cargo run -p local-history-cli -- browse .
```

Current browse behavior:

- page through raw snapshots;
- preview one snapshot by number;
- confirm restore explicitly before applying it.

### 7. Query grouped history

Hour history:

```bash
cargo run -p local-history-cli -- history hour . --hour 2026-05-03T14
```

10-minute segment history:

```bash
cargo run -p local-history-cli -- history segment . --from 2026-05-03T14:10:00Z --to 2026-05-03T14:20:00Z
```

Grouping rules:

- raw snapshots remain the exact restore targets;
- grouping is additive, not lossy;
- each hour is always divided into six fixed 10-minute windows.

### 8. Generate Markdown history views

Find the generated view root:

```bash
cargo run -p local-history-cli -- view-root .
```

Generate one hour:

```bash
cargo run -p local-history-cli -- render-markdown hour . --hour 2026-05-03T14
```

Generate one fixed 10-minute segment:

```bash
cargo run -p local-history-cli -- render-markdown segment . --from 2026-05-03T14:10:00Z --to 2026-05-03T14:20:00Z
```

Rebuild the entire Markdown tree:

```bash
cargo run -p local-history-cli -- rebuild-markdown-view .
```

Generated Markdown currently includes:

- root `README.md` with hour links;
- one `README.md` per hour;
- six fixed segment pages per hour;
- exact snapshot pages with metadata, restore command, and text preview when available.

### 9. Prune history

Apply retention rules:

```bash
cargo run -p local-history-cli -- prune .
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
cargo run -p local-history-cli -- snapshot <project-root> --file <relative-path>
cargo run -p local-history-cli -- restore <snapshot-id-or-unique-prefix>
cargo run -p local-history-cli -- restore --project-root <project-root> --recent <index>
cargo run -p local-history-cli -- undo-restore <project-root>
cargo run -p local-history-cli -- restore-last-safety <project-root>
cargo run -p local-history-cli -- safety-list <project-root>
```

### Query and browse

```bash
cargo run -p local-history-cli -- recent <project-root> [--json]
cargo run -p local-history-cli -- list <project-root> --page <n> --page-size <n> [--file <relative-path>] [--from <rfc3339>] [--to <rfc3339>] [--hour <YYYY-MM-DDTHH>] [--json]
cargo run -p local-history-cli -- show <snapshot-id>
cargo run -p local-history-cli -- browse <project-root>
```

### Grouped history and Markdown

```bash
cargo run -p local-history-cli -- history hour <project-root> --hour <YYYY-MM-DDTHH>
cargo run -p local-history-cli -- history segment <project-root> --from <rfc3339> --to <rfc3339>
cargo run -p local-history-cli -- view-root <project-root>
cargo run -p local-history-cli -- render-markdown hour <project-root> --hour <YYYY-MM-DDTHH>
cargo run -p local-history-cli -- render-markdown segment <project-root> --from <rfc3339> --to <rfc3339>
cargo run -p local-history-cli -- rebuild-markdown-view <project-root>
```

### Retention and maintenance

```bash
cargo run -p local-history-cli -- status <project-root> [--json]
cargo run -p local-history-cli -- prune <project-root> [--json]
```

## Sidecar command reference

```bash
cargo run -p local-history-sidecar -- health
cargo run -p local-history-sidecar -- version
cargo run -p local-history-sidecar -- status <project-root>
cargo run -p local-history-sidecar -- ensure-daemon <project-root>
cargo run -p local-history-sidecar -- watch <project-root>
cargo run -p local-history-sidecar -- view-root <project-root>
cargo run -p local-history-sidecar -- render-markdown current-hour <project-root>
cargo run -p local-history-sidecar -- render-markdown previous-hour <project-root>
cargo run -p local-history-sidecar -- render-markdown current-segment <project-root>
cargo run -p local-history-sidecar -- render-markdown hour <project-root> --hour <YYYY-MM-DDTHH>
cargo run -p local-history-sidecar -- render-markdown segment-at <project-root> --at <rfc3339>
cargo run -p local-history-sidecar -- restore <snapshot-id-or-unique-prefix>
```

## MCP server

`local-history-mcp` is a newline-delimited JSON-RPC stdio server that exposes local-history tools through the Model Context Protocol.

Run it directly:

```bash
cargo run -p local-history-mcp
```

Local usage help:

```bash
cargo run -p local-history-mcp -- --help
```

### Current MCP tools

- `local_history_status`
- `local_history_create_snapshot`
- `local_history_recent_snapshots`
- `local_history_view_snapshot`
- `local_history_restore_snapshot`
- `local_history_prune`

Current tool contract:

- most tools require explicit `project_root`;
- snapshot view and restore work by full `snapshot_id` or any unique snapshot ID prefix;
- all tools accept optional `data_dir` when you want to use a non-default local-history storage base directory;
- `local_history_restore_snapshot` remains safety-first and creates a safety snapshot before writing the live file;
- `local_history_diff_snapshot` does not exist yet because the project still has no dedicated diff surface.

### Zed Agent Panel usage

The Zed Agent Panel uses MCP tools, not extension slash commands.

When the dev extension is installed and `local-history-mcp` is available in `PATH`, the extension registers the `local-history` context server automatically.

Ask the Agent in natural language:

```text
Use local-history to show status for /absolute/path/to/project.
```

If you want to configure the MCP server manually instead, build the MCP binary:

```bash
cargo build -p local-history-mcp
```

Then register it in Zed settings:

```json
{
  "context_servers": {
    "local-history": {
      "command": "/absolute/path/to/zed-local-history/target/debug/local-history-mcp",
      "args": []
    }
  }
}
```

If you prefer packaged binaries over development binaries, point `command` at the installed or unpacked `local-history-mcp` executable instead.

Current release contract:

- platform bundles now include `local-history`, `local-history-sidecar`, `local-history-mcp`, `README.md`, and `LICENSE`;
- fixed-name sidecar-only archives still exist separately for Zed extension bootstrap.

### Example MCP usage

Examples of requests that map well to the current tool surface:

- "Show local-history status for `/absolute/path/to/project`."
- "Create a snapshot of `src/lib.rs` under `/absolute/path/to/project`."
- "List the last 10 local-history snapshots for `/absolute/path/to/project`."
- "Show snapshot `<snapshot-id>`."
- "Restore snapshot `<snapshot-id>`."
- "Prune local history for `/absolute/path/to/project`."

### Current MCP limitations

- no prompts surface is exposed yet;
- no resources surface is exposed yet;
- no diff tool exists yet;
- extension-managed MCP registration currently expects `local-history-mcp` to be available in `PATH`; packaged release bootstrap for the MCP binary still needs live validation.

## Zed usage

### Install as a dev extension

From Zed:

1. open the extensions page;
2. choose `Install Dev Extension`;
3. select `editors/zed`.

Then validate from the repository root:

```bash
cargo run -p xtask -- zed-ci
```

### What the extension does

- resolves `local-history-sidecar` from `PATH` for development workflows;
- otherwise downloads and caches the matching GitHub release asset;
- verifies sidecar version compatibility before use;
- runs focused sidecar commands from slash handlers;
- registers the `local-history` MCP context server for Agent Panel tool use when `local-history-mcp` is available in `PATH`.

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

- run `cargo run -p local-history-sidecar -- status <project-root>`;
- inspect `projects/<project-id>/logs/watcher.log`;
- inspect `projects/<project-id>/logs/watcher-status.json`.

### Watcher not detecting changes

- confirm you are saving to disk, not only changing in-memory buffers;
- confirm the path is not ignored;
- remember that the watcher is polling-based, not event-driven.

### Storage too large

- run `cargo run -p local-history-cli -- status <project-root>`;
- run `cargo run -p local-history-cli -- prune <project-root>`.

### Markdown not updating

- run `cargo run -p local-history-cli -- rebuild-markdown-view <project-root>`;
- confirm raw snapshots actually exist with `recent` or `list`.

### Restore failure

- confirm the snapshot still exists with `show <snapshot-id>` or `recent <project-root>`;
- note that a previously generated Markdown link can outlive the snapshot it references after pruning.

### Unsupported platform in Zed bootstrap

Current sidecar bootstrap contract covers:

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
- the MCP server still needs live validation inside a real Zed Agent `context_servers` setup and does not yet expose prompts, resources, or diff tools.

## Repository layout

```text
zed-local-history/
  README.md
  RHYTHM.md
  Cargo.toml
  rust-toolchain.toml
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
cargo run -p local-history-sidecar -- ensure-daemon .
```

Edit and save `src/lib.rs`, then inspect the newest raw snapshot:

```bash
cargo run -p local-history-cli -- recent .
```

Restore the first item from the recent list:

```bash
cargo run -p local-history-cli -- restore --project-root . --recent 1
```

If the restore was wrong, undo it:

```bash
cargo run -p local-history-cli -- undo-restore .
```

Generate an hour view for browsing:

```bash
cargo run -p local-history-cli -- render-markdown hour . --hour 2026-05-03T14
```

Look up the generated view root:

```bash
cargo run -p local-history-cli -- view-root .
```
