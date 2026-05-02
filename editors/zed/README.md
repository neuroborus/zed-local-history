# Zed Local History Extension

Thin Zed integration scaffold for `zed-local-history`.

## Why this package is thin

The project goals are intentionally filesystem-first:

- the native sidecar is the source of truth;
- the CLI and generated Markdown must work without Zed;
- editor integration should stay small and replaceable.

The current Zed extension scaffold reserves a realistic integration surface without pretending that the product UX is already implemented.

## Current shape

- `extension.toml` declares the extension manifest and starter slash commands.
- `src/lib.rs` provides a minimal Rust extension implementation.
- The extension is kept outside the root workspace because it follows Zed's WebAssembly packaging model and will evolve on its own cadence.

## Planned responsibilities

- detect platform and architecture;
- locate or download the correct sidecar release artifact;
- make the sidecar executable where needed;
- run focused sidecar commands such as `ensure-daemon`, `status`, and Markdown view lookups;
- expose the most useful recovery flows through Zed-supported extension surfaces.

## Current commands

- `/local-history-status`
- `/local-history-recent`
- `/local-history-view`

These commands currently return bootstrap text only. They exist to anchor the intended Zed-side workflow while Stage 1 API validation happens.

## Validation target

This package carries its own toolchain file so the extension can target current Zed requirements without forcing the root native workspace to upgrade in lockstep.

When you start implementing the real extension logic, validate it from this directory:

```bash
cd editors/zed
cargo check --target wasm32-wasip2
```
