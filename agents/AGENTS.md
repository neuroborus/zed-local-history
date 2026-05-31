# Agent Working Guide

## Project goals

- Build a practical, local-only history tool for Zed that can recover previous saved file states reliably.
- Keep the native sidecar as the source of truth; Zed integration is a convenience layer, not the core product.
- Preserve a filesystem-first recovery model: CLI and generated Markdown must remain usable even without the editor.
- Keep restore flows safety-first and reversible.

## Working agreements

- All code comments, logs, and user-facing diagnostics must be in English.
- Keep `local-history-core` editor-agnostic. Zed-specific code belongs under `editors/zed` or at the sidecar boundary.
- Keep `local-history-mcp` as a thin adapter over `local-history-core`; do not move recovery business logic into the MCP layer.
- Prefer small, explicit interfaces between crates; the JSON contract should stay stable and easy to test.
- Do not introduce heavy dependencies early. Add crates when they are conventional for the problem and clearly justified.
- Treat generated Markdown as a presentation layer, not the source of truth.
- Restore logic must never silently destroy the current file state; create a safety snapshot first.
- Respect ignore rules and storage boundaries so the tool does not pollute user repositories.
- Keep native workspace validation and Zed extension validation explicit; `editors/zed` uses its own toolchain on purpose.

## Agent-facing documentation

Two different audiences:

- **`agents/CURRENT_STATUS.md`** — the short current-state index for contributors and coding agents; read it before larger roadmap files.
- **`agents/AGENTS.md` (this file)** — how to work on the repository.
- **`llms.txt`** — how a coding agent should use local-history at runtime (MCP tools or CLI). Packaged into `local-history-mcp` as `local-history://guide` and summarized in MCP `SERVER_INSTRUCTIONS`.

When MCP tools, agent workflows, or natural-language routing change, keep these aligned:

- `llms.txt` (canonical runtime guide)
- MCP `SERVER_INSTRUCTIONS` in `crates/local-history-mcp`
- README agent/MCP sections and [Examples](../README.md#examples) when user-facing demos change
- `ZED_MANUAL_TESTING.md` agent acceptance prompts

Document capability-based integration: prefer MCP when `local_history_*` tools are exposed; use the CLI mapping in `llms.txt` when they are not. Do not assume every agent host exposes MCP.

Stored snapshot timestamps remain RFC3339 UTC. Human-readable CLI and MCP tables use the local system timezone with an explicit `UTC` / `+HH:MM` suffix; `--json` and structured MCP snapshot fields stay UTC.

## Change policy

- Prefer focused changes over broad speculative refactors.
- Keep the workspace compiling as the default standard for repository changes.
- If you touch the Zed extension, document any API limitations or assumptions directly in the extension README or the project docs.
- After changes, run:
  - `cargo run -p xtask -- ci`
  - `cargo run -p xtask -- zed-ci` when working on `editors/zed`
  - `cargo run -p xtask -- full-ci` before broader changes that affect both native crates and the Zed extension
