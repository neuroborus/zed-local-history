# zed-local-history

Pragmatic repository scaffold for a filesystem-first local-history tool for Zed.

This repository is set up around the product direction captured in [agents/GOALS.md](./agents/GOALS.md) and [agents/DEVELOPMENT_PLAN.md](./agents/DEVELOPMENT_PLAN.md):

- native Rust sidecar as the source of truth;
- CLI and Markdown as the dependable recovery interface;
- thin Zed integration instead of a custom editor UI dependency;
- clean monorepo layout that can grow without structural churn.

The documented architecture also leaves room for an MCP server surface for Zed Agent workflows, but that remains separate from the current MVP path.

The repository now has a real storage-backed recovery path for manual CLI snapshots and safe restore flows, a polling-based sidecar watcher with `watch`, `ensure-daemon`, and `status`, generated Markdown history views for hour/segment browsing, and a real Zed slash-command integration layer with sidecar bootstrap. The extension now resolves `local-history-sidecar` from `PATH` for dev workflows, otherwise falls back to a cached/downloaded GitHub release asset for supported platforms, verifies sidecar version compatibility before use, and can trigger both hour and fixed 10-minute segment Markdown renders.

## What is here

- `agents/` keeps the project goals, roadmap, agent agreements, and supporting notes together.
- `crates/local-history-core` is the editor-agnostic place for storage/layout/domain logic.
- `crates/local-history-cli` is the user-facing command surface.
- `crates/local-history-sidecar` is the native daemon/process boundary.
- `editors/zed` is a dedicated Zed extension package, kept outside the root workspace on purpose.
- `xtask` provides one-command local checks for the Rust workspace, the Zed extension package, and the combined repository path.
- `RHYTHM.md` records meaningful repository decisions in newest-first order.

Documented, but not implemented yet:

- `local-history-mcp` as a thin MCP adapter for agent-facing tool calls.

## Quickstart

Run the workspace checks:

```bash
cargo run -p xtask -- ci
```

Run the full repository checks, including `editors/zed`:

```bash
cargo run -p xtask -- full-ci
```

Try the CLI recovery surfaces:

```bash
cargo run -p local-history-cli -- snapshot . --file README.md
cargo run -p local-history-cli -- recent .
cargo run -p local-history-cli -- recent . --json
cargo run -p local-history-cli -- list . --page 2 --page-size 20
cargo run -p local-history-cli -- list . --file README.md --from 2026-05-02T14:00:00Z --to 2026-05-02T15:00:00Z --json
cargo run -p local-history-cli -- history hour . --hour 2026-05-02T14
cargo run -p local-history-cli -- history segment . --from 2026-05-02T14:10:00Z --to 2026-05-02T14:20:00Z --json
cargo run -p local-history-cli -- view-root .
cargo run -p local-history-cli -- render-markdown hour . --hour 2026-05-02T14
cargo run -p local-history-cli -- render-markdown segment . --from 2026-05-02T14:10:00Z --to 2026-05-02T14:20:00Z
cargo run -p local-history-cli -- rebuild-markdown-view .
cargo run -p local-history-cli -- prune .
cargo run -p local-history-cli -- show <snapshot-id>
cargo run -p local-history-cli -- restore <snapshot-id>
cargo run -p local-history-cli -- restore --project-root . --recent 1
cargo run -p local-history-cli -- safety-list .
cargo run -p local-history-cli -- browse .
cargo run -p local-history-cli -- undo-restore .
cargo run -p local-history-cli -- restore-last-safety .
cargo run -p local-history-sidecar -- health
cargo run -p local-history-sidecar -- version
cargo run -p local-history-sidecar -- status .
cargo run -p local-history-sidecar -- ensure-daemon .
cargo run -p local-history-sidecar -- render-markdown current-segment .
```

Current CLI behavior:

- `recent` lists raw user snapshots only, so safety snapshots do not pollute normal restore numbering.
- `list` adds paginated browsing with `--page` and `--page-size`.
- `recent`, `list`, `show`, `status`, and `safety-list` support `--json`.
- `status` also exposes the default retention policy: `250` snapshots per file, `512 MiB` estimated project storage, `4 MiB` max snapshot file size, and `30` days max snapshot age.
- `recent`, `list`, and `safety-list` support `--file`, `--from`, `--to`, and `--hour YYYY-MM-DDTHH`.
- `browse` provides a minimal interactive recovery loop with next/previous navigation, snapshot preview, and restore confirmation.
- `history hour` and `history segment` group raw snapshots into fixed 10-minute windows and list affected files with exact snapshot IDs preserved.
- `render-markdown hour` generates a browsable hour directory with `README.md`, six fixed segment pages, and exact snapshot Markdown pages under `view/<date>/<hour>/`.
- `render-markdown segment` validates a fixed 10-minute window and refreshes the parent hour view before returning the exact segment Markdown path.
- `view-root` prints the Markdown view root; `rebuild-markdown-view` clears and rebuilds the full filesystem-browsable Markdown tree from raw snapshots.
- `prune` applies the default retention policy, preserves only the latest restore/undo chain needed for current undo behavior, removes stale restore-operation rows and orphaned blobs, and rebuilds the Markdown view.
- `restore` always creates a safety snapshot first, records a restore operation, and then applies the target snapshot.
- `undo-restore` replays the latest safety snapshot for the project.
- `restore-last-safety` is an explicit escape hatch to restore the newest safety snapshot directly.
- `safety-list` shows the stored safety snapshots separately from normal history.

Current sidecar behavior:

- `watch <project-root>` performs an initial scan, applies default ignore rules, and then polls for saved file changes.
- when a tracked file changes, the sidecar stores the previous known state as a raw snapshot;
- when a tracked file is deleted, the sidecar stores the previous known state before dropping it from the cache;
- files larger than the current retention limit are skipped instead of repeatedly failing the watcher loop;
- `ensure-daemon <project-root>` starts a background watcher if there is no fresh heartbeat;
- `status <project-root>` reports watcher state from the sidecar heartbeat file.

## Privacy, Storage, and Limits

Local history is stored outside the user repository:

- macOS: `~/Library/Application Support/local-history`
- Linux: `$XDG_DATA_HOME/local-history` or `~/.local/share/local-history`
- Windows: `%LOCALAPPDATA%\\local-history`

Per project, the storage layout is:

- `projects/<project-id>/metadata.sqlite` for snapshot metadata and restore operations;
- `projects/<project-id>/blobs/` for compressed snapshot contents;
- `projects/<project-id>/view/` for generated Markdown history;
- `projects/<project-id>/logs/` for watcher logs and heartbeat status.

Current data-handling rules and limits:

- any saved file that is not ignored can be snapshotted, including credentials embedded in normal source files;
- built-in ignores currently skip `.git/`, `node_modules/`, `target/`, `dist/`, `build/`, `.next/`, `.cache/`, `coverage/`, `.env*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.sqlite`, `*.db`, and `*.log`;
- the current max snapshot file size is `4 MiB`;
- retention defaults keep at most `250` snapshots per file, cap referenced project storage at `512 MiB`, and prune snapshots older than `30` days;
- the `IgnorePolicy` reserves `.local-history-ignore`, but custom project-local ignore parsing is not wired yet, so ignore behavior is built-in only today.

Deletion and cleanup:

- remove one project's history by deleting its `projects/<project-id>/` directory;
- remove all history by deleting the whole `local-history` base directory;
- run `cargo run -p local-history-cli -- prune <project-root>` to apply retention without deleting the entire project history.

## Troubleshooting

- Sidecar not starting: run `cargo run -p local-history-sidecar -- status <project-root>` and inspect `projects/<project-id>/logs/watcher.log` plus `watcher-status.json`.
- Extension capabilities disabled or bootstrap failing: use the CLI directly first. The current Zed integration is additive and recovery does not depend on slash commands.
- Unsupported platform: the extension bootstrap currently targets macOS `x86_64` / `aarch64`, Linux `x86_64` / `aarch64`, and Windows `x86_64` / `aarch64`. Other platforms need a manual sidecar path or direct CLI usage.
- Watcher not detecting changes: only saved on-disk changes are captured, built-in ignored paths are skipped, and the current watcher is polling-based rather than event-driven.
- Storage too large: run `cargo run -p local-history-cli -- status <project-root>` to inspect retention settings, then run `cargo run -p local-history-cli -- prune <project-root>`.
- Markdown not updating: rerun `cargo run -p local-history-cli -- rebuild-markdown-view <project-root>`. Markdown generation writes under external storage and does not snapshot itself recursively.
- Restore failure: confirm the snapshot still exists with `show <snapshot-id>` or `recent <project-root>`. A previously generated Markdown link can outlive the snapshot it points to after pruning.

## Zed Extension Notes

The `editors/zed` package follows the current documented Zed extension shape:

- Git repository with `extension.toml`;
- Rust `cdylib` crate compiled to WebAssembly;
- thin integration surface centered around slash commands today.

Zed also supports MCP servers for the Agent Panel, either through direct user `context_servers` settings or through extension-managed MCP server registration. The project docs treat that as an additive integration path, not as a replacement for CLI and Markdown recovery.

The current extension no longer returns placeholder text. It resolves `local-history-sidecar` from `PATH` for dev installs, otherwise downloads and caches a matching GitHub release asset in the extension work directory, then calls real sidecar commands for status / watcher startup / hour rendering / segment rendering / restore. Before using a discovered binary, the extension probes `local-history-sidecar version` and requires a compatible sidecar version; an outdated `PATH` binary is ignored in favor of the bundled release path. Because the current extension API does not expose a direct "open arbitrary external file" action, the MVP path is to print or reveal the generated Markdown path in a usable way rather than pretending it can always be opened automatically.

Tagged GitHub releases now also publish `SHA256SUMS.txt` alongside platform archives and fixed-name sidecar assets. The current extension bootstrap contract now covers macOS `x86_64` / `aarch64`, Linux `x86_64` / `aarch64`, and Windows `x86_64` / `aarch64`.

Current Zed-facing commands now cover:

- `/local-history-status`
- `/local-history-start-watcher`
- `/local-history-view`
- `/local-history-current-hour`
- `/local-history-current-segment`
- `/local-history-previous-hour`
- `/local-history-hour <YYYY-MM-DDTHH>`
- `/local-history-segment <YYYY-MM-DDTHH:MM:SSZ>`
- `/local-history-restore <snapshot-id>`

The root workspace is pinned to Rust `1.75.0` so the core/cli/sidecar scaffold can compile immediately in conservative environments. The Zed extension package keeps its own `stable` toolchain in `editors/zed/rust-toolchain.toml`, because `wasm32-wasip2` support belongs to the newer extension path and should not force the native workspace to move in lockstep.

## Repository Layout

```text
zed-local-history/
  README.md
  RHYTHM.md
  LICENSE
  Cargo.toml
  rust-toolchain.toml
  .gitignore
  agents/
    AGENTS.md
    README.md
    GOALS.md
    DEVELOPMENT_PLAN.md
  crates/
    local-history-core/
    local-history-sidecar/
    local-history-cli/
  editors/
    zed/
  xtask/
  .github/
    workflows/
      ci.yml
      release.yml
```
