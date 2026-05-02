# zed-local-history

Pragmatic repository scaffold for a filesystem-first local-history tool for Zed.

This repository is set up around the product direction captured in [agents/GOALS.md](./agents/GOALS.md) and [agents/DEVELOPMENT_PLAN.md](./agents/DEVELOPMENT_PLAN.md):

- native Rust sidecar as the source of truth;
- CLI and Markdown as the dependable recovery interface;
- thin Zed integration instead of a custom editor UI dependency;
- clean monorepo layout that can grow without structural churn.

The documented architecture also leaves room for an MCP server surface for Zed Agent workflows, but that remains separate from the current MVP path.

The repository now has a real storage-backed recovery path for manual CLI snapshots and safe restore flows. File watching, generated Markdown views, and full Zed-side sidecar lifecycle are still later-stage work.

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
cargo run -p local-history-cli -- show <snapshot-id>
cargo run -p local-history-cli -- restore <snapshot-id>
cargo run -p local-history-cli -- restore --project-root . --recent 1
cargo run -p local-history-cli -- safety-list .
cargo run -p local-history-cli -- browse .
cargo run -p local-history-cli -- undo-restore .
cargo run -p local-history-cli -- restore-last-safety .
cargo run -p local-history-sidecar -- health
```

Current CLI behavior:

- `recent` lists raw user snapshots only, so safety snapshots do not pollute normal restore numbering.
- `list` adds paginated browsing with `--page` and `--page-size`.
- `recent`, `list`, `show`, `status`, and `safety-list` support `--json`.
- `recent`, `list`, and `safety-list` support `--file`, `--from`, `--to`, and `--hour YYYY-MM-DDTHH`.
- `browse` provides a minimal interactive recovery loop with next/previous navigation, snapshot preview, and restore confirmation.
- `restore` always creates a safety snapshot first, records a restore operation, and then applies the target snapshot.
- `undo-restore` replays the latest safety snapshot for the project.
- `restore-last-safety` is an explicit escape hatch to restore the newest safety snapshot directly.
- `safety-list` shows the stored safety snapshots separately from normal history.

## Zed Extension Notes

The `editors/zed` package follows the current documented Zed extension shape:

- Git repository with `extension.toml`;
- Rust `cdylib` crate compiled to WebAssembly;
- thin integration surface centered around slash commands today.

Zed also supports MCP servers for the Agent Panel, either through direct user `context_servers` settings or through extension-managed MCP server registration. The project docs treat that as an additive integration path, not as a replacement for CLI and Markdown recovery.

The richer UX described in the product docs, such as opening generated Markdown directly or managing the sidecar lifecycle entirely from editor actions, still needs Stage 1 validation against the current Zed extension API.

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
