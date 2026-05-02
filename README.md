# zed-local-history

Pragmatic repository scaffold for a filesystem-first local-history tool for Zed.

This repository is set up around the product direction captured in [agents/GOALS.md](./agents/GOALS.md) and [agents/DEVELOPMENT_PLAN.md](./agents/DEVELOPMENT_PLAN.md):

- native Rust sidecar as the source of truth;
- CLI and Markdown as the dependable recovery interface;
- thin Zed integration instead of a custom editor UI dependency;
- clean monorepo layout that can grow without structural churn.

The current scaffold establishes the repository baseline for implementation. It does not claim that snapshotting, restore, watcher, or Zed-side bootstrapping are complete yet.

## What is here

- `agents/` keeps the project goals, roadmap, agent agreements, and supporting notes together.
- `crates/local-history-core` is the editor-agnostic place for storage/layout/domain logic.
- `crates/local-history-cli` is the user-facing command surface.
- `crates/local-history-sidecar` is the native daemon/process boundary.
- `editors/zed` is a dedicated Zed extension package, kept outside the root workspace on purpose.
- `xtask` provides one-command local checks for the Rust workspace, the Zed extension package, and the combined repository path.
- `RHYTHM.md` records meaningful repository decisions in newest-first order.

## Quickstart

Run the workspace checks:

```bash
cargo run -p xtask -- ci
```

Run the full repository checks, including `editors/zed`:

```bash
cargo run -p xtask -- full-ci
```

Try the placeholder CLI surfaces:

```bash
cargo run -p local-history-cli -- status .
cargo run -p local-history-cli -- view-root .
cargo run -p local-history-sidecar -- health
```

## Zed Extension Notes

The `editors/zed` package follows the current documented Zed extension shape:

- Git repository with `extension.toml`;
- Rust `cdylib` crate compiled to WebAssembly;
- thin integration surface centered around slash commands today.

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
