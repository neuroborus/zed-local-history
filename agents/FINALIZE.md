# Finalization Checklist

Run through this after feature or refactor work to keep the native workspace, Zed extension, and project docs aligned.

## 1. Tests for new functionality

- [ ] New behavior has appropriate coverage for repository maturity:
  - unit tests for pure storage/layout/domain helpers in `crates/local-history-core`
  - CLI behavior tests where command parsing or output contracts become real
  - sidecar/runtime tests where watcher, restore, or storage behavior becomes real
  - extension validation only where `editors/zed` behavior materially changes
- [ ] Existing tests still make sense; obsolete bootstrap tests or assumptions are updated or removed
- [ ] New behavior is verified at the right boundary instead of only through indirect smoke coverage

---

## 2. Full check run

- [ ] Run the relevant checks before considering the change set done:

```bash
cargo run -p xtask -- ci
```

- [ ] If `editors/zed` changed, also run:

```bash
cargo run -p xtask -- zed-ci
```

- [ ] If the change spans both native crates and the Zed extension, run:

```bash
cargo run -p xtask -- full-ci
```

- [ ] Fix failures before considering the change set done

---

## 3. Boundary review

- [ ] Editor-agnostic logic stays in `crates/local-history-core`
- [ ] User-facing CLI behavior stays in `crates/local-history-cli`
- [ ] Long-running watcher / daemon concerns stay in `crates/local-history-sidecar`
- [ ] Zed-specific integration stays under `editors/zed`
- [ ] No editor-specific assumptions leak into core storage or snapshot models

---

## 4. Recovery and storage review

- [ ] Restore flows still create a safety snapshot before modifying current file state
- [ ] Generated Markdown remains a presentation layer, not the source of truth
- [ ] Ignore rules and storage boundaries still prevent repository pollution
- [ ] Markdown or generated view files do not accidentally become recursive snapshot inputs
- [ ] JSON / CLI / sidecar contracts remain coherent when snapshot or restore behavior changes

---

## 5. Zed extension review

- [ ] If `editors/zed` changed, extension behavior still matches the documented Zed API surface actually in use
- [ ] The extension-specific toolchain remains isolated under `editors/zed/rust-toolchain.toml`
- [ ] Sidecar bootstrap/download/command assumptions are reflected in `editors/zed/README.md` when they change materially
- [ ] No extension-specific toolchain requirement was accidentally pushed into the root native workspace

---

## 6. Convention review

- [ ] Comments, logs, and user-facing diagnostics are in English
- [ ] New interfaces are small and explicit rather than coupled across crates
- [ ] Heavy dependencies were not added without clear justification
- [ ] Naming and structure still follow the current repository layout and intent

---

## 7. Error and logging review

- [ ] Errors remain useful at the user boundary: CLI, sidecar, or extension
- [ ] Logs include enough runtime context to debug watcher/restore/install failures
- [ ] No secrets, private paths beyond what is necessary, or sensitive file contents are logged carelessly

---

## 8. Agents / goals alignment

- [ ] Changes match the repository agent guidance, not only compiler success
- [ ] Re-read `agents/AGENTS.md` for contribution rules and validation expectations
- [ ] Re-read `agents/GOALS.md` when product boundaries, storage model, recovery assumptions, or Zed integration shape changed
- [ ] Re-read `agents/DEVELOPMENT_PLAN.md` when the work changes stage scope, milestone ordering, or implementation assumptions

---

## 9. Documentation review

- [ ] `README.md` still reflects the current repository shape and validation flow (including [Examples](../README.md#examples) and `docs/` demo GIFs when user-facing demos change)
- [ ] `RHYTHM.md` records meaningful architectural, workflow, or behavior changes in newest-first order
- [ ] `llms.txt` and MCP `SERVER_INSTRUCTIONS` stay aligned when agent routing or MCP tool behavior changes
- [ ] `editors/zed/README.md` still reflects the current extension strategy when that package changes
- [ ] `agents/*` remain accurate if repository boundaries, workflow, or architecture changed materially
- [ ] Examples and command snippets still match the actual repository commands

---

## 10. Commit preparation (do not commit)

- [ ] Review the final diff for scaffolding drift, temporary text, and stale bootstrap comments
- [ ] Draft a concise commit message that matches the actual scope of the change set
- [ ] Present the proposed commit summary to the user; do not run `git commit` automatically
