# Zed Local History Extension

Thin Zed integration scaffold for `zed-local-history`.

## Why this package is thin

The project goals are intentionally filesystem-first:

- the native sidecar is the source of truth;
- the CLI and generated Markdown must work without Zed;
- editor integration should stay small and replaceable.

The current Zed extension is no longer pure placeholder text, but it still stays intentionally thin.

Zed's documented MCP server support also creates a second possible integration route: the extension may register a local-history MCP server for the Agent Panel, or users may connect such a server directly through their `context_servers` settings. That path is documented as additive to the current CLI/Markdown workflow.

## Current shape

- `extension.toml` declares the extension manifest and slash commands.
- `src/lib.rs` resolves `local-history-sidecar` from `PATH` and calls real sidecar commands from slash-command handlers.
- The extension is kept outside the root workspace because it follows Zed's WebAssembly packaging model and will evolve on its own cadence.

## Planned responsibilities

- detect platform and architecture;
- locate or download the correct sidecar release artifact;
- make the sidecar executable where needed;
- run focused sidecar commands such as `ensure-daemon`, `status`, and Markdown view lookups;
- expose the most useful recovery flows through Zed-supported extension surfaces.
- optionally register a local-history MCP server when that adapter exists.

The native sidecar now already exposes real JSON `health`, `status`, `watch`, and `ensure-daemon` behavior, so the remaining extension work is mainly about invoking those commands from Zed and presenting their results cleanly.

## Current commands

- `/local-history-status`
- `/local-history-view`
- `/local-history-start-watcher`
- `/local-history-current-hour`
- `/local-history-previous-hour`
- `/local-history-hour <YYYY-MM-DDTHH>`
- `/local-history-restore <snapshot-id>`

Current behavior:

- `status` calls `local-history-sidecar status <project-root>`
- `start-watcher` calls `local-history-sidecar ensure-daemon <project-root>`
- `view` exposes the generated Markdown view root path
- `current-hour`, `previous-hour`, and `hour` call sidecar Markdown render commands and return the generated file path
- `restore` calls `local-history-sidecar restore <snapshot-id>`

Current limitations:

- the extension API does not provide a direct "open arbitrary external file path" action, so the MVP path is to expose the generated Markdown path instead of pretending it can always auto-open it
- the extension currently expects `local-history-sidecar` to already be on `PATH`; automatic platform-specific download/install remains later-stage work

If MCP integration is added, these commands may coexist with Agent Panel tools rather than being replaced by them.

## Validation target

This package carries its own toolchain file so the extension can target current Zed requirements without forcing the root native workspace to upgrade in lockstep.

When you start implementing the real extension logic, validate it from this directory:

```bash
cd editors/zed
cargo check --target wasm32-wasip2
```
