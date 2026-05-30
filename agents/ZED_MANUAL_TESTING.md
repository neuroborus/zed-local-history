# Zed Manual Testing Guide

This document is the manual acceptance checklist for `zed-local-history` in a real Zed install and for CLI-only agent hosts without `local-history-mcp` tools.

Local dev details such as `cargo build`, `target/debug`, and shell `PATH` setup live here instead of the root README.

Capability-based agent guidance lives in [llms.txt](../llms.txt): use MCP tools when the client exposes them; otherwise use the CLI mapping in that file.

## CLI-only Agent Testing

Use this section when the agent host has shell access to `local-history` but no `local_history_*` MCP tools.

Prerequisites:

```bash
export REPO=/path/to/zed-local-history
export TEST_PROJECT=/tmp/lh-agent-cli
export PATH="$HOME/.cargo/bin:$REPO/target/debug:$PATH"
mkdir -p "$TEST_PROJECT/src"
echo 'v1' > "$TEST_PROJECT/note.txt"
local-history-sidecar ensure-daemon "$TEST_PROJECT"
# edit and save note.txt to v2 so a raw snapshot exists
```

Ask the agent to recover history using **CLI commands only**, for example:

```text
Use local-history CLI commands to show status, list recent snapshots, show the latest snapshot for note.txt, diff it against the live file, and explain whether restore would be safe. Project root: /tmp/lh-agent-cli
```

Expected:

- the agent runs `local-history status`, `local-history recent`, `local-history show`, and `local-history diff` via shell;
- the agent does not claim MCP tools ran if they are unavailable;
- restore is not performed unless explicitly requested;
- if restore is performed, the agent mentions safety snapshot behavior and `local-history undo-restore` as the rollback path.

## Local Dev Shell Setup

Set these once for the examples below:

```bash
export REPO=/path/to/zed-local-history
export TEST_PROJECT=/tmp/lh-zed-manual
export PATH="$HOME/.cargo/bin:$REPO/target/debug:$PATH"
```

## Current Version Baseline

As of 2026-05-30:

- Zed Stable: 1.4.4, released 2026-05-28.
- Zed Preview: v1.5.3-pre.
- `zed-local-history`: 0.1.0.
- Native workspace Rust: 1.75.0, from `rust-toolchain.toml`.
- Zed extension Rust: stable + `wasm32-wasip2`, from `editors/zed/rust-toolchain.toml`.
- Zed extension API dependency: `zed_extension_api = "0.7.0"`.

Do not use local Zed 1.0.1 for final acceptance. Update Zed first, then verify:

```bash
zed --version
```

Expected: Zed 1.4.4 or newer stable.

## Where Extension Slash Commands Work (Zed 1.4+)

Extension slash commands such as `/local-history-status` are **not** shell commands and **not** Agent Panel chat commands.

| Surface | `/local-history-*` works? | Notes |
|--------|---------------------------|-------|
| Integrated terminal (bash) | **No** | `/local-history-status` is interpreted as a filesystem path; expect `No such file or directory`. Use `local-history-sidecar` instead. |
| Agent Panel → default thread (`Zed Agent`, `agent: new thread`) | **No** | The input hint says `/ for commands`, but that menu is for **built-in and agent** slash commands (`/file`, `/terminal`, …), not extension commands. The Agent may report `/local-history-status` as unrecognized. |
| Agent Panel → **Text Thread** | **Yes** (when available) | Type `/` at the **start of a line** in the text-thread editor buffer. Extension commands should appear in the completion list. |
| MCP in Agent Panel | **N/A** | When MCP tools are available, use `local_history_*` tools. When not, use the [CLI-only agent flow](#cli-only-agent-testing). |

### Opening a Text Thread (optional UI path)

If your Zed build still exposes text threads:

1. Open the Agent Panel (✨ in the status bar or Command Palette → `agent: new thread`).
2. Click **`+`** (top-right of the panel).
3. Choose **`New Text Thread`** — **not** `Zed Agent`, `Terminal`, or an external agent.

On Zed 1.4.4 stable, the **`+` menu may omit `New Text Thread`** (only `New From Summary`, `Zed Agent`, `Terminal`, external agents). That is a Zed UI limitation, not a failed dev-extension install. See [Sidecar CLI fallback](#sidecar-cli-fallback-zed-144-without-text-thread).

Other ways to reach a text thread when the menu entry is missing:

- Command Palette: search **`New Text Thread`** or **`agent: new text thread`** (do not confuse with `multi workspace: next thread`).
- Settings → open `settings.json` and set:

```json
{
  "agent": {
    "default_view": "text_thread"
  }
}
```

Restart Zed and reopen the Agent Panel.

Extension slash commands were historically tied to text threads; some current Zed builds no longer expose text threads in the UI.

### Primary acceptance path on Zed 1.4.4

When text threads or extension slash completions are unavailable, treat **`local-history-sidecar` and `local-history` in the terminal** plus **MCP in the Agent Panel** as the primary manual acceptance path. Optional extension slash commands are a bonus check when the Text Thread UI exists.

## Automated Preflight

From the repository root:

```bash
git status --short
cargo run -p xtask -- full-ci
```

Expected:

- `git status --short` is empty, unless testing intentionally modified files.
- `full-ci` completes successfully.

The full check covers:

- native formatting;
- native clippy with warnings denied;
- native workspace tests;
- native workspace build;
- Zed extension formatting;
- Zed extension clippy for `wasm32-wasip2`;
- Zed extension check for `wasm32-wasip2`.

## Zed Extension Toolchain Preflight

Zed's Rust extension builder must be able to compile the extension to `wasm32-wasip2`.

Run:

```bash
rustup +stable target add wasm32-wasip2
rustup +stable target list --installed
cd editors/zed
cargo check --target wasm32-wasip2
cd ../..
```

Expected:

- `wasm32-wasip2` is listed for the stable toolchain.
- `cargo check --target wasm32-wasip2` succeeds inside `editors/zed`.

Important: the root workspace intentionally uses Rust 1.75.0. The Zed extension intentionally uses stable because `wasm32-wasip2` is not the root native target.

## Build Local Binaries

Build the dev binaries that the extension and optional MCP test will use:

```bash
cargo build -p local-history-sidecar -p local-history-cli -p local-history-mcp
```

Expected binaries:

```text
target/debug/local-history
target/debug/local-history-sidecar
target/debug/local-history-mcp
```

Verify they are on `PATH` before Zed/manual tests:

```bash
command -v local-history local-history-sidecar local-history-mcp
```

## Launch Zed With The Correct Environment

Create a clean manual test project:

```bash
mkdir -p "$TEST_PROJECT"
printf 'v1\n' > "$TEST_PROJECT/note.txt"
```

Launch Zed from the shell with both rustup and the local debug binaries visible:

```bash
RUSTUP_TOOLCHAIN=stable \
PATH="$HOME/.cargo/bin:$REPO/target/debug:$PATH" \
zed --foreground "$TEST_PROJECT"
```

This launch form matters for dev extension testing. It prevents Zed from accidentally trying to build the extension with the root 1.75.0 toolchain or from missing the locally built `local-history-sidecar` and `local-history-mcp`.

In the same shell, before starting Zed:

```bash
command -v local-history-sidecar local-history-mcp
```

If either command prints nothing, fix `PATH` on the **same line** as the `zed` launch command.

## Install The Dev Extension

In Zed:

1. Open Extensions.
2. Choose `Install Dev Extension`.
3. Select:

```text
$REPO/editors/zed
```

Expected:

- the dev extension installs successfully;
- Zed compiles the Rust extension without a `wasm32-wasip2` error;
- **Local History** appears in the installed extensions list (version `0.1.0`).

Installing the dev extension does **not** by itself prove that slash commands are reachable in your Zed UI. Continue with [Sidecar CLI fallback](#sidecar-cli-fallback-zed-144-without-text-thread) on Zed 1.4.4 builds without **New Text Thread**.

## Sidecar CLI Fallback (Zed 1.4.4+ Without Text Thread)

Run from the Zed integrated terminal or any shell where `PATH` includes `$REPO/target/debug`:

```bash
local-history-sidecar status "$TEST_PROJECT"
```

Expected:

- output includes `project_root`;
- output includes `project_id`;
- output includes `view_root`;
- watcher may be inactive before it is started.

Start the watcher:

```bash
local-history-sidecar ensure-daemon "$TEST_PROJECT"
```

Expected:

- watcher becomes active, or a new process is reported with a PID;
- output includes `view_root` and `log_path`.

Confirm:

```bash
local-history-sidecar status "$TEST_PROJECT"
```

Expected:

- watcher is active for `$TEST_PROJECT`.

These commands are what extension slash handlers call internally (`status` / `ensure-daemon`).

## Slash Command Smoke Test (Optional — Text Thread Only)

Skip this section if **New Text Thread** is unavailable and `/local-history-*` never appears in completions. Use [Sidecar CLI fallback](#sidecar-cli-fallback-zed-144-without-text-thread) instead.

Prerequisites:

- dev extension installed;
- project folder open as a worktree (`$TEST_PROJECT`, not a lone file without a project root);
- a **Text Thread** open (see [Where Extension Slash Commands Work](#where-extension-slash-commands-work-zed-144)).

In the text-thread editor buffer, at the **beginning of a line**, run:

```text
/local-history-status
```

Expected:

- output is inserted into the thread (not returned as an Agent LLM reply);
- output includes `project_root`, `project_id`, and `view_root`;
- watcher may be inactive before it is started.

Run:

```text
/local-history-start-watcher
```

Expected:

- output includes `watcher_active: true`, or a newly started process with a PID;
- output includes `view_root` and `log_path`.

Run status again:

```text
/local-history-status
```

Expected:

- watcher is active for `$TEST_PROJECT`.

## Snapshot Capture Test

In Zed, open:

```text
$TEST_PROJECT/note.txt
```

Change the file to:

```text
v2
```

Save. Wait 2-3 seconds.

Change the file to:

```text
v3
```

Save. Wait 2-3 seconds.

From the Zed terminal or a normal shell (with `PATH` set):

```bash
local-history recent "$TEST_PROJECT"
```

Expected:

- snapshots exist for `note.txt`;
- at least one snapshot contains an older saved state, such as `v1` or `v2`.

## Markdown View Test

### Option A — sidecar / CLI (recommended on Zed 1.4.4)

```bash
local-history-sidecar render-markdown current-hour "$TEST_PROJECT"
```

Or:

```bash
local-history render-markdown hour "$TEST_PROJECT" --hour "$(date -u +%Y-%m-%dT%H)"
```

Expected:

- output includes a `markdown_path` or equivalent path field;
- the Markdown file exists on disk under the project `view_root`.

### Option B — extension slash (Text Thread only)

```text
/local-history-current-hour
```

Expected:

- output includes `markdown_path` and `view_root`;
- the Markdown file exists on disk.

Open the returned path manually (File → Open, or Command Palette → open file). The extension cannot open arbitrary external paths through the current Zed extension API.

## Markdown Navigation And Restore Test

This test validates the user-facing Markdown browsing model, not only that a file was generated.

Print the generated view root:

```bash
local-history view-root "$TEST_PROJECT"
```

Expected:

- output is an absolute path under the local-history data directory;
- the path is outside `$TEST_PROJECT`;
- the directory contains a generated `README.md` after render or rebuild.

Rebuild the full Markdown view:

```bash
local-history rebuild-markdown-view "$TEST_PROJECT"
```

Open the generated root `README.md` in Zed or a pager:

```bash
local-history view-root "$TEST_PROJECT"
```

Then open:

```text
<view-root>/README.md
```

Expected:

- the root Markdown page links to day/hour history pages;
- an hour page links to fixed 10-minute segment pages;
- a segment page links to exact snapshot pages for `note.txt`;
- generated links use absolute local paths under `view_root`, so clicking from Zed opens the target file instead of resolving relative to `$TEST_PROJECT`;
- an exact snapshot page includes file path, timestamp, full snapshot ID, restore command, and text preview when the snapshot is text.

Copy the restore command from one exact snapshot page and run it from a shell. It should look like:

```bash
local-history restore <snapshot-id>
```

Expected:

- `note.txt` changes to the snapshot content shown or implied by the page;
- restore output includes a `safety_snapshot_id`;
- `local-history undo-restore "$TEST_PROJECT"` returns `note.txt` to the pre-restore state.

Important interpretation:

- Markdown files are a generated browsing view, not the source of truth.
- Deleting `view/` should not delete history; `local-history rebuild-markdown-view "$TEST_PROJECT"` should recreate it from stored snapshots.
- After pruning, old Markdown links may point to snapshots that no longer exist. Confirm with `local-history show <snapshot-id-or-unique-prefix>`.

## Restore Test

Copy a snapshot ID prefix from:

```bash
local-history recent "$TEST_PROJECT"
```

Or a full snapshot ID from:

```bash
local-history recent "$TEST_PROJECT" --json
```

Before restoring, inspect the code-level change against the current live file:

```bash
local-history diff <snapshot-id-or-unique-prefix>
```

Expected:

- output is a unified text diff;
- `--- snapshot:<id>:note.txt` represents the stored previous state;
- `+++ current:<path>` represents the current live file.

### Option A — CLI (recommended on Zed 1.4.4)

```bash
local-history restore <snapshot-id-or-unique-prefix>
```

Or by recent-list position:

```bash
local-history restore --project-root "$TEST_PROJECT" --recent 1
```

### Option B — extension slash (Text Thread only)

```text
/local-history-restore <snapshot-id-or-unique-prefix>
```

### Expected (both options)

- `note.txt` changes back to the selected snapshot content;
- restore output includes `restored_snapshot_id`, `safety_snapshot_id`, and `restore_operation_id` (field names may be JSON keys when using sidecar).

Verify undo from shell:

```bash
local-history undo-restore "$TEST_PROJECT"
```

Expected:

- `note.txt` returns to the state it had before the restore.

## MCP Test In Zed Agent

This section is for Zed Agent Panel hosts that expose `local_history_*` MCP tools. For shell-only agents without MCP tools, use [CLI-only Agent Testing](#cli-only-agent-testing) instead.

The Zed Agent Panel uses MCP tools. It does **not** run extension slash commands such as `/local-history-status` from the agent chat input, even when the input hint mentions `/ for commands`.

The dev extension registers the `local-history` context server automatically. During local dev it prefers `local-history-mcp` from `PATH`; in the packaged release path it should download and cache the matching MCP release binary.

Prerequisites for Agent chat:

- select a model in the Agent Panel model picker (`No Model Selected` blocks sending messages);
- MCP server running (extension-managed or explicit settings below).

If extension-managed registration does not start the server, add this to Zed settings:

```json
{
  "context_servers": {
    "local-history": {
      "command": "/absolute/path/to/zed-local-history/target/debug/local-history-mcp",
      "args": []
    }
  }
}
```

Replace the `command` path with your `$REPO/target/debug/local-history-mcp` during dev testing.

In the Zed Agent Panel (default **Zed Agent** thread is correct here), ask:

```text
Show local-history status for /tmp/lh-zed-manual
```

Use your real `$TEST_PROJECT` path in the prompt.

Expected:

- the MCP server responds through a tool call;
- status matches the same project root as `local-history-sidecar status`.

Then verify the agent-facing guide is visible through MCP:

```text
Use the local_history_guide tool to read the local-history MCP guide and explain how restore safety works.
```

Expected:

- the Agent can use the `local_history_guide` MCP tool;
- if the client exposes resources, the same guide is also available as `local-history://guide`;
- the answer explains that restore creates a safety snapshot before modifying the live file;
- the answer does not claim that generated Markdown is the source of truth.

Then verify snapshot diff through MCP:

```text
Use local_history_diff_snapshot to show the unified diff between the latest raw snapshot for note.txt and the current live file in /tmp/lh-zed-manual.
```

Use your real `$TEST_PROJECT` path and a file that has both a stored snapshot and a different live state.

Expected:

- the Agent calls `local_history_diff_snapshot` (or uses the equivalent MCP tool flow);
- structured output includes `diff` and `unchanged`;
- the diff direction is snapshot → current live file;
- restore is not performed during the diff call.

## Troubleshooting The 2026-05-30 First Manual Run

Observed log:

```text
ERROR [crates/project/src/git_store.rs:7843] opening repository at "/tmp/.git"
ERROR [crates/git_ui/src/git_panel.rs:3638] oneshot canceled
ERROR [gpui_linux::linux::wayland::client] activation token received with no pending activation
ERROR [agent] Failed to authenticate provider: ChatGPT Subscription: Sign in with your ChatGPT Plus or Pro subscription to use this provider.
ERROR [extension::extension_builder] failed to compile Rust extension: failed to install the `wasm32-wasip2` target
ERROR [extensions_ui] Failed to install dev extension: failed to compile Rust extension
```

Interpretation:

- `opening repository at "/tmp/.git"` is Zed Git discovery noise from opening a project under `/tmp`. It is not a `zed-local-history` failure. If `/tmp/.git` exists and is broken, remove or fix it separately.
- `oneshot canceled` is secondary Git UI noise after the Git discovery failure.
- `activation token received with no pending activation` is Wayland window activation noise.
- `Failed to authenticate provider: ChatGPT Subscription` is unrelated to this extension unless testing Agent Panel provider auth and sending agent messages.
- `failed to install the wasm32-wasip2 target` is the real blocker for installing the dev extension.

Fix for the real blocker:

```bash
rustup +stable target add wasm32-wasip2
rustup +stable target list --installed
cargo run -p xtask -- zed-ci
```

Then relaunch Zed with the stable Rust extension toolchain forced:

```bash
RUSTUP_TOOLCHAIN=stable \
PATH="$HOME/.cargo/bin:$REPO/target/debug:$PATH" \
zed --foreground "$TEST_PROJECT"
```

If the dev extension still fails:

1. Confirm `zed --version` is Zed 1.4.4 or newer stable.
2. Confirm `which cargo` inside the launch shell resolves under `$HOME/.cargo/bin`.
3. Confirm `cargo run -p xtask -- zed-ci` succeeds from the repository root.
4. Start Zed from the shell command above, not from a desktop launcher.
5. Retry `Install Dev Extension`.

### Common manual-testing mistakes

| Symptom | Likely cause | Fix |
|--------|----------------|-----|
| `No such file or directory` after typing `/local-history-status` in terminal | slash entered in bash, not in Zed Text Thread | use `local-history-sidecar status "$TEST_PROJECT"` |
| Agent says `/local-history-status` is unknown | command entered in **Zed Agent** chat, not Text Thread | use MCP prompt or sidecar CLI |
| Command Palette search `text thread` only finds `multi workspace: next thread` | wrong palette query | search `New Text Thread` or use sidecar CLI |
| `+` menu has no **New Text Thread** | Zed 1.4.4 UI may omit text threads | sidecar CLI + MCP; optional `agent.default_view` |
| MCP agent does nothing | no model selected or MCP not configured | pick a model; verify `context_servers` / extension MCP bootstrap |
| Watcher never snapshots | watcher not started | `local-history-sidecar ensure-daemon "$TEST_PROJECT"` |
| Extension slash: "must run inside an opened Zed worktree" | no project folder open | open `$TEST_PROJECT` as a folder in Zed |

## Tagged Release Validation

The dev-extension flow above uses the locally built `local-history-sidecar` and `local-history-mcp` from `PATH`.

It does not validate GitHub Release bootstrap.

After creating a real tag such as `v0.1.0`, validate separately:

1. GitHub Actions release workflow completes.
2. Release assets include platform bundles with:
   - `local-history`;
   - `local-history-sidecar`;
   - `local-history-mcp`;
   - `README.md`;
   - `LICENSE`.
3. Release assets include sidecar-only and MCP-only archives used by the extension bootstrap.
4. `SHA256SUMS.txt` is published.
5. Install the dev extension without `target/debug` in `PATH`.
6. Verify slash commands download the matching sidecar archive (only where Text Thread slash UI exists).
7. Verify Agent Panel MCP startup downloads the matching MCP archive.

## MVP Acceptance

The MVP manual acceptance passes on Zed **1.4.4 or newer** when all of the following are true.

Required (always):

- dev extension installs;
- watcher starts via **`local-history-sidecar ensure-daemon`** or `/local-history-start-watcher` when Text Thread slash is available;
- saving `note.txt` creates snapshots visible in `local-history recent`;
- current-hour Markdown is reachable via **sidecar/CLI** or `/local-history-current-hour` when slash is available;
- restore works via **CLI** or `/local-history-restore` when slash is available;
- restore creates a safety snapshot;
- CLI `undo-restore` reverses the restore;
- extension-managed MCP startup or explicit `context_servers` setup works in the Zed Agent Panel (with a model selected).

Optional (when Zed exposes Text Thread + extension slash completions):

- `/local-history-status` and `/local-history-start-watcher` in a Text Thread;
- `/local-history-current-hour` and `/local-history-restore` in a Text Thread.

On Zed 1.4.4 builds whose **`+` menu omits New Text Thread**, passing the required sidecar + MCP + CLI checks is sufficient for MVP acceptance.
