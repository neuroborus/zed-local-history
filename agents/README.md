# agents

This directory replaces a single root-level `AGENTS.md`.

It keeps the project's agent-facing and implementation-guiding documentation in one place:

- `AGENTS.md` contains the working agreements for contributors and coding agents.
- `GOALS.md` captures product direction and architectural constraints.
- `DEVELOPMENT_PLAN.md` turns those goals into staged implementation work.
- `FINALIZE.md` is the post-change checklist before considering a change set done.
- `ZED_MANUAL_TESTING.md` contains the live Zed/manual acceptance flow and local dev setup.
- `../llms.txt` is the compact operating guide for LLM agents and is also exposed through MCP as `local-history://guide`. It includes natural-language intent mapping (for example "what changed" → recent + diff), MCP↔CLI workflow, restore safety, and integration boundaries.

## Why this exists

`zed-local-history` is a monorepo-style project with multiple executables and an editor integration package. Keeping the guidance docs together avoids scattering project intent between the root and individual packages.

## How to use it

- Read `GOALS.md` first for product boundaries and architectural intent.
- Read `DEVELOPMENT_PLAN.md` when planning implementation work or scoping milestones.
- Read `AGENTS.md` before making repository-wide changes.
- Read `FINALIZE.md` before closing out a change set or preparing it for commit.
- Read `ZED_MANUAL_TESTING.md` when validating the Zed extension, MCP, or release bootstrap manually.
- Read `../llms.txt` when an agent needs the runtime model, intent mapping, restore safety rules, and MCP or CLI usage in one place.
