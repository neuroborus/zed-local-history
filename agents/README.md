# agents

This directory replaces a single root-level `AGENTS.md`.

It keeps the project's agent-facing and implementation-guiding documentation in one place:

- `CURRENT_STATUS.md` is the short current-state index for contributors and coding agents. Start here before opening the larger roadmap files.
- `AGENTS.md` contains the working agreements for **contributors and coding agents working on the repo**.
- `GOALS.md` captures product direction and architectural constraints. It includes future-looking and historical design context.
- `DEVELOPMENT_PLAN.md` turns those goals into staged implementation work. It includes closed milestones and open roadmap items.
- `FINALIZE.md` is the post-change checklist before considering a change set done.
- `ZED_MANUAL_TESTING.md` contains the live Zed/manual acceptance flow and local dev setup.
- `../README.md` is the user-facing overview, including [Examples](../README.md#examples) with `docs/` demo GIFs (agent, CLI, Markdown preview).
- `../llms.txt` is the compact **runtime** operating guide for LLM agents in MCP or shell-only hosts. It is exposed through MCP as `local-history://guide` and includes natural-language intent mapping, MCP↔CLI workflow, restore safety, and integration boundaries (~115 lines, agent-ops only; crate architecture stays in `GOALS.md`).

## Why this exists

`zed-local-history` is a monorepo-style project with multiple executables and an editor integration package. Keeping the guidance docs together avoids scattering project intent between the root and individual packages.

## How to use it

- Read `CURRENT_STATUS.md` first for the current implemented behavior and known limits.
- Read `AGENTS.md` before making repository-wide changes.
- Read the relevant crate README or source module for local implementation details.
- Read `GOALS.md` only when you need product boundaries and architectural intent beyond the current-state index.
- Read `DEVELOPMENT_PLAN.md` only when planning implementation work or scoping milestones.
- Read `FINALIZE.md` before closing out a change set or preparing it for commit.
- Read `../RHYTHM.md` when recording meaningful architectural or workflow decisions.
- Read `ZED_MANUAL_TESTING.md` when validating the Zed extension, MCP, or release bootstrap manually.
- Read `../README.md` for user-facing usage and demo GIFs.
- Read `../llms.txt` when an **end-user agent** needs runtime behavior: intent mapping, restore safety, and MCP or CLI usage. Do not confuse it with `AGENTS.md` in this directory.
