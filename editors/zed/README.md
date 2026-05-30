# Zed Local History Extension

Thin Zed integration scaffold for `zed-local-history`.

## Why this package is thin

The project goals are intentionally filesystem-first:

- the native sidecar is the source of truth;
- the CLI and generated Markdown must work without Zed;
- editor integration should stay small and replaceable.

The current Zed extension is no longer pure placeholder text, but it still stays intentionally thin.

Zed's documented MCP server support also creates a second integration route: the extension registers the implemented `local-history-mcp` server as the `local-history` context server for the Zed Agent Panel. Users may still connect it directly through their `context_servers` settings when they want an explicit binary path. That path is additive to the current CLI/Markdown workflow.

## Current shape

- `extension.toml` declares the extension manifest, the `local-history` context server, and slash commands.
- `src/lib.rs` resolves `local-history-sidecar` from `PATH` for dev installs, otherwise downloads a matching GitHub release asset into the extension work directory, verifies sidecar version compatibility, calls real sidecar commands from slash-command handlers, and starts `local-history-mcp` for Agent Panel tool use when that binary is in `PATH`.
- The extension is kept outside the root workspace because it follows Zed's WebAssembly packaging model and will evolve on its own cadence.

## Planned responsibilities

- detect platform and architecture;
- locate or download the correct sidecar release artifact;
- make the sidecar executable where needed;
- run focused sidecar commands such as `ensure-daemon`, `status`, and Markdown view lookups;
- expose the most useful recovery flows through Zed-supported extension surfaces.
- register the existing `local-history-mcp` server for Agent Panel MCP tool use.

The native sidecar now already exposes real JSON `health`, `status`, `watch`, and `ensure-daemon` behavior. The extension intentionally keeps direct slash-command output small while Agent Panel functionality goes through MCP tools.

## Current commands

These are extension slash commands for Zed surfaces that support extension slash commands. They are not Agent Panel commands; the new Agent Panel should use the `local-history` MCP context server instead.

- `/local-history-status`
- `/local-history-view`
- `/local-history-start-watcher`
- `/local-history-current-hour`
- `/local-history-current-segment`
- `/local-history-previous-hour`
- `/local-history-hour <YYYY-MM-DDTHH>`
- `/local-history-segment <YYYY-MM-DDTHH:MM:SSZ>`
- `/local-history-restore <snapshot-id-or-unique-prefix>`

Current behavior:

- `status` calls `local-history-sidecar status <project-root>`
- `start-watcher` calls `local-history-sidecar ensure-daemon <project-root>`
- `view` exposes the generated Markdown view root path
- `current-hour`, `current-segment`, `previous-hour`, `hour`, and `segment` call sidecar Markdown render commands and return the generated file path
- `restore` calls `local-history-sidecar restore <snapshot-id-or-unique-prefix>`
- the extension probes `local-history-sidecar version` before use; if a `PATH` binary is missing or too old, it falls back to the cached/downloaded release asset for the current extension version
- tagged releases publish `SHA256SUMS.txt` alongside the archives that the extension bootstrap relies on
- release bootstrap currently has explicit asset mappings for macOS `x86_64` / `aarch64`, Linux `x86_64` / `aarch64`, and Windows `x86_64` / `aarch64`
- the extension registers `local-history` as a context server and starts `local-history-mcp` from `PATH`

Current limitations:

- the extension API does not provide a direct "open arbitrary external file path" action, so the MVP path is to expose the generated Markdown path instead of pretending it can always auto-open it
- sidecar bootstrap currently depends on GitHub release assets with stable names; the workflow now produces those assets plus release checksums, but the full packaging/release story still belongs to later-stage release hardening
- extension-managed MCP registration currently expects `local-history-mcp` to be available in `PATH`; packaged release bootstrap for the MCP binary still needs live validation
- `x86_64-unknown-linux-musl` is still not part of the extension bootstrap contract because the current platform mapping distinguishes OS and CPU architecture, not Linux libc family

The current MCP server can also coexist with these slash commands through direct `context_servers` configuration if users prefer an explicit binary path.

## Validation target

This package carries its own toolchain file so the extension can target current Zed requirements without forcing the root native workspace to upgrade in lockstep.

When you start implementing the real extension logic, validate it from this directory:

```bash
cd editors/zed
cargo check --target wasm32-wasip2
```
