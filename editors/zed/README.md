# Zed Local History Extension

Thin Zed extension package for `zed-local-history`.

## Why this package is thin

The project goals are intentionally filesystem-first:

- the native sidecar is the source of truth;
- the CLI and generated Markdown must work without Zed;
- editor integration should stay small and replaceable.

The current Zed extension has real sidecar and MCP behavior while staying intentionally thin.

Zed's documented MCP server support also creates a second integration route: the extension registers the implemented `local-history-mcp` server as the `local-history` context server for the Zed Agent Panel. Users may still connect it directly through their `context_servers` settings when they want an explicit binary path. That path is additive to the current CLI/Markdown workflow.

## Current shape

- `extension.toml` declares the extension manifest, required Zed extension capabilities (`process:exec`, `download_file`), the `local-history` context server, and slash commands.
- `src/lib.rs` resolves `local-history-sidecar` and `local-history-mcp` from `PATH` for dev installs, otherwise downloads matching GitHub release assets into the extension work directory, verifies binary version compatibility, calls real sidecar commands from slash-command handlers, and starts the MCP server for Agent Panel tool use.
- The extension is kept outside the root workspace because it follows Zed's WebAssembly packaging model and will evolve on its own cadence.

## Responsibilities

- detect platform and architecture;
- locate or download the correct sidecar and MCP release artifacts;
- make downloaded binaries executable where needed;
- run focused sidecar commands such as `ensure-daemon`, `status`, and Markdown view lookups;
- expose the most useful recovery flows through Zed-supported extension surfaces;
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
- the extension probes `local-history-mcp --version` before Agent Panel launch and uses the same `PATH` first, cached/downloaded release asset second behavior
- tagged releases publish `SHA256SUMS.txt` alongside the archives that the extension bootstrap relies on
- release bootstrap currently has explicit asset mappings for macOS `x86_64` / `aarch64`, Linux `x86_64` / `aarch64`, and Windows `x86_64` / `aarch64`
- the extension registers `local-history` as a context server; `context_server_command` returns the resolved executable path to Zed because Zed otherwise reinterprets relative extension commands under the extension work directory
- for sidecar slash commands and binary compatibility probes, the extension launches stable names (`local-history-sidecar`, `local-history-mcp`, and Windows `.exe` variants) with the resolved binary directory prepended to `PATH`, so the manifest can avoid wildcard process execution
- PATH/dev MCP binaries are resolved through the host OS lookup command (`command -v` on Unix, `where` on Windows); if lookup resolution fails, the extension falls back to the cached/downloaded release binary. Cached release MCP paths are canonicalized under the extension work directory before being returned to Zed for context-server startup

## Extension capabilities

Zed 1.4+ requires explicit capability declarations in `extension.toml` before the WASM extension may download release assets or execute sidecar/MCP binaries:

- `process:exec` for the stable binary names `local-history-sidecar`, `local-history-sidecar.exe`, `local-history-mcp`, and `local-history-mcp.exe`
- narrow `process:exec` entries for MCP PATH lookup helpers: `sh -c "command -v local-history-mcp"` on Unix and `where local-history-mcp[.exe]` on Windows
- `download_file` from `github.com/neuroborus/zed-local-history/**`

The manifest intentionally does not grant `{ kind = "process:exec", command = "*", args = ["**"] }`. Without the narrower entries above, Agent Panel settings show **Local History** but the MCP toggle fails to stay on and `~/.local/share/zed/logs/Zed.log` reports missing `process:exec` capabilities.

Current limitations:

- the extension API does not provide a direct "open arbitrary external file path" action, so the MVP path is to expose the generated Markdown path instead of pretending it can always auto-open it
- binary bootstrap depends on GitHub release assets with stable names; sidecar/MCP bootstrap archives and `SHA256SUMS.txt` are published by the release workflow, but each extension version still needs a matching tagged release and live Agent Panel validation before store submission
- `x86_64-unknown-linux-musl` is still not part of the extension bootstrap contract because the current platform mapping distinguishes OS and CPU architecture, not Linux libc family

The current MCP server can also coexist with these slash commands through direct `context_servers` configuration. Use a distinct custom server ID such as `local-history-dev` so manual fallback does not shadow the extension-managed `local-history` server.

## Validation target

This package carries its own toolchain file so the extension can target current Zed requirements without forcing the root native workspace to upgrade in lockstep.

For full manual validation, use [agents/ZED_MANUAL_TESTING.md](../../agents/ZED_MANUAL_TESTING.md). For automated checks from the repository root:

```bash
cargo run -p xtask -- zed-ci
```

`zed-ci` and `full-ci` also run `cargo test` in this package (18 unit tests covering MCP spawn-path validation and release-target mapping).
