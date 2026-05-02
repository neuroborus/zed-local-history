# GOALS.md

# Local History for Zed

_Last updated: 2026-04-30_

## 1. Project Vision

The goal of this project is to provide a practical, reliable, local-only history layer for Zed users who want JetBrains-style safety: the ability to recover previous file states even when Git history, stash, or uncommitted changes are unavailable.

This project should not replace Git. It solves a different problem:

- accidental destructive edits before commit;
- failed refactoring;
- deleted or overwritten files;
- branch/reset/rebase mistakes;
- generated code or manual edits that were never committed;
- cases where the user expects editor-level local history similar to WebStorm, IntelliJ IDEA, or other JetBrains IDEs.

The first version should prioritize dependable recovery over a perfect IDE-native UI. Zed’s current public extension API should be treated as limited for custom visual UI. The MVP should therefore avoid depending on custom panels, webviews, or tree views inside Zed.

The product should feel simple:

```text
Install the Zed extension
→ the extension installs the native helper automatically
→ the helper watches the project
→ snapshots are created on file saves / disk changes
→ snapshots are visible through Markdown files and CLI commands
→ the user can inspect, diff, and restore previous versions safely
```

The core principle:

```text
The sidecar is the source of truth.
Zed integration is a convenience layer.
Markdown files are a browsable presentation layer.
CLI commands are the reliable recovery interface.
```

## 2. MVP Product Direction

The MVP should follow a simple, robust direction:

```text
Rust sidecar binary
+ Zed extension that downloads and starts it
+ SQLite metadata
+ compressed raw snapshots
+ stable JSON contract
+ filesystem-browsable Markdown snapshot view
+ CLI recovery commands
```

The MVP should not require a browser UI.

The MVP should not require users to manually install Rust, Node.js, npm packages, or system dependencies. The Zed extension should download the correct prebuilt sidecar binary for the user’s platform and start it automatically.

## 3. Non-Goals

Out of scope for the MVP:

- capturing every in-memory buffer edit before the file is saved;
- replacing Git, stash, commits, reflog, or backups;
- cloud sync;
- telemetry;
- browser-based UI as the default experience;
- full JetBrains-style UI;
- a native Zed custom panel if the current Zed extension API does not support it;
- tracking huge generated files by default;
- deep binary diffing;
- destructive squashing of raw snapshots.

The MVP should solve one thing well:

```text
When a user saves a file and later regrets it, the tool should usually be able to recover the previous saved state.
```

## 4. Current Zed Constraints and Design Implications

As of 2026-04-30, Zed extensions are Rust code compiled to WebAssembly and packaged through an `extension.toml` manifest. The public extension surface is primarily useful for language support, debuggers, snippets, themes, icon themes, MCP/context servers, slash commands, and external tooling integration.

The project should not assume that a Zed extension can currently create a VS Code-like custom panel, custom webview, custom tree view, or rich nested UI with buttons and arbitrary layout.

The project should also not assume a stable hook like:

```text
onDidChangeTextDocument(...)
onDidSaveTextDocument(...)
onDidOpenTextDocument(...)
onDidCloseTextDocument(...)
```

Therefore, the architecture must be filesystem-first:

```text
Zed extension
  installs and starts the sidecar
  opens generated Markdown files
  runs focused sidecar commands

Native Rust sidecar
  watches files on disk
  stores raw snapshots
  stores metadata
  generates Markdown views
  exposes CLI and JSON output
  performs safe restore operations
```

If Zed later exposes a visual extension API, the same sidecar JSON contract should be reusable to build a native Zed panel without changing the storage model.

## 5. High-Level Architecture

This should be a monorepo with two main deliverables:

1. A Zed extension.
2. A native Rust sidecar binary.

Recommended repository shape:

```text
zed-local-history/
  README.md
  LICENSE
  agents/
    AGENTS.md
    README.md
    GOALS.md
    DEVELOPMENT_PLAN.md

  crates/
    local-history-core/
      src/
        lib.rs
      Cargo.toml

    local-history-sidecar/
      src/
        main.rs
      Cargo.toml

    local-history-cli/
      src/
        main.rs
      Cargo.toml

  editors/
    zed/
      extension.toml
      Cargo.toml
      src/
        lib.rs
      README.md
      LICENSE

  xtask/
    src/
      main.rs
    Cargo.toml

  .github/
    workflows/
      ci.yml
      release.yml

  rust-toolchain.toml
  Cargo.toml
```

### 5.1 `local-history-core`

Shared library with pure logic:

- project identity;
- file hashing;
- snapshot identity;
- time grouping;
- 10-minute segment calculation;
- ignore rules;
- retention policies;
- SQLite schema helpers;
- path normalization;
- text/binary detection;
- compression helpers;
- diff metadata;
- JSON output models;
- restore safety models;
- error types.

This crate must be independent from Zed.

### 5.2 `local-history-sidecar`

Native long-running process responsible for watching projects and storing snapshots.

Responsibilities:

- watch one or more project roots;
- maintain a cache of last known file contents or hashes;
- create snapshots when files change on disk;
- debounce filesystem events;
- handle atomic editor writes safely;
- avoid duplicate snapshots;
- persist metadata in SQLite;
- store compressed snapshot blobs on disk;
- generate Markdown snapshot views;
- expose commands for status, list, diff, restore, undo restore, prune, and health checks.

The sidecar must work without Zed. Zed is only one possible integration.

### 5.3 `local-history-cli`

User-facing CLI wrapper and recovery interface.

The CLI is not secondary. It is the reliable interface for:

- listing recent snapshots;
- browsing snapshots with pagination;
- inspecting a snapshot;
- opening generated Markdown;
- restoring a snapshot;
- undoing a restore;
- rebuilding the Markdown view;
- exporting JSON.

### 5.4 `editors/zed`

Thin Zed integration.

Responsibilities:

- detect OS/architecture;
- install the matching sidecar binary from GitHub Releases or another stable release host;
- make the binary executable where needed;
- start or verify the sidecar via a short command such as `ensure-daemon`;
- open generated Markdown reports or snapshot view files in Zed;
- expose focused commands where the Zed extension API allows it;
- show clear errors when required capabilities are disabled.

The Zed extension should not be the only way to recover data.

## 6. Sidecar Installation Strategy

The extension should provide an installation experience that feels autonomous:

```text
Install extension from Zed
→ extension detects OS/architecture
→ extension downloads the correct prebuilt sidecar binary
→ extension stores it inside the extension working directory
→ extension marks it executable where needed
→ extension runs `local-history ensure-daemon <project-root>`
```

Expected release assets:

```text
local-history-aarch64-apple-darwin.tar.gz
local-history-x86_64-apple-darwin.tar.gz
local-history-x86_64-unknown-linux-gnu.tar.gz
local-history-x86_64-unknown-linux-musl.tar.gz
local-history-aarch64-unknown-linux-gnu.tar.gz
local-history-x86_64-pc-windows-msvc.zip
local-history-aarch64-pc-windows-msvc.zip
```

The code should be one Rust codebase, but native binaries must be built per OS and architecture.

The extension should support an advanced setting for locked-down environments:

```text
local_history.sidecar_path = "/custom/path/to/local-history"
```

If this setting is provided, the extension should use the user-provided binary instead of downloading one.

## 7. Rust Toolchain Goals

Use modern Rust while keeping compatibility practical.

Recommended baseline as of 2026-04-30:

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.95.0"
components = ["rustfmt", "clippy"]
```

Recommended crate settings:

```toml
[package]
edition = "2024"
rust-version = "1.95"
```

Exception: if the Zed extension WASM crate or the current `zed_extension_api` requires a different compatible setup, only the Zed extension crate may temporarily use the required configuration. The sidecar/core crates should stay on the modern Rust baseline.

All code comments and logs must be written in English.

## 8. Platform Support

MVP support targets:

- Linux x86_64;
- macOS Apple Silicon;
- macOS Intel;
- Windows x86_64.

Follow-up targets:

- Linux arm64;
- Windows arm64.

Recommended Rust crates:

- `notify` for filesystem watching;
- `rusqlite` or another mature SQLite binding for metadata;
- `zstd` for compressed snapshot blobs;
- `ignore` for `.gitignore`-style rules;
- `camino` or careful UTF-8 path handling where useful;
- `tracing` and `tracing-subscriber` for logs;
- `clap` for CLI;
- `serde` and `serde_json` for JSON output;
- `ratatui` or `inquire` for optional interactive CLI mode.

Platform-specific concerns:

- Linux: inotify event bursts, temporary file writes, glibc/musl compatibility;
- macOS: FSEvents batching and delayed events;
- Windows: path normalization, locked files, case-insensitive filesystems, CRLF handling.

## 9. Snapshot Model

The sidecar should snapshot the previous known version when a changed file is observed.

Basic algorithm:

```text
on project start:
  scan tracked files
  cache current content hash and metadata

on filesystem change:
  debounce path
  read new file content
  compare with cached hash

  if content changed:
    store cached previous content as a raw snapshot
    update cache to new content
    update metadata
    update generated Markdown view incrementally
```

This produces a useful history of saved states:

```text
initial file state
save #1 -> previous state is stored
save #2 -> save #1 state is stored
save #3 -> save #2 state is stored
```

The project may later support storing the new version as well, but the MVP should avoid redundant storage unless it is required for a clearer recovery model.

Important edge cases:

- file created;
- file deleted;
- file renamed;
- file moved;
- file replaced through atomic write;
- file temporarily unreadable;
- very large file;
- binary file;
- file excluded by ignore rules;
- file edited by another program while Zed is open;
- generated Markdown view files must not recursively trigger snapshots.

## 10. Storage Layout

Snapshots should be stored outside the project by default to avoid polluting repositories.

Recommended base path:

```text
Linux:   ~/.local/share/local-history/
macOS:   ~/Library/Application Support/local-history/
Windows: %LOCALAPPDATA%\local-history\
```

Recommended internal layout:

```text
local-history/
  projects/
    <project-id>/
      metadata.sqlite
      blobs/
        <hash-prefix>/
          <content-hash>.zst
      view/
        README.md
        2026-04-30/
          14/
            README.md
            14-00__14-10.md
            14-10__14-20.md
            snapshots/
              14-14-28__src_orders_order.service.ts__abc123.md
      logs/
        sidecar.log
```

Project identity should be stable but not leak unnecessary private information:

```text
project-id = hash(canonical_project_root + machine_specific_salt)
```

The metadata database should store:

- project root;
- file path relative to project root;
- snapshot ID;
- content hash;
- file size;
- timestamp;
- reason/event kind;
- original permissions where relevant;
- sidecar version;
- schema version;
- whether the snapshot is a normal snapshot or a safety snapshot created before restore.

## 11. Time Grouping Model

The MVP should use deterministic time grouping.

Primary grouping:

```text
Hour bucket
  10-minute segment
```

Example:

```text
Today
  14:00–15:00
    14:00–14:10
    14:10–14:20
    14:20–14:30
    14:30–14:40
    14:40–14:50
    14:50–15:00
```

The first level groups changes by hour.

The second level splits each hour into six fixed 10-minute segments.

For MVP, “squashing” means UI grouping only. It must not destroy raw snapshots.

Rules:

```text
14:00:00–14:09:59  -> segment 14:00–14:10
14:10:00–14:19:59  -> segment 14:10–14:20
14:20:00–14:29:59  -> segment 14:20–14:30
14:30:00–14:39:59  -> segment 14:30–14:40
14:40:00–14:49:59  -> segment 14:40–14:50
14:50:00–14:59:59  -> segment 14:50–15:00
```

The grouping must be predictable. Future versions may add smarter grouping such as inactivity-based sessions, manual checkpoints, or bulk-change detection, but those features should not be part of the first MVP unless the core model is already stable.

## 12. JSON Contract

JSON output is required. It is the compatibility contract for:

- the CLI;
- the Zed extension;
- generated Markdown;
- future native Zed UI;
- other editor integrations;
- tests;
- scripts.

Query commands should support JSON output through a flag such as:

```text
--json
```

The core JSON shape should represent the MVP grouping explicitly:

```json
{
  "projectRoot": "/path/to/project",
  "timeZone": "Europe/Paris",
  "from": "2026-04-30T14:00:00+02:00",
  "to": "2026-04-30T15:00:00+02:00",
  "hours": [
    {
      "from": "2026-04-30T14:00:00+02:00",
      "to": "2026-04-30T15:00:00+02:00",
      "segments": [
        {
          "from": "2026-04-30T14:10:00+02:00",
          "to": "2026-04-30T14:20:00+02:00",
          "affectedFiles": [
            {
              "path": "src/orders/order.service.ts",
              "snapshotCount": 3,
              "snapshots": [
                {
                  "id": "snapshot-id",
                  "timestamp": "2026-04-30T14:14:28+02:00",
                  "sizeBytes": 12450,
                  "contentHash": "sha256:...",
                  "kind": "normal"
                }
              ]
            }
          ]
        }
      ]
    }
  ]
}
```

The schema may evolve, but the principle should not change:

```text
hour
→ 10-minute segment
→ affected files
→ exact raw snapshots
```

## 13. Filesystem-Browsable Markdown Snapshot View

By default, the sidecar should generate a browsable Markdown representation of local history.

This is the primary MVP UI model:

```text
Local history is visible as files.
The user can open Markdown files in Zed.
The user can browse by hour, 10-minute segment, affected file, and exact snapshot.
```

The generated Markdown view must not be the source of truth. It is a presentation/cache layer generated from SQLite metadata and raw snapshot blobs.

Recommended generated view layout:

```text
<local-history-data-dir>/
  projects/
    <project-id>/
      view/
        README.md
        2026-04-30/
          14/
            README.md
            14-00__14-10.md
            14-10__14-20.md
            14-20__14-30.md
            14-30__14-40.md
            14-40__14-50.md
            14-50__15-00.md
            snapshots/
              14-14-28__src_orders_order.service.ts__abc123.md
              14-18-51__src_history_history.mapper.ts__def456.md
```

The sidecar should generate Markdown for exact snapshots as well.

Each exact snapshot Markdown file should show:

- project;
- original file path;
- timestamp;
- snapshot ID;
- snapshot kind: normal or safety;
- content hash;
- restore command;
- undo-restore command if relevant;
- optional short diff summary;
- optional preview for text files within size limits.

Example exact snapshot Markdown file:

````markdown
# Snapshot `abc123`

File: `src/orders/order.service.ts`
Timestamp: `2026-04-30T14:14:28+02:00`
Kind: `normal`
Content hash: `sha256:...`

## Restore

```sh
local-history restore abc123
```

## Preview

```ts
// truncated preview or full text if below configured limit
```
````

A segment Markdown file should show:

- selected time window;
- affected files;
- snapshot count per file;
- exact snapshot IDs;
- timestamps;
- restore command examples;
- optional short diff summary if cheap to generate.

Example segment Markdown file:

```markdown
# Local History — 14:10–14:20

Project: `/path/to/project`
Window: `2026-04-30T14:10:00+02:00` → `2026-04-30T14:20:00+02:00`

## Affected files

- `src/orders/order.service.ts` — 3 snapshots
- `src/history/history.mapper.ts` — 2 snapshots

## Snapshots

### `src/orders/order.service.ts`

- `14:11:03` — snapshot `abc123`
  - Restore: `local-history restore abc123`
- `14:14:28` — snapshot `def456`
  - Restore: `local-history restore def456`
```

Important rules:

- generated Markdown can be deleted and rebuilt;
- generated Markdown should not be created inside the user project by default;
- project-local Markdown views may be supported later, but must be opt-in and ignored by Git by default;
- generated Markdown must not recursively trigger local-history snapshots;
- raw snapshots and SQLite metadata remain the source of truth.

## 14. On-Demand Markdown Reports

In addition to the filesystem-browsable Markdown view, the sidecar should support on-demand Markdown reports.

Use cases:

```text
Show current hour
Show previous hour
Show selected hour
Show current 10-minute segment
Show selected 10-minute segment
```

The Zed extension should be able to call the sidecar, generate the report, and open the Markdown file inside Zed.

Suggested commands:

```text
local-history render-markdown hour <project-root> --hour <ISO-hour>
local-history render-markdown segment <project-root> --from <ISO-datetime> --to <ISO-datetime>
local-history view-root <project-root>
local-history rebuild-markdown-view <project-root>
```

Expected Zed behavior:

```text
User runs "Local History: Show Current Hour"
→ extension calls sidecar render-markdown hour
→ sidecar writes a temporary or cached Markdown report
→ extension opens the Markdown report in Zed
```

## 15. CLI Recovery Model

The CLI should be the main reliable recovery interface.

It should support both non-interactive commands and an optional interactive mode.

### 15.1 Recent Snapshots

The CLI should allow users to quickly see the latest snapshots.

Example:

```text
local-history recent --limit 10
```

Output should be human-readable and numbered:

```text
Recent snapshots

[1] 2026-04-30 14:58:12  src/orders/order.service.ts        abc123
[2] 2026-04-30 14:55:40  src/history/history.mapper.ts      def456
[3] 2026-04-30 14:51:03  package.json                       ghi789
[4] 2026-04-30 14:47:19  src/main.ts                         jkl012
...

Commands:
  local-history restore 1 --from-last-list
  local-history show 1 --from-last-list
  local-history diff 1 --from-last-list
```

The numbering should be valid for the latest list result.

The CLI may store the last list result in a small session file so that numbered commands work safely:

```text
local-history restore 1 --from-last-list
```

Safety rule:

```text
A number must resolve to the exact snapshot ID from the last displayed list.
If the last list is stale, expired, or from another project, the CLI must refuse or ask for confirmation.
```

### 15.2 Paginated Snapshot Browsing

The CLI should support pagination for larger history lists.

Non-interactive examples:

```text
local-history list --page 1 --page-size 20
local-history list --page 2 --page-size 20
local-history list --file src/orders/order.service.ts --page 1 --page-size 20
local-history list --from 2026-04-30T14:00:00+02:00 --to 2026-04-30T15:00:00+02:00
```

The CLI should make it possible to browse all snapshots without needing a custom Zed UI.

### 15.3 Interactive CLI Mode

The CLI should optionally provide an interactive mode:

```text
local-history browse
local-history restore --select
```

Desired behavior:

```text
Open a paginated list of snapshots
→ navigate with keyboard
→ inspect snapshot metadata
→ open preview/diff
→ restore selected snapshot
→ create safety snapshot before restore
```

Suggested controls:

```text
Up/Down or j/k   Move selection
PageUp/PageDown  Change page
Enter            Show details
D                Show diff
O                Open generated Markdown snapshot
R                Restore selected snapshot
U                Undo last restore
Q                Quit
```

Interactive mode is useful, but it should not be required for automation. Every interactive operation must also have a non-interactive command equivalent.

### 15.4 Snapshot Inspection

The CLI should support:

```text
local-history show <snapshot-id>
local-history diff <snapshot-id> --with-current
local-history open <snapshot-id>
```

Where:

- `show` prints metadata and optional content preview;
- `diff` shows a unified diff between the snapshot and the current file state;
- `open` opens or prints the path to the generated Markdown snapshot file.

### 15.5 Restore Commands

The CLI should support exact restore:

```text
local-history restore <snapshot-id>
```

Numbered restore from last list:

```text
local-history recent --limit 10
local-history restore 3 --from-last-list
```

Interactive restore:

```text
local-history restore --select
```

File-scoped restore:

```text
local-history restore <snapshot-id> --file src/orders/order.service.ts
```

Restore should always target an exact raw snapshot.

The CLI must never restore an aggregated hour bucket or 10-minute segment as if it were a single snapshot.

## 16. Restore Safety and Undo Restore

Before any restore operation changes the working tree, the tool must create a safety snapshot of the current state.

This rule is mandatory.

```text
restore requested
→ read current file content
→ create safety snapshot
→ mark safety snapshot as pre-restore
→ apply selected snapshot
→ print safety snapshot ID
```

Example output:

```text
Restored snapshot abc123 to src/orders/order.service.ts
Created safety snapshot before restore: safety_789xyz

Undo:
  local-history undo-restore
  local-history restore safety_789xyz
```

The safety snapshot must allow the user to return to the state that existed immediately before the restore.

CLI commands:

```text
local-history undo-restore
local-history restore-last-safety
local-history safety-list
```

`undo-restore` should restore the most recent safety snapshot for the current project/file, after creating another safety snapshot of the current state.

The restore chain should remain reversible:

```text
state A
→ restore snapshot B
  safety snapshot A is created
→ undo restore
  safety snapshot B/current is created
  state A is restored
```

For multi-file restore in the future, the sidecar should create a restore transaction:

```text
restore transaction
  target snapshots
  safety snapshots for all affected files
  status: pending / applied / failed / rolled_back
```

MVP may start with single-file restore only, but the storage model should not block future multi-file restore.

## 17. CLI Command Catalog

The MVP CLI should include commands like:

```text
local-history ensure-daemon <project-root>
local-history watch <project-root>
local-history status <project-root>

local-history recent --limit 10
local-history list --page 1 --page-size 20
local-history list --file <relative-path>
local-history list --from <ISO-datetime> --to <ISO-datetime>

local-history history hour <project-root> --hour <ISO-hour>
local-history history segment <project-root> --from <ISO-datetime> --to <ISO-datetime>
local-history files <project-root> --from <ISO-datetime> --to <ISO-datetime>
local-history snapshots <project-root> --file <relative-path> --from <ISO-datetime> --to <ISO-datetime>

local-history show <snapshot-id>
local-history diff <snapshot-id> --with-current
local-history open <snapshot-id>

local-history restore <snapshot-id>
local-history restore <number> --from-last-list
local-history restore --select
local-history undo-restore
local-history restore-last-safety
local-history safety-list

local-history render-markdown hour <project-root> --hour <ISO-hour>
local-history render-markdown segment <project-root> --from <ISO-datetime> --to <ISO-datetime>
local-history view-root <project-root>
local-history rebuild-markdown-view <project-root>

local-history prune <project-root>
```

Most query commands should support:

```text
--json
--page <number>
--page-size <number>
--file <relative-path>
--from <ISO-datetime>
--to <ISO-datetime>
```

## 18. Zed Extension MVP Behavior

The Zed extension should focus on making the sidecar convenient from inside Zed.

Suggested Zed commands:

```text
Local History: Open Snapshot View
Local History: Show Current Hour
Local History: Show Previous Hour
Local History: Show Hour...
Local History: Show Current 10-Minute Segment
Local History: Show Segment...
Local History: Show Recent Snapshots
Local History: Restore Snapshot...
Local History: Start Watcher
Local History: Status
```

Expected behavior:

```text
User runs "Local History: Open Snapshot View"
→ extension asks sidecar for the generated view root
→ extension opens the root README.md or selected Markdown file in Zed
```

```text
User runs "Local History: Show Current Hour"
→ extension calls sidecar render-markdown hour
→ sidecar writes a Markdown report
→ extension opens the report in Zed
```

```text
User runs "Local History: Restore Snapshot..."
→ extension calls sidecar restore <snapshot-id>
→ sidecar creates a safety snapshot first
→ sidecar restores the exact selected snapshot
```

No browser UI should be part of the MVP.

## 19. Privacy and Security

This project stores source code snapshots. That is sensitive.

Default behavior must be conservative:

- no telemetry by default;
- no cloud sync;
- snapshots stored locally;
- clear documentation that secrets may be captured if files are tracked;
- default ignore patterns for common secret and dependency files;
- generated Markdown view stored outside the project by default.

Suggested default ignores:

```text
.git/
node_modules/
target/
dist/
build/
.next/
.nuxt/
.cache/
coverage/
.env
.env.*
*.pem
*.key
*.p12
*.pfx
*.sqlite
*.db
*.log
```

The project should respect:

- `.gitignore`;
- optional `.local-history-ignore`;
- global config;
- size limits.

Open question for MVP:

```text
Should ignored files be protected by local history or ignored by default?
```

The safer first version should respect `.gitignore` by default and allow opt-in overrides.

## 20. Retention Policy

The MVP should include retention from the beginning.

Recommended defaults:

```text
max file size: 1 MiB for text files by default
max snapshots per file: 100
max project storage: 1 GiB
compression: zstd
content deduplication: enabled
```

Possible retention tiers:

```text
last 24 hours: keep all saved versions
last 7 days: keep hourly versions
last 30 days: keep daily versions
older: keep weekly versions or prune
```

For MVP, retention may be simpler:

```text
Keep the newest N snapshots per file.
Keep total project storage below configured limit.
Never delete safety snapshots until they exceed a separate safety retention policy.
```

Safety snapshots should have a clear retention rule. They should not be deleted immediately because they are needed for undo-restore.

## 21. Diff Strategy

MVP diff strategy should be simple:

- show unified diff in CLI;
- optionally include short diff summary in Markdown;
- open exact snapshot Markdown for inspection;
- avoid blocking MVP on native Zed diff UI.

Commands:

```text
local-history diff <snapshot-id> --with-current
local-history diff <snapshot-id> --with <other-snapshot-id>
```

Future Zed integration may open diffs natively if Zed exposes a suitable extension API.

## 22. Development Quality Bar

This project should be treated as infrastructure.

Required early:

- structured logging with `tracing`;
- integration tests for snapshot lifecycle;
- restore safety tests;
- undo-restore tests;
- pagination tests;
- last-list numbered restore tests;
- filesystem watcher tests where practical;
- retention tests;
- path normalization tests;
- Windows path tests;
- CI on Linux, macOS, and Windows;
- release artifacts built in GitHub Actions;
- checksums for release assets;
- basic security review for path traversal and arbitrary command execution.

Recommended CI commands:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

## 23. MVP Scope

MVP should include:

- Rust sidecar;
- Zed extension that installs and starts the sidecar;
- one watched project root;
- snapshot-on-save/disk-change behavior;
- SQLite metadata;
- compressed raw snapshot blobs;
- default ignore rules;
- retention limits;
- two-level time grouping: hour bucket → 10-minute segment;
- stable JSON output for grouped history queries;
- filesystem-browsable generated Markdown snapshot view;
- generated Markdown for each exact snapshot;
- generated Markdown reports for a selected hour;
- generated Markdown reports for a selected 10-minute segment;
- CLI recent snapshots list with numbering;
- CLI pagination for history lists;
- CLI restore by exact snapshot ID;
- CLI restore by number from the last displayed list;
- CLI interactive browse/restore mode if feasible;
- mandatory safety snapshot before every restore;
- CLI undo-restore / restore-last-safety;
- opening generated Markdown files inside Zed through extension commands;
- Linux and macOS support at minimum;
- Windows support if release time allows.

MVP should not require:

- browser UI;
- custom Zed panel;
- custom Zed webview;
- full in-memory edit tracking;
- cloud sync;
- Git integration.

## 24. Future Roadmap

### Phase 1 — Reliable Core

- sidecar daemon;
- snapshot lifecycle;
- SQLite metadata;
- compressed blob storage;
- restore safety;
- undo restore;
- retention;
- JSON output;
- CLI basics.

### Phase 2 — Markdown-First UX

- filesystem-browsable Markdown view;
- exact snapshot Markdown files;
- hour reports;
- 10-minute segment reports;
- rebuildable generated view;
- Zed command to open snapshot view.

### Phase 3 — CLI Recovery UX

- recent snapshots list;
- numbered restore from last list;
- pagination;
- interactive browse mode;
- interactive restore mode;
- better diff output;
- safer restore transaction metadata.

### Phase 4 — Zed Integration

- automatic sidecar install;
- platform-specific asset selection;
- start/status commands;
- open current hour report;
- open selected segment report;
- restore snapshot command;
- better error reporting.

### Phase 5 — Advanced History

- rename tracking;
- smarter retention;
- manual checkpoints;
- operation/session grouping;
- bulk-change detection;
- Git commit-aware labels;
- multi-file restore transactions.

### Phase 6 — Native UI if Zed Allows It

If Zed exposes visual extension APIs, add a native UI:

```text
Hour
  10-minute segment
    affected files
      snapshots
```

The sidecar JSON contract should make this possible without changing storage.

## 25. Open Questions to Validate Early

- Can the Zed extension reliably start short helper commands with `process::Command` on all supported platforms?
- What is the cleanest way to start a long-running sidecar without blocking the extension process?
- Should the sidecar expose only CLI commands for MVP, or also a local IPC endpoint?
- Should generated Markdown include full content previews or only metadata and links?
- What size limit should be used for Markdown previews?
- How should numbered restore from the last list expire safely?
- Should interactive CLI use a full TUI or a simpler prompt-based flow?
- Should `.gitignore` be respected by default, or should local history intentionally protect ignored files unless excluded separately?
- How strict will Zed extension review be for a non-LSP native helper downloaded by the extension?
- Should the extension support a user-provided sidecar binary path from the beginning?

## 26. Recommended Initial Implementation Order

Start with the editor-independent sidecar and CLI.

Recommended order:

```text
1. Implement local-history-core.
2. Implement raw snapshot storage.
3. Implement SQLite metadata.
4. Implement sidecar file watcher.
5. Implement recent/list/show/diff CLI commands.
6. Implement restore with mandatory pre-restore safety snapshots.
7. Implement undo-restore.
8. Implement JSON output.
9. Implement hour and 10-minute grouping.
10. Implement generated Markdown snapshot view.
11. Implement Markdown reports for hour and segment.
12. Implement CLI pagination.
13. Implement numbered restore from last list.
14. Implement optional interactive CLI browse/restore mode.
15. Release native binaries.
16. Implement Zed extension installer/start/status/open-report integration.
```

This order avoids getting blocked by Zed UI limitations and produces a useful recovery tool early.

## 27. Success Criteria

The project is successful when:

- a user can install the Zed extension without manually installing Rust, Node.js, or system tools;
- the correct sidecar binary is installed automatically;
- saved file states are captured reliably;
- deleted or overwritten file content can be restored;
- every restore creates a safety snapshot first;
- undo-restore works for the latest restore;
- recent snapshots can be listed and restored quickly from CLI;
- long snapshot lists can be browsed with pagination;
- Markdown snapshot files can be opened directly in Zed;
- hour and 10-minute segment reports are generated correctly;
- JSON output is stable enough for future clients;
- storage does not grow without bounds;
- secrets and generated files are not captured accidentally by default as much as reasonably possible;
- the tool works even if Git history is missing or damaged;
- the sidecar remains useful outside Zed;
- Zed integration improves the experience but is not the only way to recover data.
