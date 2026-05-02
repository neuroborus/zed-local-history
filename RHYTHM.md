# RHYTHM.md

Chronological log of meaningful repo decisions. **Newest sections first:** add each new `## YYYY-MM-DD` block right below this paragraph, not at the end of the file.

## 2026-05-02

- Bootstrapped `zed-local-history` as a Rust monorepo with `crates/local-history-core`, `crates/local-history-cli`, `crates/local-history-sidecar`, `xtask`, and a separate `editors/zed` package. The repository keeps project guidance under `agents/` instead of a single root `AGENTS.md`, and root docs now include `README.md`, `RHYTHM.md`, and `LICENSE`.
- Zed extension validation is intentionally split from the native workspace: the root repo stays on Rust `1.75.0` for conservative native compatibility, while `editors/zed` carries its own `rust-toolchain.toml` on `stable` with `wasm32-wasip2`. `xtask` now exposes `zed-fmt`, `zed-clippy`, `zed-check`, `zed-ci`, and `full-ci`, and explicitly clears inherited `RUSTUP_TOOLCHAIN` before nested `editors/zed` cargo runs so the extension package can honor its own toolchain file.
- Release automation now distinguishes generic Linux output from pinned Ubuntu 24.04 output in `.github/workflows/release.yml`: `ubuntu-latest` produces `local-history-x86_64-unknown-linux-gnu`, while a separate `ubuntu-24.04` job produces `local-history-x86_64-ubuntu-24.04`.
- `local-history-core` moved from a placeholder helper file toward Stage 3 storage foundations: project identity is becoming a stable hash of normalized project root plus machine salt, storage layout remains external to the user repo under `projects/<project-id>/...`, and ignore policy is explicitly modeled around `.gitignore`-style defaults plus `.local-history-ignore`.
