# AGENTS.md

## Project goals

- Build a practical, local-only history tool for Zed that can recover previous saved file states reliably.
- Keep the native sidecar as the source of truth; Zed integration is a convenience layer, not the core product.
- Preserve a filesystem-first recovery model: CLI and generated Markdown must remain usable even without the editor.
- Keep restore flows safety-first and reversible.

## Working agreements

- All code comments, logs, and user-facing diagnostics must be in English.
- Keep `local-history-core` editor-agnostic. Zed-specific code belongs under `editors/zed` or at the sidecar boundary.
- If an MCP server is introduced, keep it as a thin adapter over `local-history-core` or sidecar commands; do not move recovery business logic into the MCP layer.
- Prefer small, explicit interfaces between crates; the JSON contract should stay stable and easy to test.
- Do not introduce heavy dependencies early. Add crates when they are conventional for the problem and clearly justified.
- Treat generated Markdown as a presentation layer, not the source of truth.
- Restore logic must never silently destroy the current file state; create a safety snapshot first.
- Respect ignore rules and storage boundaries so the tool does not pollute user repositories.
- Keep native workspace validation and Zed extension validation explicit; `editors/zed` uses its own toolchain on purpose.

## Change policy

- Prefer focused changes over broad speculative refactors.
- Keep the workspace compiling as the default standard for repository changes.
- If you touch the Zed extension, document any API limitations or assumptions directly in the extension README or the project docs.
- After changes, run:
  - `cargo run -p xtask -- ci`
  - `cargo run -p xtask -- zed-ci` when working on `editors/zed`
  - `cargo run -p xtask -- full-ci` before broader changes that affect both native crates and the Zed extension
