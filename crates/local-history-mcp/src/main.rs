use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use local_history_core::{
    default_data_dir, format_timestamp_local, init_local_offset_detection, normalize_project_root,
    snapshot_to_current_unified_diff, LocalHistoryStore, RestoreOutcome, RetentionPolicy,
    SnapshotId, SnapshotKind, SnapshotPage, SnapshotQuery, SnapshotRecord, SnapshotWriteRequest,
    StorageLayout,
};
use serde::Deserialize;
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, PrimitiveDateTime};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "local-history-mcp";
const WATCHER_STALE_AFTER_SECONDS: u64 = 5;
const AGENT_GUIDE_URI: &str = "local-history://guide";
const AGENT_GUIDE_TEXT: &str = include_str!("../../../llms.txt");
const SERVER_INSTRUCTIONS: &str = "\
Use these tools for filesystem-first local history recovery when the client exposes them.

If the client does not expose local_history_* tools but can run shell commands, use the CLI equivalents documented in local_history_guide / local-history://guide instead of guessing behavior.

Intent mapping (see the guide for the full table):
- Questions like \"what changed\", \"summary of changes\", \"edit history\", or \"history of edits\" when Git is unavailable or the user means saved editor states: start with recent snapshots, then diff or view the relevant snapshot against the current live file.
- \"Recover\", \"restore\", \"roll back\", or \"previous version\": recent -> view -> restore only after the target is clear.
- \"Compare\", \"diff\", \"what is different now\": local_history_diff_snapshot or CLI diff.
- \"Is history working\" / \"are snapshots saved\": local_history_status first.
- \"Checkpoint now\" / \"snapshot before edit\": local_history_create_snapshot.
- Prefer Git only when the user clearly wants commits, branches, or repository history.

Core rules:
- Most tools require an explicit absolute project_root.
- Local history stores data outside the repository; generated Markdown is only a rebuildable browsing layer.
- Raw snapshots store the previous known file state from before a save/delete, not the newly saved content.
- Snapshot IDs are opaque. Short prefixes are accepted only when unique.
- Restore is state-changing and always creates a safety snapshot first.
- Prefer local_history_view_snapshot before restoring unless the user already chose an exact snapshot.
- Use local_history_diff_snapshot for unified text diff from snapshot to the current live file before restore when code-level inspection is needed.
- Report the safety_snapshot_id after restore so the user knows the recovery point.
- Prune is state-changing and can remove old snapshots according to retention policy.
- Default to meaningful user-facing answers: after recent snapshots, enrich with view previews and/or diff summaries when that helps the question. If the user explicitly wants a compact index, IDs only, or raw tool output, honor that instead.
- When listing snapshots, include brief content previews or change summaries when useful; explain the pre-save previous-state model when relevant.
- For local_history_recent_snapshots, default presentation is rich (timestamp, path, id, one-line content preview). Pass presentation=ids_only when the user wants only snapshot IDs; pass presentation=index for timestamp/path/id without previews.

If you need the full operating model, call the read-only local_history_guide tool or read the MCP resource local-history://guide.";

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--help") | Some("-h") => {
            print_help();
            return;
        }
        Some("--version") | Some("-V") => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some(argument) => {
            eprintln!("unsupported argument: {argument}");
            std::process::exit(2);
        }
        None => {}
    }

    init_local_offset_detection();

    if let Err(error) = run_stdio_server() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_stdio_server() -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let line = line.map_err(|error| format!("failed to read stdin: {error}"))?;

        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle_message(message),
            Err(error) => Some(jsonrpc_error(
                Value::Null,
                -32700,
                format!("failed to parse JSON-RPC message: {error}"),
            )),
        };

        if let Some(response) = response {
            writeln!(
                stdout,
                "{}",
                serde_json::to_string(&response)
                    .map_err(|error| format!("failed to serialize JSON-RPC response: {error}"))?
            )
            .map_err(|error| format!("failed to write JSON-RPC response: {error}"))?;
            stdout
                .flush()
                .map_err(|error| format!("failed to flush stdout: {error}"))?;
        }
    }

    Ok(())
}

fn handle_message(message: Value) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(jsonrpc_error(
            Value::Null,
            -32600,
            "expected a JSON-RPC object".to_string(),
        ));
    };

    let id = object.get("id").cloned().unwrap_or(Value::Null);

    if object.get("jsonrpc") != Some(&Value::String("2.0".to_string())) {
        return Some(jsonrpc_error(
            id,
            -32600,
            "expected `jsonrpc` to equal `2.0`".to_string(),
        ));
    }

    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(jsonrpc_error(
            id,
            -32600,
            "missing JSON-RPC method".to_string(),
        ));
    };

    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "initialize" => Some(handle_initialize(id)),
        "notifications/initialized" => None,
        "ping" => Some(jsonrpc_success(id, json!({}))),
        "tools/list" => Some(handle_tools_list(id)),
        "tools/call" => Some(handle_tools_call(id, &params)),
        "resources/list" => Some(handle_resources_list(id)),
        "resources/read" => Some(handle_resources_read(id, &params)),
        _ if object.contains_key("id") => Some(jsonrpc_error(
            id,
            -32601,
            format!("method not found: {method}"),
        )),
        _ => None,
    }
}

fn handle_initialize(id: Value) -> Value {
    jsonrpc_success(
        id,
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": {
                    "listChanged": false,
                },
                "resources": {
                    "subscribe": false,
                    "listChanged": false,
                }
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": env!("CARGO_PKG_VERSION"),
            },
            "instructions": SERVER_INSTRUCTIONS,
        }),
    )
}

fn handle_resources_list(id: Value) -> Value {
    jsonrpc_success(
        id,
        json!({
            "resources": [
                {
                    "uri": AGENT_GUIDE_URI,
                    "name": "Local History Agent Guide",
                    "description": "Complete operating guide for using zed-local-history safely through MCP, CLI, Markdown, and Zed.",
                    "mimeType": "text/markdown",
                }
            ]
        }),
    )
}

fn handle_resources_read(id: Value, params: &Value) -> Value {
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return jsonrpc_error(
            id,
            -32602,
            "resources/read requires a `uri` string".to_string(),
        );
    };

    if uri != AGENT_GUIDE_URI {
        return jsonrpc_error(id, -32602, format!("unknown resource uri: {uri}"));
    }

    jsonrpc_success(
        id,
        json!({
            "contents": [
                {
                    "uri": AGENT_GUIDE_URI,
                    "mimeType": "text/markdown",
                    "text": AGENT_GUIDE_TEXT,
                }
            ]
        }),
    )
}

fn handle_tools_list(id: Value) -> Value {
    jsonrpc_success(
        id,
        json!({
            "tools": [
                tool_local_history_guide(),
                tool_local_history_status(),
                tool_local_history_create_snapshot(),
                tool_local_history_recent_snapshots(),
                tool_local_history_view_snapshot(),
                tool_local_history_diff_snapshot(),
                tool_local_history_restore_snapshot(),
                tool_local_history_prune(),
            ]
        }),
    )
}

fn handle_tools_call(id: Value, params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return jsonrpc_error(
            id,
            -32602,
            "tools/call requires a `name` string".to_string(),
        );
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let result = match name {
        "local_history_guide" => tool_call_result(Ok(tool_guide())),
        "local_history_status" => tool_call_result(tool_status(&arguments)),
        "local_history_create_snapshot" => tool_call_result(tool_create_snapshot(&arguments)),
        "local_history_recent_snapshots" => tool_call_result(tool_recent_snapshots(&arguments)),
        "local_history_view_snapshot" => tool_call_result(tool_view_snapshot(&arguments)),
        "local_history_diff_snapshot" => tool_call_result(tool_diff_snapshot(&arguments)),
        "local_history_restore_snapshot" => tool_call_result(tool_restore_snapshot(&arguments)),
        "local_history_prune" => tool_call_result(tool_prune(&arguments)),
        _ => return jsonrpc_error(id, -32601, format!("tool not found: {name}")),
    };

    jsonrpc_success(id, result)
}

fn tool_guide() -> Value {
    json!({
        "summary": "Returned the local-history agent operating guide.",
        "uri": AGENT_GUIDE_URI,
        "mime_type": "text/markdown",
        "text": AGENT_GUIDE_TEXT,
    })
}

fn tool_call_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(structured) => {
            let summary = structured
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or("local-history tool call completed");

            json!({
                "content": [
                    {
                        "type": "text",
                        "text": summary,
                    }
                ],
                "structuredContent": structured,
            })
        }
        Err(error) => json!({
            "content": [
                {
                    "type": "text",
                    "text": format!("local-history tool error: {error}"),
                }
            ],
            "structuredContent": {
                "error": error,
            },
            "isError": true,
        }),
    }
}

fn tool_status(arguments: &Value) -> Result<Value, String> {
    let project_root = required_project_root(arguments)?;
    let data_dir = data_dir_from_arguments(arguments)?;
    let store =
        LocalHistoryStore::open(data_dir, project_root).map_err(|error| error.to_string())?;
    let watcher = watcher_status(store.layout())?;
    let total_snapshot_count = store
        .total_snapshot_count()
        .map_err(|error| error.to_string())?;
    let raw_snapshot_count = store
        .raw_snapshot_count()
        .map_err(|error| error.to_string())?;
    let safety_snapshot_count = store
        .safety_snapshot_count()
        .map_err(|error| error.to_string())?;
    let referenced_blob_bytes = store
        .referenced_blob_bytes()
        .map_err(|error| error.to_string())?;
    let retention = store.retention_policy();

    Ok(json!({
        "summary": format!(
            "Local history for {}: {} total snapshots ({} raw, {} safety). Watcher active: {}.",
            store.project().root.display(),
            total_snapshot_count,
            raw_snapshot_count,
            safety_snapshot_count,
            watcher.get("active").and_then(Value::as_bool).unwrap_or(false)
        ),
        "project_id": store.project().id.as_str(),
        "project_root": store.project().root.display().to_string(),
        "data_dir": store.layout().project_dir.display().to_string(),
        "database_path": store.layout().database_path.display().to_string(),
        "view_root": store.layout().view_dir.display().to_string(),
        "logs_dir": store.layout().logs_dir.display().to_string(),
        "total_snapshot_count": total_snapshot_count,
        "raw_snapshot_count": raw_snapshot_count,
        "safety_snapshot_count": safety_snapshot_count,
        "referenced_blob_bytes": referenced_blob_bytes,
        "retention_policy": retention_json(&retention),
        "watcher": watcher,
    }))
}

fn tool_create_snapshot(arguments: &Value) -> Result<Value, String> {
    let project_root = required_project_root(arguments)?;
    let data_dir = data_dir_from_arguments(arguments)?;
    let relative_path = required_relative_path(arguments, "relative_path")?;
    let store =
        LocalHistoryStore::open(data_dir, project_root).map_err(|error| error.to_string())?;
    let absolute_path = store.project().root.join(&relative_path);
    let contents = fs::read(&absolute_path)
        .map_err(|error| format!("failed to read {}: {error}", absolute_path.display()))?;
    let snapshot = store
        .store_snapshot(SnapshotWriteRequest {
            relative_path: relative_path.clone(),
            contents: contents.clone(),
            timestamp: current_timestamp()?,
            kind: SnapshotKind::Raw,
            is_binary: std::str::from_utf8(&contents).is_err(),
            captures_missing_file: false,
        })
        .map_err(|error| error.to_string())?;

    Ok(json!({
        "summary": format!(
            "Stored raw snapshot {} for {}.",
            short_id(snapshot.id.as_str()),
            snapshot.relative_path.display()
        ),
        "snapshot": snapshot_json(&snapshot),
    }))
}

fn tool_recent_snapshots(arguments: &Value) -> Result<Value, String> {
    let project_root = required_project_root(arguments)?;
    let data_dir = data_dir_from_arguments(arguments)?;
    let limit = optional_usize(arguments, "limit")?.unwrap_or(10);
    let include_safety = optional_bool(arguments, "include_safety")?.unwrap_or(false);
    let presentation = optional_presentation(arguments)?.unwrap_or(RecentPresentation::Rich);
    let store =
        LocalHistoryStore::open(data_dir, project_root).map_err(|error| error.to_string())?;
    let query = snapshot_query_from_arguments(arguments, include_safety, limit)?;
    let snapshots = if query.is_none() && !include_safety {
        store
            .recent_raw_snapshots(limit)
            .map_err(|error| error.to_string())?
    } else if query.is_none() {
        store
            .recent_snapshots(limit)
            .map_err(|error| error.to_string())?
    } else {
        let page = store
            .query_snapshots(&query.clone().expect("query checked above"))
            .map_err(|error| error.to_string())?;
        return recent_page_json(&store, &page, include_safety, presentation);
    };

    Ok(json!({
        "summary": recent_summary(&store, &snapshots, include_safety, presentation)?,
        "project_id": store.project().id.as_str(),
        "project_root": store.project().root.display().to_string(),
        "limit": limit,
        "include_safety": include_safety,
        "presentation": presentation.as_str(),
        "snapshots": snapshots
            .iter()
            .map(|snapshot| snapshot_json_with_presentation(&store, snapshot, presentation))
            .collect::<Result<Vec<_>, _>>()?,
    }))
}

fn tool_view_snapshot(arguments: &Value) -> Result<Value, String> {
    let (store, snapshot_id, snapshot) = open_snapshot_from_arguments(arguments)?;
    let contents = store
        .read_snapshot_content(&snapshot_id)
        .map_err(|error| error.to_string())?;
    let preview = render_snapshot_preview(&snapshot, &contents);

    Ok(json!({
        "summary": format!(
            "Snapshot {} for {} at {}.",
            short_id(snapshot.id.as_str()),
            snapshot.relative_path.display(),
            snapshot.timestamp
        ),
        "project_id": store.project().id.as_str(),
        "project_root": store.project().root.display().to_string(),
        "snapshot": snapshot_json(&snapshot),
        "restore_command": format!("local-history restore {}", snapshot.id.as_str()),
        "preview": preview,
    }))
}

fn tool_diff_snapshot(arguments: &Value) -> Result<Value, String> {
    let (store, snapshot_id, snapshot) = open_snapshot_from_arguments(arguments)?;
    let snapshot_contents = store
        .read_snapshot_content(&snapshot_id)
        .map_err(|error| error.to_string())?;
    let live_path = store.project().root.join(&snapshot.relative_path);
    let diff = snapshot_to_current_unified_diff(&snapshot, &snapshot_contents, &live_path)
        .map_err(|error| error.to_string())?;
    let unchanged = diff == "no changes\n";

    Ok(json!({
        "summary": format!(
            "Diff from snapshot {} for {} to current {}.",
            short_id(snapshot.id.as_str()),
            snapshot.relative_path.display(),
            live_path.display()
        ),
        "project_id": store.project().id.as_str(),
        "project_root": store.project().root.display().to_string(),
        "snapshot": snapshot_json(&snapshot),
        "live_path": live_path.display().to_string(),
        "unchanged": unchanged,
        "diff": diff,
    }))
}

fn tool_restore_snapshot(arguments: &Value) -> Result<Value, String> {
    let (store, snapshot_id, _) = open_snapshot_from_arguments(arguments)?;
    let outcome = store
        .restore_snapshot(&snapshot_id, &current_timestamp()?)
        .map_err(|error| error.to_string())?;

    Ok(restore_outcome_json(
        &outcome,
        &store.project().root,
        "Restored snapshot with a new safety snapshot.",
    ))
}

fn tool_prune(arguments: &Value) -> Result<Value, String> {
    let project_root = required_project_root(arguments)?;
    let data_dir = data_dir_from_arguments(arguments)?;
    let store =
        LocalHistoryStore::open(data_dir, project_root).map_err(|error| error.to_string())?;
    let retention = store.retention_policy();
    let report = store
        .prune(&retention, &current_timestamp()?)
        .map_err(|error| error.to_string())?;

    Ok(json!({
        "summary": format!(
            "Pruned local history for {}: deleted {} snapshots and {} restore-operation rows.",
            store.project().root.display(),
            report.deleted_snapshot_count,
            report.deleted_restore_operation_count
        ),
        "project_id": store.project().id.as_str(),
        "project_root": store.project().root.display().to_string(),
        "retention_policy": retention_json(&retention),
        "report": prune_report_json(&report),
    }))
}

fn snapshot_query_from_arguments(
    arguments: &Value,
    include_safety: bool,
    limit: usize,
) -> Result<Option<SnapshotQuery>, String> {
    let relative_path = optional_relative_path(arguments, "relative_path")?;
    let from = optional_string(arguments, "from_timestamp")?;
    let to = optional_string(arguments, "to_timestamp")?;
    let hour = optional_string(arguments, "hour")?;

    if relative_path.is_none() && from.is_none() && to.is_none() && hour.is_none() {
        return Ok(None);
    }

    let (from_timestamp, to_timestamp) = resolve_time_filters(&from, &to, &hour)?;

    Ok(Some(SnapshotQuery {
        relative_path,
        from_timestamp,
        to_timestamp,
        kind: if include_safety {
            None
        } else {
            Some(SnapshotKind::Raw)
        },
        page: 1,
        page_size: limit,
    }))
}

fn resolve_time_filters(
    from: &Option<String>,
    to: &Option<String>,
    hour: &Option<String>,
) -> Result<(Option<String>, Option<String>), String> {
    if let Some(hour) = hour {
        if from.is_some() || to.is_some() {
            return Err(
                "`hour` cannot be combined with `from_timestamp` or `to_timestamp`".to_string(),
            );
        }

        let hour_start = PrimitiveDateTime::parse(
            hour,
            &time::macros::format_description!("[year]-[month]-[day]T[hour]"),
        )
        .map_err(|error| format!("invalid ISO hour `{hour}`: {error}"))?
        .assume_utc();
        let hour_end = hour_start + Duration::hours(1);

        return Ok((
            Some(
                hour_start
                    .format(&Rfc3339)
                    .map_err(|error| format!("failed to format hour start: {error}"))?,
            ),
            Some(
                hour_end
                    .format(&Rfc3339)
                    .map_err(|error| format!("failed to format hour end: {error}"))?,
            ),
        ));
    }

    Ok((from.clone(), to.clone()))
}

fn watcher_status(layout: &StorageLayout) -> Result<Value, String> {
    let status_path = layout.logs_dir.join("watcher-status.json");

    if !status_path.is_file() {
        return Ok(json!({
            "active": false,
        }));
    }

    let bytes = fs::read(&status_path).map_err(|error| {
        format!(
            "failed to read watcher status {}: {error}",
            status_path.display()
        )
    })?;
    let record: WatcherStatusRecord = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "failed to parse watcher status {}: {error}",
            status_path.display()
        )
    })?;
    let active = current_unix_seconds().saturating_sub(record.heartbeat_unix_seconds)
        <= WATCHER_STALE_AFTER_SECONDS;

    Ok(json!({
        "active": active,
        "pid": record.pid,
        "started_at": record.started_at,
        "heartbeat_at": record.heartbeat_at,
        "heartbeat_unix_seconds": record.heartbeat_unix_seconds,
        "watched_files": record.watched_files,
        "log_path": record.log_path.display().to_string(),
        "last_error": record.last_error,
    }))
}

fn restore_outcome_json(outcome: &RestoreOutcome, project_root: &Path, summary: &str) -> Value {
    json!({
        "summary": summary,
        "project_root": project_root.display().to_string(),
        "restored_snapshot": snapshot_json(&outcome.restored_snapshot),
        "safety_snapshot": snapshot_json(&outcome.safety_snapshot),
        "restore_operation": {
            "id": outcome.operation.id,
            "relative_path": outcome.operation.relative_path.display().to_string(),
            "restored_snapshot_id": outcome.operation.restored_snapshot_id.as_str(),
            "safety_snapshot_id": outcome.operation.safety_snapshot_id.as_str(),
            "previous_file_existed": outcome.operation.previous_file_existed,
            "previous_content_hash": outcome.operation.previous_content_hash.as_str(),
            "restored_content_hash": outcome.operation.restored_content_hash.as_str(),
            "timestamp": outcome.operation.timestamp,
        },
        "restored_path": outcome.restored_path.display().to_string(),
    })
}

fn recent_page_json(
    store: &LocalHistoryStore,
    page: &SnapshotPage,
    include_safety: bool,
    presentation: RecentPresentation,
) -> Result<Value, String> {
    Ok(json!({
        "summary": recent_summary(store, &page.items, include_safety, presentation)?,
        "project_id": store.project().id.as_str(),
        "project_root": store.project().root.display().to_string(),
        "include_safety": include_safety,
        "presentation": presentation.as_str(),
        "page": page.page,
        "page_size": page.page_size,
        "total_items": page.total_items,
        "total_pages": page.total_pages,
        "snapshots": page
            .items
            .iter()
            .map(|snapshot| snapshot_json_with_presentation(store, snapshot, presentation))
            .collect::<Result<Vec<_>, _>>()?,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecentPresentation {
    Rich,
    Index,
    IdsOnly,
}

impl RecentPresentation {
    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "rich" => Ok(Self::Rich),
            "index" => Ok(Self::Index),
            "ids_only" => Ok(Self::IdsOnly),
            other => Err(format!(
                "unsupported presentation `{other}`; expected rich, index, or ids_only"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Rich => "rich",
            Self::Index => "index",
            Self::IdsOnly => "ids_only",
        }
    }
}

fn recent_summary(
    store: &LocalHistoryStore,
    snapshots: &[SnapshotRecord],
    include_safety: bool,
    presentation: RecentPresentation,
) -> Result<String, String> {
    if snapshots.is_empty() {
        return Ok("No matching snapshots were found.".to_string());
    }

    match presentation {
        RecentPresentation::IdsOnly => {
            let mut lines = vec!["| Snapshot ID |".to_string(), "|---|".to_string()];
            for snapshot in snapshots {
                lines.push(format!("| {} |", short_id(snapshot.id.as_str())));
            }
            Ok(lines.join("\n"))
        }
        RecentPresentation::Index | RecentPresentation::Rich => {
            let mut lines = vec![if include_safety {
                "Recent snapshots:".to_string()
            } else {
                "Recent raw snapshots (previous-state captures):".to_string()
            }];

            for (index, snapshot) in snapshots.iter().enumerate() {
                let kind_suffix = if snapshot.kind == SnapshotKind::Safety {
                    " [safety]"
                } else {
                    ""
                };
                let missing_suffix = if snapshot.captures_missing_file {
                    " [missing]"
                } else {
                    ""
                };

                lines.push(format!(
                    "{}. {} — {} — {}{}{}",
                    index + 1,
                    format_timestamp_local(&snapshot.timestamp),
                    snapshot.relative_path.display(),
                    short_id(snapshot.id.as_str()),
                    kind_suffix,
                    missing_suffix
                ));

                if presentation == RecentPresentation::Rich {
                    let preview = snapshot_preview_line(store, snapshot)?;
                    lines.push(format!("   content: {preview}"));
                }
            }

            Ok(lines.join("\n"))
        }
    }
}

fn snapshot_preview_line(
    store: &LocalHistoryStore,
    snapshot: &SnapshotRecord,
) -> Result<String, String> {
    if snapshot.captures_missing_file {
        return Ok("[missing file state]".to_string());
    }

    let contents = store
        .read_snapshot_content(&snapshot.id)
        .map_err(|error| error.to_string())?;

    if contents.is_empty() {
        return Ok("<empty file>".to_string());
    }

    let text = match std::str::from_utf8(&contents) {
        Ok(text) => text,
        Err(_) => return Ok(format!("[binary, {} bytes]", contents.len())),
    };

    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("<empty file>");
    const MAX_CHARS: usize = 100;

    if line.chars().count() > MAX_CHARS {
        Ok(format!(
            "{}...",
            line.chars().take(MAX_CHARS).collect::<String>()
        ))
    } else {
        Ok(line.to_string())
    }
}

fn snapshot_json_with_presentation(
    store: &LocalHistoryStore,
    snapshot: &SnapshotRecord,
    presentation: RecentPresentation,
) -> Result<Value, String> {
    let mut value = snapshot_json(snapshot);

    if presentation == RecentPresentation::Rich {
        value["preview_line"] = json!(snapshot_preview_line(store, snapshot)?);
    }

    Ok(value)
}

fn retention_json(policy: &RetentionPolicy) -> Value {
    json!({
        "max_snapshots_per_file": policy.max_snapshots_per_file,
        "max_project_storage_bytes": policy.max_project_storage_bytes,
        "max_file_size_bytes": policy.max_file_size_bytes,
        "max_snapshot_age_days": policy.max_snapshot_age_days,
    })
}

fn prune_report_json(report: &local_history_core::PruneReport) -> Value {
    json!({
        "pruned_at": report.pruned_at,
        "deleted_restore_operation_count": report.deleted_restore_operation_count,
        "deleted_snapshot_count": report.deleted_snapshot_count,
        "deleted_blob_count": report.deleted_blob_count,
        "deleted_blob_bytes": report.deleted_blob_bytes,
        "remaining_snapshot_count": report.remaining_snapshot_count,
        "remaining_referenced_blob_bytes": report.remaining_referenced_blob_bytes,
        "protected_snapshot_count": report.protected_snapshot_count,
        "pruned_for_age_count": report.pruned_for_age_count,
        "pruned_for_file_count": report.pruned_for_file_count,
        "pruned_for_storage_count": report.pruned_for_storage_count,
        "rebuilt_markdown_view": report.rebuilt_markdown_view,
    })
}

fn snapshot_json(snapshot: &SnapshotRecord) -> Value {
    json!({
        "id": snapshot.id.as_str(),
        "project_id": snapshot.project_id.as_str(),
        "relative_path": snapshot.relative_path.display().to_string(),
        "blob_hash": snapshot.blob_hash.as_str(),
        "size_bytes": snapshot.size_bytes,
        "timestamp": snapshot.timestamp,
        "kind": snapshot.kind.as_str(),
        "captures_missing_file": snapshot.captures_missing_file,
    })
}

fn render_snapshot_preview(snapshot: &SnapshotRecord, contents: &[u8]) -> Value {
    if snapshot.captures_missing_file {
        return json!({
            "kind": "missing",
            "text": "[missing file state]",
            "truncated": false,
        });
    }

    if contents.is_empty() {
        return json!({
            "kind": "text",
            "text": "<empty file>",
            "truncated": false,
        });
    }

    match std::str::from_utf8(contents) {
        Ok(text) => {
            let mut lines = text.lines();
            let mut preview = lines.by_ref().take(20).collect::<Vec<_>>().join("\n");
            let truncated = lines.next().is_some();

            if truncated {
                preview.push_str("\n... (truncated)");
            }

            if preview.is_empty() {
                preview = "<empty file>".to_string();
            }

            json!({
                "kind": "text",
                "text": preview,
                "truncated": truncated,
            })
        }
        Err(_) => json!({
            "kind": "binary",
            "text": "[binary content omitted]",
            "truncated": false,
        }),
    }
}

fn required_project_root(arguments: &Value) -> Result<PathBuf, String> {
    let raw = required_string(arguments, "project_root")?;
    let path = PathBuf::from(raw);

    if !path.is_dir() {
        return Err(format!(
            "project_root must point to an existing directory, got {}",
            path.display()
        ));
    }

    Ok(normalize_project_root(&path))
}

fn required_relative_path(arguments: &Value, key: &str) -> Result<PathBuf, String> {
    let raw = required_string(arguments, key)?;
    let path = PathBuf::from(raw);

    if path.is_absolute() {
        return Err(format!(
            "{key} must be relative to the project root, got {}",
            path.display()
        ));
    }

    Ok(path)
}

fn optional_relative_path(arguments: &Value, key: &str) -> Result<Option<PathBuf>, String> {
    optional_string(arguments, key)?
        .map(|raw| {
            let path = PathBuf::from(raw);

            if path.is_absolute() {
                Err(format!(
                    "{key} must be relative to the project root, got {}",
                    path.display()
                ))
            } else {
                Ok(path)
            }
        })
        .transpose()
}

fn data_dir_from_arguments(arguments: &Value) -> Result<PathBuf, String> {
    Ok(optional_string(arguments, "data_dir")?
        .map(PathBuf::from)
        .unwrap_or_else(default_data_dir))
}

fn required_snapshot_id(
    arguments: &Value,
    key: &str,
    data_dir: &Path,
) -> Result<SnapshotId, String> {
    resolve_snapshot_id(data_dir, required_string(arguments, key)?)
}

fn resolve_snapshot_id(data_dir: &Path, input: &str) -> Result<SnapshotId, String> {
    let snapshot_id = SnapshotId::new(input);

    if LocalHistoryStore::open_for_snapshot_id(data_dir, &snapshot_id)
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Ok(snapshot_id);
    }

    let matches = LocalHistoryStore::find_snapshot_ids_by_prefix(data_dir, input)
        .map_err(|error| error.to_string())?;

    match matches.as_slice() {
        [] => Err(format!("snapshot not found: {input}")),
        [snapshot_id] => Ok(snapshot_id.clone()),
        _ => Err(ambiguous_snapshot_prefix_error(input, &matches)),
    }
}

fn open_snapshot_from_arguments(
    arguments: &Value,
) -> Result<(LocalHistoryStore, SnapshotId, SnapshotRecord), String> {
    let data_dir = data_dir_from_arguments(arguments)?;
    let snapshot_id = required_snapshot_id(arguments, "snapshot_id", &data_dir)?;
    let store = LocalHistoryStore::open_for_snapshot_id(&data_dir, &snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;
    let snapshot = store
        .snapshot(&snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;

    Ok((store, snapshot_id, snapshot))
}

fn ambiguous_snapshot_prefix_error(prefix: &str, matches: &[SnapshotId]) -> String {
    let mut message = format!(
        "snapshot prefix `{prefix}` is ambiguous; use a longer prefix or full snapshot ID:"
    );

    for snapshot_id in matches.iter().take(10) {
        message.push_str("\n  ");
        message.push_str(snapshot_id.as_str());
    }

    if matches.len() > 10 {
        message.push_str(&format!("\n  ... {} more matches", matches.len() - 10));
    }

    message
}

fn required_string<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing required string argument `{key}`"))
}

fn optional_string(arguments: &Value, key: &str) -> Result<Option<String>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
        Some(Value::String(_)) => Ok(None),
        Some(_) => Err(format!("expected `{key}` to be a string")),
    }
}

fn optional_bool(arguments: &Value, key: &str) -> Result<Option<bool>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("expected `{key}` to be a boolean")),
    }
}

fn optional_usize(arguments: &Value, key: &str) -> Result<Option<usize>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .map(|value| Some(value as usize))
            .ok_or_else(|| format!("expected `{key}` to be a non-negative integer")),
        Some(_) => Err(format!("expected `{key}` to be a non-negative integer")),
    }
}

fn optional_presentation(arguments: &Value) -> Result<Option<RecentPresentation>, String> {
    optional_string(arguments, "presentation")?
        .map(|value| RecentPresentation::from_str(value.trim()))
        .transpose()
}

fn current_timestamp() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("failed to format timestamp: {error}"))
}

fn short_id(value: &str) -> &str {
    &value[..std::cmp::min(value.len(), 8)]
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn jsonrpc_success(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn jsonrpc_error(id: Value, code: i64, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

fn tool_local_history_guide() -> Value {
    json!({
        "name": "local_history_guide",
        "title": "Local History Guide",
        "description": "Return the complete read-only operating guide for using local-history safely through MCP, CLI, Markdown, and Zed.",
        "inputSchema": {
            "type": "object",
            "properties": {},
            "additionalProperties": false
        },
        "outputSchema": {
            "type": "object",
            "properties": {
                "uri": { "type": "string" },
                "mime_type": { "type": "string" },
                "text": { "type": "string" }
            },
            "required": ["uri", "mime_type", "text"]
        },
        "annotations": {
            "title": "Local History Guide",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

fn tool_local_history_status() -> Value {
    json!({
        "name": "local_history_status",
        "title": "Local History Status",
        "description": "Show storage layout, retention policy, snapshot counts, and watcher status for a project root.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project_root": {
                    "type": "string",
                    "description": "Existing project root directory. Use an absolute path when possible."
                },
                "data_dir": {
                    "type": "string",
                    "description": "Optional local-history base data directory. Defaults to the standard platform-specific local-history directory."
                }
            },
            "required": ["project_root"]
        },
        "outputSchema": {
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "project_root": { "type": "string" },
                "total_snapshot_count": { "type": "integer" },
                "raw_snapshot_count": { "type": "integer" },
                "safety_snapshot_count": { "type": "integer" }
            },
            "required": ["project_id", "project_root", "total_snapshot_count", "raw_snapshot_count", "safety_snapshot_count"]
        },
        "annotations": {
            "title": "Local History Status",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

fn tool_local_history_create_snapshot() -> Value {
    json!({
        "name": "local_history_create_snapshot",
        "title": "Create Local Snapshot",
        "description": "Create a raw snapshot from the current on-disk contents of one file under a project root.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project_root": {
                    "type": "string",
                    "description": "Existing project root directory."
                },
                "data_dir": {
                    "type": "string",
                    "description": "Optional local-history base data directory."
                },
                "relative_path": {
                    "type": "string",
                    "description": "File path relative to the project root."
                }
            },
            "required": ["project_root", "relative_path"]
        },
        "annotations": {
            "title": "Create Local Snapshot",
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": false
        }
    })
}

fn tool_local_history_recent_snapshots() -> Value {
    json!({
        "name": "local_history_recent_snapshots",
        "title": "Recent Local Snapshots",
        "description": "List recent local-history snapshots for a project. Default presentation=rich returns timestamp, path, short id, and a one-line content preview in the text summary Zed shows to the agent. Use presentation=ids_only when the user wants only snapshot IDs; use presentation=index for timestamp/path/id without previews.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project_root": {
                    "type": "string",
                    "description": "Existing project root directory."
                },
                "data_dir": {
                    "type": "string",
                    "description": "Optional local-history base data directory."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of snapshots to return. Default: 10."
                },
                "presentation": {
                    "type": "string",
                    "enum": ["rich", "index", "ids_only"],
                    "description": "Text summary format for the agent. rich (default): timestamp, path, id, one-line content preview. index: timestamp, path, id only. ids_only: markdown table of snapshot ID prefixes only."
                },
                "relative_path": {
                    "type": "string",
                    "description": "Optional file path relative to the project root."
                },
                "from_timestamp": {
                    "type": "string",
                    "description": "Optional RFC3339 lower timestamp bound."
                },
                "to_timestamp": {
                    "type": "string",
                    "description": "Optional RFC3339 upper timestamp bound."
                },
                "hour": {
                    "type": "string",
                    "description": "Optional ISO hour in the form YYYY-MM-DDTHH. Cannot be combined with from_timestamp or to_timestamp."
                },
                "include_safety": {
                    "type": "boolean",
                    "description": "Include safety snapshots. Default: false."
                }
            },
            "required": ["project_root"]
        },
        "annotations": {
            "title": "Recent Local Snapshots",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

fn tool_local_history_view_snapshot() -> Value {
    json!({
        "name": "local_history_view_snapshot",
        "title": "View Local Snapshot",
        "description": "Return metadata and a preview for one snapshot by full snapshot ID or unique snapshot ID prefix.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "snapshot_id": {
                    "type": "string",
                    "description": "Full local-history snapshot ID or unique snapshot ID prefix."
                },
                "data_dir": {
                    "type": "string",
                    "description": "Optional local-history base data directory."
                }
            },
            "required": ["snapshot_id"]
        },
        "annotations": {
            "title": "View Local Snapshot",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

fn tool_local_history_diff_snapshot() -> Value {
    json!({
        "name": "local_history_diff_snapshot",
        "title": "Diff Local Snapshot",
        "description": "Return a unified text diff from one snapshot to the current live file by full snapshot ID or unique snapshot ID prefix.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "snapshot_id": {
                    "type": "string",
                    "description": "Full local-history snapshot ID or unique snapshot ID prefix."
                },
                "data_dir": {
                    "type": "string",
                    "description": "Optional local-history base data directory."
                }
            },
            "required": ["snapshot_id"]
        },
        "annotations": {
            "title": "Diff Local Snapshot",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

fn tool_local_history_restore_snapshot() -> Value {
    json!({
        "name": "local_history_restore_snapshot",
        "title": "Restore Local Snapshot",
        "description": "Restore one snapshot by full snapshot ID or unique snapshot ID prefix. This always creates a safety snapshot first.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "snapshot_id": {
                    "type": "string",
                    "description": "Full local-history snapshot ID or unique snapshot ID prefix."
                },
                "data_dir": {
                    "type": "string",
                    "description": "Optional local-history base data directory."
                }
            },
            "required": ["snapshot_id"]
        },
        "annotations": {
            "title": "Restore Local Snapshot",
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": false
        }
    })
}

fn tool_local_history_prune() -> Value {
    json!({
        "name": "local_history_prune",
        "title": "Prune Local History",
        "description": "Apply the default retention policy to one project root and rebuild the Markdown view.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project_root": {
                    "type": "string",
                    "description": "Existing project root directory."
                },
                "data_dir": {
                    "type": "string",
                    "description": "Optional local-history base data directory."
                }
            },
            "required": ["project_root"]
        },
        "annotations": {
            "title": "Prune Local History",
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": false
        }
    })
}

#[derive(Debug, Deserialize)]
struct WatcherStatusRecord {
    pid: u32,
    started_at: String,
    heartbeat_at: String,
    heartbeat_unix_seconds: u64,
    watched_files: usize,
    log_path: PathBuf,
    last_error: Option<String>,
}

fn print_help() {
    println!(
        "\
local-history-mcp

Runs a newline-delimited JSON-RPC MCP stdio server.

Usage:
  local-history-mcp
  local-history-mcp --help
  local-history-mcp --version
"
    );
}

#[cfg(test)]
mod tests {
    use super::{handle_message, AGENT_GUIDE_URI, MCP_PROTOCOL_VERSION};
    use local_history_core::{LocalHistoryStore, SnapshotKind, SnapshotWriteRequest};
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn initialize_advertises_tools_capability() {
        let response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "0.1.0"
                }
            }
        }))
        .expect("initialize must return a response");

        assert_eq!(response["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(
            response["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
        assert_eq!(
            response["result"]["capabilities"]["resources"]["listChanged"],
            false
        );
        assert!(response["result"]["instructions"]
            .as_str()
            .expect("instructions must be a string")
            .contains("safety snapshot"));
        assert_eq!(
            response["result"]["serverInfo"]["name"],
            "local-history-mcp"
        );
    }

    #[test]
    fn resources_expose_agent_guide() {
        let list_response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "resources/list",
            "params": {}
        }))
        .expect("resources/list must return a response");
        let resources = list_response["result"]["resources"]
            .as_array()
            .expect("resources must be an array");

        assert_eq!(resources[0]["uri"], AGENT_GUIDE_URI);

        let read_response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "resources/read",
            "params": {
                "uri": AGENT_GUIDE_URI
            }
        }))
        .expect("resources/read must return a response");
        let text = read_response["result"]["contents"][0]["text"]
            .as_str()
            .expect("resource text must be a string");

        assert!(text.contains("zed-local-history LLM Guide"));
        assert!(text.contains("Natural language intent mapping"));
        assert!(text.contains("safety snapshot"));
    }

    #[test]
    fn tools_list_exposes_expected_tool_names() {
        let response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .expect("tools/list must return a response");
        let names = response["result"]["tools"]
            .as_array()
            .expect("tools list must be an array")
            .iter()
            .map(|tool| {
                tool["name"]
                    .as_str()
                    .expect("tool name must be a string")
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "local_history_guide",
                "local_history_status",
                "local_history_create_snapshot",
                "local_history_recent_snapshots",
                "local_history_view_snapshot",
                "local_history_diff_snapshot",
                "local_history_restore_snapshot",
                "local_history_prune",
            ]
        );
    }

    #[test]
    fn guide_tool_returns_agent_guide() {
        let response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {
                "name": "local_history_guide",
                "arguments": {}
            }
        }))
        .expect("guide tool must respond");
        let result = &response["result"]["structuredContent"];

        assert_eq!(result["uri"], AGENT_GUIDE_URI);
        assert_eq!(result["mime_type"], "text/markdown");
        assert!(result["text"]
            .as_str()
            .expect("guide text must be a string")
            .contains("Natural language intent mapping"));
    }

    #[test]
    fn create_snapshot_and_recent_tools_work() {
        let root = create_test_root("mcp-create");
        let base_dir = root.join("data");
        let project_root = root.join("project");
        let source_dir = project_root.join("src");
        let live_path = source_dir.join("main.rs");

        fs::create_dir_all(&base_dir).expect("base dir must exist");
        fs::create_dir_all(&source_dir).expect("source dir must exist");
        fs::write(live_path, b"fn main() {}\n").expect("live file must exist");

        let create_response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "local_history_create_snapshot",
                "arguments": {
                    "data_dir": base_dir.display().to_string(),
                    "project_root": project_root.display().to_string(),
                    "relative_path": "src/main.rs"
                }
            }
        }))
        .expect("create snapshot tool must respond");
        let create_result = &create_response["result"]["structuredContent"];

        assert_eq!(create_result["snapshot"]["relative_path"], "src/main.rs");
        assert_eq!(create_result["snapshot"]["kind"], "raw");

        let recent_response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "local_history_recent_snapshots",
                "arguments": {
                    "data_dir": base_dir.display().to_string(),
                    "project_root": project_root.display().to_string(),
                    "limit": 10
                }
            }
        }))
        .expect("recent snapshots tool must respond");
        let recent_result = &recent_response["result"]["structuredContent"];

        assert_eq!(
            recent_result["snapshots"][0]["relative_path"],
            "src/main.rs"
        );
        assert_eq!(recent_result["snapshots"][0]["kind"], "raw");
        assert_eq!(recent_result["presentation"], "rich");
        assert!(recent_result["snapshots"][0]["preview_line"]
            .as_str()
            .expect("rich presentation must include preview_line")
            .contains("fn main()"));
        assert!(recent_response["result"]["content"][0]["text"]
            .as_str()
            .expect("summary text must be present")
            .contains("content:"));

        let ids_only_response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "local_history_recent_snapshots",
                "arguments": {
                    "data_dir": base_dir.display().to_string(),
                    "project_root": project_root.display().to_string(),
                    "limit": 10,
                    "presentation": "ids_only"
                }
            }
        }))
        .expect("ids_only recent snapshots tool must respond");
        let ids_only_text = ids_only_response["result"]["content"][0]["text"]
            .as_str()
            .expect("ids_only summary text must be present");

        assert!(ids_only_text.contains("| Snapshot ID |"));
        assert!(!ids_only_text.contains("content:"));
        assert!(!ids_only_text.contains(" — "));

        cleanup_test_root(&root);
    }

    #[test]
    fn view_and_restore_tools_work() {
        let root = create_test_root("mcp-restore");
        let base_dir = root.join("data");
        let project_root = root.join("project");
        let source_dir = project_root.join("src");
        let live_path = source_dir.join("restore.txt");

        fs::create_dir_all(&base_dir).expect("base dir must exist");
        fs::create_dir_all(&source_dir).expect("source dir must exist");

        let store = LocalHistoryStore::open(&base_dir, &project_root).expect("store must open");
        let snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/restore.txt"),
                contents: b"target state\n".to_vec(),
                timestamp: "2026-05-03T10:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("snapshot must store");
        fs::write(&live_path, b"current state\n").expect("live file must exist");
        let snapshot_prefix = &snapshot.id.as_str()[..12];

        let view_response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "local_history_view_snapshot",
                "arguments": {
                    "snapshot_id": snapshot_prefix,
                    "data_dir": base_dir.display().to_string()
                }
            }
        }))
        .expect("view snapshot tool must respond");
        let view_result = &view_response["result"]["structuredContent"];

        assert_eq!(view_result["snapshot"]["id"], snapshot.id.as_str());
        assert_eq!(view_result["preview"]["kind"], "text");

        let restore_response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "local_history_restore_snapshot",
                "arguments": {
                    "snapshot_id": snapshot_prefix,
                    "data_dir": base_dir.display().to_string()
                }
            }
        }))
        .expect("restore snapshot tool must respond");
        let restore_result = &restore_response["result"]["structuredContent"];

        assert_eq!(
            restore_result["restored_snapshot"]["id"],
            snapshot.id.as_str()
        );
        assert_eq!(
            fs::read_to_string(&live_path).expect("live file must reflect restored snapshot"),
            "target state\n"
        );

        cleanup_test_root(&root);
    }

    #[test]
    fn diff_snapshot_tool_works() {
        let root = create_test_root("mcp-diff");
        let base_dir = root.join("data");
        let project_root = root.join("project");
        let live_path = project_root.join("note.txt");

        fs::create_dir_all(&base_dir).expect("base dir must exist");
        fs::create_dir_all(&project_root).expect("project root must exist");

        let store = LocalHistoryStore::open(&base_dir, &project_root).expect("store must open");
        let snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("note.txt"),
                contents: b"v1\n".to_vec(),
                timestamp: "2026-05-03T10:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("snapshot must store");
        fs::write(live_path, b"v2\n").expect("live file must exist");
        let snapshot_prefix = &snapshot.id.as_str()[..12];

        let diff_response = handle_message(json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "local_history_diff_snapshot",
                "arguments": {
                    "snapshot_id": snapshot_prefix,
                    "data_dir": base_dir.display().to_string()
                }
            }
        }))
        .expect("diff snapshot tool must respond");
        let diff_result = &diff_response["result"]["structuredContent"];
        let diff_text = diff_result["diff"]
            .as_str()
            .expect("diff text must be a string");

        assert_eq!(diff_result["snapshot"]["id"], snapshot.id.as_str());
        assert_eq!(diff_result["unchanged"], false);
        assert!(diff_text.contains("-v1"));
        assert!(diff_text.contains("+v2"));

        cleanup_test_root(&root);
    }

    fn create_test_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be after UNIX_EPOCH")
            .as_nanos();

        std::env::temp_dir().join(format!("zed-local-history-mcp-{label}-{unique}"))
    }

    fn cleanup_test_root(path: &Path) {
        if path.exists() {
            fs::remove_dir_all(path).expect("test directory must be removed");
        }
    }
}
