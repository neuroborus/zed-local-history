# DEVELOPMENT_PLAN.md

# Local History for Zed — Development Plan

_Last updated: 2026-05-02_

## Goal

Build a local-history tool for Zed that can recover previous saved states of files even when Git history, stash, or uncommitted changes are unavailable.

The MVP should be useful without a custom Zed UI panel.

The initial product surface is:

```text
Rust sidecar
+ CLI recovery interface
+ stable JSON output
+ generated Markdown history view
+ thin Zed extension integration
+ additive MCP stdio server
```

The Zed extension should install/start the sidecar and make generated Markdown history easy to open inside Zed.

The architecture should also leave room for a local MCP server so Zed's Agent Panel can call local-history tools directly without replacing the CLI workflow. This is an additive integration concern, not a numbered MVP stage.

## Additional Architecture Note — MCP Surface

Alongside the MVP surfaces, the repository should leave room for an MCP server adapter that exposes local-history tools to agent clients such as the Zed Agent Panel.

This is not a numbered roadmap stage and not a replacement for the CLI. It is an additional integration surface.

If added, the intended shape is:

```text
crates/
  local-history-core/
  local-history-cli/
  local-history-mcp/
```

The MCP layer should stay thin and adapt protocol calls to existing core or sidecar behavior.

First implemented MCP tool slice:

- `local_history_status`
- `local_history_create_snapshot`
- `local_history_recent_snapshots`
- `local_history_view_snapshot`
- `local_history_restore_snapshot`
- `local_history_prune`

Future MCP additions once dedicated product surfaces exist:

- `local_history_diff_snapshot`

If exposed through Zed, it may be connected either:

- directly through user `context_servers` settings; or
- through extension-managed registration in `extension.toml`.

## Final MVP Result

At the end of MVP, the user should be able to:

```text
1. Install the Zed extension.
2. Let the extension install the native sidecar automatically.
3. Start watching the current project.
4. Edit and save files normally.
5. Have previous saved states stored as local snapshots.
6. List recent snapshots through CLI.
7. Browse snapshots with pagination.
8. Generate Markdown reports by hour or 10-minute segment.
9. Open generated Markdown history inside Zed.
10. Restore an exact snapshot.
11. Automatically create a safety snapshot before restore.
12. Undo the restore if needed.
```

## Development Roadmap

```text
Stage 1 — Validate the architecture
Stage 2 — Set up repository and Rust workspace
Stage 3 — Implement core storage
Stage 4 — Implement CLI recovery without watcher
Stage 5 — Add restore safety and undo
Stage 6 — Add pagination, filtering, JSON, and interactive CLI
Stage 7 — Implement file watcher and sidecar daemon
Stage 8 — Implement hour / 10-minute grouping
Stage 9 — Generate Markdown history views
Stage 10 — Integrate with Zed extension
Stage 11 — Package cross-platform releases
Stage 12 — Harden, document, and accept MVP
Stage 13 — Post-MVP improvements
```

---

# Stage 1 — Validate the Architecture

## Goal

Confirm that the planned architecture works with the current Zed extension model and native sidecar approach.

## Tasks

### 1.1 Validate Zed extension capabilities

Check whether the extension can:

- detect OS and architecture;
- download a file;
- make a downloaded binary executable;
- run a short external command;
- resolve the current project/worktree root;
- open or expose a generated Markdown file in a usable way.

### 1.2 Validate sidecar startup model

The expected model is:

```text
Zed extension
→ local-history ensure-daemon <project-root>
→ sidecar starts watcher if needed
→ command returns quickly
```

The extension must not directly block on a long-running process.

### 1.3 Validate filesystem watching

Create a minimal Rust prototype with file watching.

Test:

- normal save;
- repeated save;
- atomic write;
- delete;
- rename;
- bulk file change;
- temporary unreadable file.

### 1.4 Validate Markdown opening workflow

Confirm the simplest way for Zed to let the user open generated Markdown.

Possible acceptable outcomes:

- extension opens generated Markdown directly;
- extension reveals or prints the generated Markdown path;
- extension opens the generated view root;
- slash-command output points to the generated file.

## Expected Result

A small prototype proves that:

- Zed can invoke the sidecar;
- the sidecar can return JSON;
- the sidecar can generate Markdown;
- the generated Markdown can be opened or located from Zed.

## Acceptance Criteria

- A minimal Zed extension can execute a sidecar command.
- A minimal sidecar command returns valid JSON.
- A minimal sidecar command writes a Markdown file.
- The generated Markdown file can be opened or located from Zed.
- Any Zed API limitation is documented before deeper implementation starts.

---

# Stage 2 — Set Up Repository and Rust Workspace

## Goal

Create a clean monorepo foundation.

## Tasks

### 2.1 Create repository structure

```text
zed-local-history/
  README.md
  LICENSE
  Cargo.toml
  rust-toolchain.toml
  agents/
    AGENTS.md
    README.md
    GOALS.md
    DEVELOPMENT_PLAN.md

  crates/
    local-history-core/
    local-history-sidecar/
    local-history-cli/
    local-history-mcp/

  editors/
    zed/

  xtask/

  .github/
    workflows/
      ci.yml
      release.yml
```

### 2.2 Configure Rust workspace

Use Rust 2024 edition where possible.

Recommended baseline:

```toml
[workspace]
resolver = "3"
members = [
  "crates/local-history-core",
  "crates/local-history-sidecar",
  "crates/local-history-cli",
  "crates/local-history-mcp",
  "xtask"
]
```

## Additional Implementation Note — MCP Server

The MCP server is not a numbered stage, but when implemented it should follow this order:

1. Create `crates/local-history-mcp` as a stdio JSON-RPC server.
2. Support at least:
   - `initialize`
   - `ping`
   - `tools/list`
   - `tools/call`
3. Expose the first stable tool slice:
   - `local_history_status`
   - `local_history_create_snapshot`
   - `local_history_recent_snapshots`
   - `local_history_view_snapshot`
   - `local_history_restore_snapshot`
   - `local_history_prune`
4. Keep handlers thin by adapting to `local-history-core` or existing sidecar-facing behavior.
5. Return both human-readable text content and stable structured JSON content in tool results.
6. Keep direct Zed `context_servers` setup as a supported fallback.
7. When the Zed extension registers the MCP server automatically, treat any `PATH` lookup for `local-history-mcp` as a development-only shortcut. Production UX must resolve, download, cache, and launch the MCP binary through the same release-bootstrap model used for the sidecar.

Acceptance for this additive slice:

- The server completes MCP initialization over stdio.
- `tools/list` returns the documented tool set.
- `tools/call` works for status, manual snapshot creation, recent listing, snapshot view, restore, and prune.
- Restore remains safety-first through the MCP path.

Recommended package defaults:

```toml
edition = "2024"
rust-version = "1.95"
```

If the Zed extension crate requires different compatibility, isolate that under `editors/zed`.

### 2.3 Add base tooling

Add:

- `clap` for CLI;
- `tracing` for logs;
- `thiserror` or equivalent for errors;
- `serde` / `serde_json` for JSON;
- `rusqlite` or equivalent for SQLite;
- `notify` for file watching;
- `ignore` for `.gitignore`-style rules;
- `zstd` or equivalent for compression.

### 2.4 Add CI

CI should run:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

## Expected Result

The project has a stable structure, workspace, and CI baseline.

## Acceptance Criteria

- Repository structure matches the planned monorepo layout.
- `cargo fmt --all --check` passes.
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- `cargo test --workspace` passes.
- CI runs on pull requests.
- Code comments and logs are in English.

---

# Stage 3 — Implement Core Storage

## Goal

Implement the data model and storage layer before watching files.

## Tasks

### 3.1 Define core entities

Required entities:

- project;
- tracked file;
- raw snapshot;
- safety snapshot;
- content blob;
- restore operation;
- hour bucket;
- 10-minute segment;
- generated Markdown view entry.

### 3.2 Define project identity

Use stable but privacy-aware project identity.

Recommended:

```text
project_id = hash(canonical_project_root + machine_specific_salt)
```

### 3.3 Implement local data directory

Default locations:

```text
Linux:   ~/.local/share/local-history/
macOS:   ~/Library/Application Support/local-history/
Windows: %LOCALAPPDATA%\local-history\
```

Project storage:

```text
projects/
  <project-id>/
    metadata.sqlite
    blobs/
    view/
    logs/
```

### 3.4 Implement SQLite schema

Store:

- projects;
- files;
- snapshots;
- safety snapshots;
- restore operations;
- content blobs;
- generated view metadata.

### 3.5 Implement content-addressed blob storage

Requirements:

- hash content before storing;
- deduplicate identical content;
- compress content blobs;
- store metadata separately;
- read exact content back by snapshot ID.

### 3.6 Implement ignore and size rules

Default ignored paths/patterns:

```text
.git/
node_modules/
target/
dist/
build/
.next/
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

Support:

- `.gitignore`;
- optional `.local-history-ignore`;
- max file size;
- text/binary detection.

## Expected Result

The core crate can store, query, and restore snapshot content without a watcher.

## Acceptance Criteria

- A snapshot can be stored and read back exactly.
- Duplicate content is deduplicated.
- Snapshot metadata is stored in SQLite.
- Blob content is compressed.
- Ignored files are skipped.
- Large files are skipped or handled safely.
- Unit tests cover hashing, storage, path normalization, ignore rules, and blob retrieval.

---

# Stage 4 — Implement CLI Recovery Without Watcher

## Goal

Make the storage layer usable through CLI before implementing the watcher.

## Tasks

### 4.1 Add manual snapshot command

```text
local-history snapshot <project-root> --file <relative-path>
```

This is useful for testing storage and restore before watcher support exists.

### 4.2 Add show command

```text
local-history show <snapshot-id>
```

It should print metadata and optionally content preview.

### 4.3 Add recent snapshots command

```text
local-history recent <project-root> --limit 10
```

Default output should be numbered:

```text
Latest snapshots

[1] 2026-05-02 14:18:51  src/orders/order.service.ts        abc123
[2] 2026-05-02 14:14:28  src/history/history.mapper.ts      def456
[3] 2026-05-02 14:11:03  src/orders/order.service.ts        ghi789
```

### 4.4 Add basic restore by snapshot ID

```text
local-history restore <snapshot-id>
```

At this stage, restore may be simple, but it must be replaced by safety-first restore in Stage 5 before MVP.

## Expected Result

A developer can manually create snapshots, list them, inspect them, and perform a basic restore.

## Acceptance Criteria

- Manual snapshot command works.
- `recent --limit 10` shows numbered snapshots.
- `show <snapshot-id>` displays useful metadata.
- Basic restore by snapshot ID works.
- Commands return clear errors for missing files or missing snapshots.

---

# Stage 5 — Add Restore Safety and Undo

## Goal

Guarantee that restore operations are reversible.

## Tasks

### 5.1 Create safety snapshot before every restore

Before any restore:

```text
read current file state
store it as safety snapshot
perform restore
record restore operation
```

This is mandatory.

### 5.2 Record restore operation

Store:

- restored snapshot ID;
- affected file;
- safety snapshot ID;
- timestamp;
- previous content hash;
- restored content hash.

### 5.3 Add undo restore

```text
local-history undo-restore
```

This should restore the latest safety snapshot created by the last restore operation.

### 5.4 Add restore-last-safety

```text
local-history restore-last-safety
```

This is an explicit escape hatch.

### 5.5 Add safety snapshot list

```text
local-history safety-list <project-root>
```

The user should be able to inspect safety snapshots.

### 5.6 Add restore by number from latest recent list

After `recent`, users should be able to restore by list number.

Possible command:

```text
local-history restore --recent 1
```

This should restore snapshot `[1]` from the last `recent` result for that project.

The command must still create a new safety snapshot before restore.

## Expected Result

Restore becomes safe and reversible.

## Acceptance Criteria

- Every restore creates a safety snapshot first.
- Restore operation is recorded.
- `undo-restore` restores the previous state.
- `restore-last-safety` works.
- `safety-list` shows safety snapshots.
- Restore by recent-list number works.
- No restore path silently destroys current content.

---

# Stage 6 — Add Pagination, Filtering, JSON, and Interactive CLI

## Goal

Make CLI usable for real browsing and recovery.

## Tasks

### 6.1 Add paginated listing

```text
local-history list <project-root> --page 1 --page-size 20
local-history list <project-root> --page 2 --page-size 20
```

### 6.2 Add filters

Support:

```text
--file <relative-path>
--from <datetime>
--to <datetime>
--hour <ISO-hour>
--limit <n>
```

### 6.3 Add JSON output

Every query command should support:

```text
--json
```

Required for:

- `status`;
- `recent`;
- `list`;
- `show`;
- `history hour`;
- `history segment`;
- `files`;
- `snapshots`;
- `safety-list`.

### 6.4 Add basic interactive browse mode

```text
local-history browse <project-root>
```

Minimum behavior:

- show paginated snapshots;
- next page;
- previous page;
- select snapshot by number;
- preview metadata;
- restore selected snapshot with confirmation;
- always create safety snapshot before restore.

This can be prompt-based. It does not need to be a full TUI for MVP.

### 6.5 Make snapshot IDs ergonomic without weakening identity

Keep stored snapshot identity opaque and stable:

- internal snapshot IDs remain opaque hash-like identifiers;
- do not make timestamp, path, or display formatting part of the storage identity contract;
- keep timestamp and path as separate metadata fields in human, JSON, Markdown, and MCP output.

Improve human recovery ergonomics:

- show timestamp, path, list number, and a 12-character snapshot ID prefix in human tables;
- keep full snapshot IDs available in `--json`, Markdown detail pages, logs, and MCP structured output;
- let `restore` and `show` accept either a full snapshot ID or a unique snapshot ID prefix;
- if a prefix is ambiguous, fail with a clear message and suggest longer matching prefixes;
- keep `restore --project-root <path> --recent <index>` as the fastest fresh-list recovery path.

The goal is the familiar Git/Docker-style workflow: short prefixes are convenient for humans, while full opaque IDs remain the durable machine contract.

## Expected Result

A user can browse and recover snapshots without knowing exact snapshot IDs.

## Acceptance Criteria

- Paginated listing works.
- Filtering by file and time range works.
- Query commands support `--json`.
- Human tables use compact ID prefixes without making those prefixes dead-end values.
- `show` and `restore` accept unique snapshot ID prefixes and report ambiguity clearly.
- Interactive browse mode supports page navigation.
- Interactive browse mode supports selecting a snapshot.
- Interactive restore asks for confirmation.
- Interactive restore creates safety snapshot first.

---

# Stage 7 — Implement File Watcher and Sidecar Daemon

## Goal

Automatically create snapshots on saved file changes.

## Tasks

### 7.1 Implement initial project scan

On watcher start:

- scan project files;
- apply ignore rules;
- cache current file state;
- do not snapshot every file immediately by default.

### 7.2 Implement snapshot-on-change

On file change:

```text
read new content
compare with cached previous state
if changed:
  store previous known state as raw snapshot
  update cache to new state
```

The previous state is stored because this is what the user usually wants after a bad save.

### 7.3 Add debouncing

Avoid duplicate snapshots from noisy filesystem events.

### 7.4 Handle atomic writes

Support common editor write patterns:

- temp file write;
- rename over original file;
- delete + create;
- rapid write bursts.

### 7.5 Handle delete

If a tracked file is deleted, snapshot the previous known state.

### 7.6 Implement daemon commands

```text
local-history watch <project-root>
local-history ensure-daemon <project-root>
local-history status <project-root>
```

`ensure-daemon` should return quickly after verifying or starting the watcher.

## Expected Result

The sidecar can watch a project and automatically store previous saved states.

## Acceptance Criteria

- Saving a tracked file creates a recoverable previous-state snapshot.
- Duplicate events do not create duplicate snapshots.
- Deletes create recoverable snapshots.
- Ignored files are not snapshotted.
- `ensure-daemon` starts or verifies watcher process.
- `status` reports watcher state.

---

# Stage 8 — Implement Hour and 10-Minute Grouping

## Goal

Implement the MVP history presentation model.

## Tasks

### 8.1 Group snapshots by hour

```text
local-history history hour <project-root> --hour <ISO-hour>
```

### 8.2 Split each hour into six fixed 10-minute segments

```text
14:00–14:10
14:10–14:20
14:20–14:30
14:30–14:40
14:40–14:50
14:50–15:00
```

### 8.3 Return affected files per selected window

For an hour or segment, return:

- affected files;
- snapshot count per file;
- snapshot IDs;
- timestamps.

### 8.4 Preserve raw snapshot precision

Grouping must never replace exact snapshots.

Restore must always target a specific raw snapshot.

## Expected Result

The system can answer: “What changed in this hour?” and “What changed in this 10-minute segment?”

## Acceptance Criteria

- Hour query works.
- Segment query works.
- Each hour has six fixed 10-minute segments.
- Affected files are listed per window.
- Exact snapshot IDs are included.
- Grouping does not destroy or merge raw snapshots.

---

# Stage 9 — Generate Markdown History Views

## Goal

Provide filesystem-first UI without a custom Zed panel.

## Tasks

### 9.1 Generate Markdown for selected hour

```text
local-history render-markdown hour <project-root> --hour <ISO-hour>
```

### 9.2 Generate Markdown for selected 10-minute segment

```text
local-history render-markdown segment <project-root> --from <ISO-datetime> --to <ISO-datetime>
```

### 9.3 Generate filesystem-browsable Markdown view

Recommended layout:

```text
projects/
  <project-id>/
    view/
      README.md
      2026-05-02/
        14/
          README.md
          14-00__14-10.md
          14-10__14-20.md
          snapshots/
            14-14-28__src_orders_order.service.ts__abc123.md
```

### 9.4 Generate exact snapshot Markdown files

Each snapshot Markdown file should include:

- snapshot ID;
- original file path;
- timestamp;
- content hash;
- restore command;
- optional preview for text files.

### 9.5 Add view commands

```text
local-history view-root <project-root>
local-history rebuild-markdown-view <project-root>
```

## Expected Result

The user can browse local history through normal Markdown files.

## Acceptance Criteria

- Hour Markdown report is generated.
- Segment Markdown report is generated.
- Filesystem Markdown view exists.
- Exact snapshot Markdown files exist.
- Markdown view can be deleted and rebuilt.
- Markdown generation does not trigger recursive snapshots.
- Markdown restore examples point to valid commands.

---

# Stage 10 — Integrate with Zed Extension

## Goal

Make the sidecar convenient to use from Zed.

## Tasks

### 10.1 Create Zed extension package

```text
editors/
  zed/
    extension.toml
    Cargo.toml
    src/lib.rs
    README.md
    LICENSE
```

### 10.2 Detect platform

The extension should detect OS and architecture to select the correct sidecar asset.

### 10.3 Download sidecar

The extension should:

- download the matching binary;
- store it in an extension-managed location;
- make it executable where needed;
- handle errors clearly.

### 10.4 Start sidecar

Call:

```text
local-history ensure-daemon <project-root>
```

### 10.5 Add Zed commands

Suggested commands:

```text
Local History: Open Snapshot View
Local History: Show Current Hour
Local History: Show Previous Hour
Local History: Show Hour...
Local History: Show Current 10-Minute Segment
Local History: Show Segment...
Local History: Restore Snapshot...
Local History: Start Watcher
Local History: Status
```

### 10.6 Open generated Markdown

Expected flow:

```text
user runs command
→ extension calls sidecar
→ sidecar generates or returns Markdown path
→ extension opens or reveals Markdown file
```

### 10.7 Restore through sidecar

Restore from Zed must call the sidecar.

The extension must not implement restore logic itself.

### 10.8 Keep MCP registration optional

If the project adds an MCP server, the extension may register it through `context_servers.*` in `extension.toml` and return its startup command from the extension API.

That route should remain additive. The MVP extension must not depend on MCP for basic recovery.

If the extension registers MCP for Agent Panel use, the production path must not require users to put `local-history-mcp` in `PATH`. The extension should:

- resolve a development `local-history-mcp` from `PATH` only for local/dev installs;
- otherwise download the matching release asset;
- cache it in an extension-managed location;
- make it executable where needed;
- verify version compatibility before launching;
- produce clear errors when no supported MCP binary is available.

This should mirror the sidecar bootstrap behavior closely enough that sidecar and MCP binary release contracts stay aligned.

## Expected Result

A Zed user can install the extension, start watching, open history Markdown, and restore snapshots.

## Acceptance Criteria

- Extension installs or locates sidecar.
- Extension starts or verifies sidecar.
- Extension can open snapshot view or generated report.
- Extension can show status.
- Extension can invoke restore by snapshot ID.
- If Agent Panel MCP registration is enabled, the extension can start the MCP server without requiring manual `PATH` setup in production.
- Errors are clear when required capabilities are unavailable.
- User does not need to manually install Rust, Node.js, or system dependencies.

---

# Stage 11 — Package Cross-Platform Releases

## Goal

Publish real binaries and make installation reliable.

## Tasks

### 11.1 Build release matrix

Target assets:

```text
aarch64-apple-darwin
x86_64-apple-darwin
x86_64-unknown-linux-gnu
x86_64-unknown-linux-musl
aarch64-unknown-linux-gnu
x86_64-pc-windows-msvc
aarch64-pc-windows-msvc
```

MVP can start with fewer targets, but Linux x86_64 and macOS Apple Silicon should be prioritized.

### 11.2 Generate checksums

Generate checksums for every release asset.

### 11.3 Add sidecar version compatibility

Track:

- extension version;
- sidecar version;
- MCP binary version;
- minimum compatible sidecar version.
- minimum compatible MCP binary version.

### 11.3.1 Add MCP binary bootstrap parity

The Zed extension's MCP registration must not depend on `local-history-mcp` being manually installed or present in `PATH` for normal users.

Implement release bootstrap parity with the sidecar:

- publish fixed-name MCP-only archives for every supported platform;
- map Zed OS/architecture to the correct MCP archive;
- download and cache the matching MCP binary in the extension work directory;
- mark the binary executable on Unix platforms;
- probe `local-history-mcp --version` or an equivalent machine-readable version command;
- fall back from incompatible or missing `PATH` binaries to the cached/downloaded release binary;
- report unsupported platforms and missing release assets clearly.

Development installs may still prefer a `PATH` binary to support local iteration.

### 11.4 Define update behavior

Decide whether the extension:

- downloads sidecar once;
- downloads MCP binary once;
- updates sidecar automatically;
- updates MCP binary automatically;
- checks sidecar version on startup;
- checks MCP binary version before Agent Panel launch;
- supports user-provided binary path.

### 11.5 Prepare Zed extension submission

Ensure marketplace requirements are met.

## Expected Result

The project can publish installable artifacts for supported platforms.

## Acceptance Criteria

- Release workflow builds native binaries.
- Artifacts include checksums.
- Extension selects correct asset.
- Extension selects correct MCP asset when Agent Panel MCP registration is enabled.
- Unsupported platforms show clear error.
- Sidecar compatibility is checked.
- MCP binary compatibility is checked.
- Zed extension is ready for submission.

---

# Stage 12 — Harden, Document, and Accept MVP

## Goal

Make MVP safe enough for real technical users.

## Tasks

### 12.1 Add retention policy

Defaults should include:

- max snapshots per file;
- max project storage size;
- max file size;
- time-based pruning.

### 12.2 Add prune command

```text
local-history prune <project-root>
```

Pruning must preserve metadata integrity.

### 12.3 Add restore safety tests

Test:

- restore creates safety snapshot;
- undo restore works;
- repeated restore chain is recoverable;
- restoring missing/deleted files works.

### 12.4 Add privacy documentation

Document:

- what is stored;
- where it is stored;
- how to delete local history;
- how to configure ignores;
- how secrets may be captured if not ignored.

### 12.5 Add troubleshooting documentation

Cover:

- sidecar not starting;
- extension capabilities disabled;
- unsupported platform;
- watcher not detecting changes;
- storage too large;
- Markdown not updating;
- restore failure.

### 12.6 Add end-to-end tests

Test full workflow:

```text
create temp project
start watcher
edit file
save file
list recent snapshots
restore snapshot
undo restore
generate Markdown
rebuild Markdown view
```

## Expected Result

The MVP is usable, documented, safe, and test-covered.

## Acceptance Criteria

- User can install and run the tool.
- User can save a file and see a snapshot.
- User can list latest 10 snapshots.
- User can paginate snapshots.
- User can restore by ID.
- User can restore by recent-list number.
- Restore always creates a safety snapshot.
- User can undo the last restore.
- User can generate Markdown by hour.
- User can generate Markdown by 10-minute segment.
- User can browse generated Markdown files.
- Zed extension starts sidecar and opens generated Markdown.
- Retention limits exist.
- Ignore rules protect common generated and secret files.
- Documentation explains limitations clearly.

---

# Stage 13 — Post-MVP Improvements

## Goal

Improve UX and intelligence after the MVP is reliable.

## Possible Improvements

### 13.1 Smarter grouping

Add optional grouping by:

- inactivity sessions;
- bulk operations;
- manual checkpoints;
- VCS operations;
- formatting/refactoring bursts.

### 13.2 Better diff support

Add:

- unified diff in CLI;
- generated diff Markdown;
- temporary files for manual comparison;
- native side-by-side diff if Zed exposes a suitable API.

### 13.3 Native Zed UI if supported

If Zed later exposes visual extension APIs, add:

```text
Local History panel
  hour
    10-minute segment
      affected files
        snapshots
```

### 13.4 Manual checkpoints

```text
local-history checkpoint "Before BTC Direct refactor"
```

### 13.5 More editor integrations

Because the sidecar is editor-independent, support can be added for:

- VS Code;
- JetBrains external tools;
- Neovim;
- standalone TUI.

---

# MVP Acceptance Checklist

## Core

- [x] Project identity is stable.
- [x] Snapshot storage is content-addressed.
- [x] SQLite metadata is persisted.
- [x] Blobs are compressed.
- [x] Ignored files are skipped.
- [x] Large files are skipped or handled safely.

## Watcher

- [x] Initial scan works.
- [x] Save/change detection works.
- [x] Atomic writes are handled.
- [x] Duplicate events are debounced.
- [x] Delete snapshots are recoverable.
- [x] Daemon status is available.

## CLI

- [x] `recent --limit 10` works.
- [x] Recent list is numbered.
- [x] Restore by snapshot ID works.
- [x] Restore/show by unique snapshot ID prefix works.
- [x] Restore by recent-list number works.
- [x] Pagination works.
- [x] Filters by file/time work.
- [x] Basic interactive browse mode works.
- [x] JSON output is available for query commands.

## Restore Safety

- [x] Safety snapshot is created before every restore.
- [x] Restore operation is recorded.
- [x] Undo restore works.
- [x] Safety snapshots are visible through CLI.
- [x] Restore never silently destroys current state.

## Markdown

- [x] Hour report generation works.
- [x] 10-minute segment report generation works.
- [x] Filesystem-browsable Markdown view exists.
- [x] Exact snapshot Markdown files exist.
- [x] Markdown view can be rebuilt.
- [x] Markdown generation does not trigger recursive snapshots.

## Zed Extension

- [x] Extension can install/download sidecar.
- [x] Extension can start sidecar.
- [x] Extension can show status.
- [x] Extension can open or reveal generated Markdown in the currently supported API shape.
- [x] Extension can request restore by snapshot ID.
- [x] Clear errors are shown when capabilities are missing.

## Release

- [x] CI passes.
- [ ] Release artifacts are built on a real tagged run.
- [ ] Checksums are generated and verified on a real tagged run.
- [x] Platform compatibility is documented.
- [x] Installation flow is documented.

---

# Recommended Implementation Order

Use this as the actual execution order:

```text
1. Validate Zed extension → sidecar command execution.
2. Create monorepo and Rust workspace.
3. Implement core snapshot storage.
4. Implement manual CLI snapshot/list/show/restore.
5. Add safety snapshot before restore.
6. Add undo restore.
7. Add recent numbered list and restore by number.
8. Add pagination, filters, JSON output, and basic interactive browse.
9. Implement file watcher and sidecar daemon.
10. Add hour and 10-minute grouping.
11. Generate Markdown reports and filesystem-browsable view.
12. Implement Zed extension sidecar install/start/status.
13. Add Zed commands to open generated Markdown.
14. Add release pipeline.
15. Add retention, docs, tests, and MVP acceptance pass.
```

This order keeps the project useful early through CLI and avoids blocking the MVP on advanced Zed UI APIs.

---

# External Validation Plan

These validations are intentionally outside local unit tests and local CI. They must be run against real packaging, a real Zed install, and a real repository workflow.

## 1. Tagged Release Validation

### Goal

Verify that the GitHub Release path works end-to-end, not just as static YAML.

### Steps

1. Push a real test tag from a clean commit.
2. Wait for `.github/workflows/release.yml` to complete.
3. Verify that the GitHub Release contains:
   - user-facing platform bundles;
   - fixed-name sidecar bootstrap archives;
   - `SHA256SUMS.txt`.
4. Download at least one archive and verify its checksum manually.
5. Confirm archive contents match the documented contract.

### Acceptance

- Tagged workflow succeeds.
- Assets are published to the Release, not only to workflow artifacts.
- Checksums match downloaded files.

## 2. Native Install Validation

### Goal

Confirm that a user can use the native surfaces without local repo-specific assumptions.

### Steps

1. On a fresh machine or clean shell profile, acquire a released binary bundle.
2. Run:
   - `local-history --help`
   - `local-history-sidecar health`
   - `local-history-sidecar version`
3. Start a watcher on a sample project.
4. Save a file and confirm a raw snapshot appears.
5. Restore a snapshot and then undo the restore.
6. Generate and inspect Markdown output.

### Acceptance

- Native binaries run outside the development workspace.
- Watcher, restore, undo, and Markdown flows all work from released binaries.

## 3. Live Zed Extension Validation

### Goal

Validate the real Zed user path instead of only compile-time extension checks.

### Steps

1. Install the extension in Zed as a dev extension or packaged extension.
2. Open a real worktree.
3. Run:
   - `/local-history-status`
   - `/local-history-start-watcher`
   - `/local-history-current-hour`
   - `/local-history-current-segment`
   - `/local-history-restore <snapshot-id>`
4. Confirm sidecar bootstrap behavior:
   - dev `PATH` binary path;
   - cached release asset path;
   - incompatible `PATH` binary fallback.
5. Confirm the returned Markdown paths are usable in real editor workflow.

### Acceptance

- Extension can resolve or download the sidecar.
- Slash commands execute correctly inside a real worktree.
- Restore works through the extension path.
- Error messages are understandable when capabilities are missing.

## 4. Real Project Watcher Validation

### Goal

Exercise watcher behavior on real editing patterns instead of synthetic unit-test-only cases.

### Steps

1. Use a non-trivial project with nested directories.
2. Validate:
   - normal save;
   - repeated save without content change;
   - atomic replace save;
   - delete;
   - rename through delete-and-create behavior;
   - ignored path updates;
   - large file updates above the size cap.
3. Confirm raw snapshots match the previous on-disk state, not the new state.

### Acceptance

- The watcher captures previous contents correctly.
- Unchanged saves do not create noise.
- Ignore and size-limit rules behave as documented.

## 5. Recovery Safety Validation

### Goal

Validate recovery trust on a real repository with realistic operator behavior.

### Steps

1. Restore by snapshot ID.
2. Restore by recent-list number.
3. Confirm safety snapshot creation before each restore.
4. Undo the most recent restore.
5. Restore the newest safety snapshot directly.
6. Run `prune` and confirm the latest restore/undo chain remains recoverable.

### Acceptance

- Restore never silently destroys current state.
- Undo remains usable after realistic restore activity.
- Prune does not break the latest recovery chain.

## 6. Documentation Smoke Validation

### Goal

Confirm that the root README is sufficient for a technical user without requiring internal contributor docs.

### Steps

1. Follow `README.md` from a clean clone.
2. Run the documented validation commands.
3. Use only the README for:
   - watcher startup;
   - snapshot browsing;
   - restore;
   - undo;
   - Markdown generation;
   - prune;
   - Zed extension setup.
4. Note any step that requires unstated assumptions.

### Acceptance

- README is self-contained for setup and usage.
- Commands, examples, and caveats match actual behavior.

## 7. MCP Validation

### Goal

Validate the additive MCP surface in a real agent client, not only through local unit tests.

### Steps

1. Validate the development path:
   - install the Zed dev extension;
   - put `target/debug/local-history-mcp` in `PATH`;
   - confirm the extension-managed `local-history` context server starts.
2. Validate the production path:
   - remove local development binaries from `PATH`;
   - install or run the extension against a tagged release;
   - confirm the extension downloads/caches the matching `local-history-mcp` release asset.
3. Validate the manual fallback path by registering `local-history-mcp` in a real Zed `context_servers` config with an explicit binary path.
4. Confirm MCP initialization succeeds and the server appears active in each path.
5. Verify `tools/list` exposes the expected local-history tools.
6. Call:
   - `local_history_status`
   - `local_history_create_snapshot`
   - `local_history_recent_snapshots`
   - `local_history_view_snapshot`
   - `local_history_restore_snapshot`
   - `local_history_prune`
7. Confirm restore still creates a safety snapshot before modifying the live file.
8. Verify destructive-tool approval behavior in the real Agent Panel settings.

### Acceptance

- Zed can start the MCP server successfully.
- Production Agent Panel use does not require manual `PATH` setup for `local-history-mcp`.
- The documented tools are available and callable.
- Structured MCP output is usable by the agent.
- Safety-first restore behavior is preserved through MCP.
