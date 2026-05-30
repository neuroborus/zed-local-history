use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use local_history_core::{
    default_data_dir, project_id_for_root, snapshot_to_current_unified_diff, HourHistory,
    LocalHistoryStore, PruneReport, RestoreOutcome, RetentionPolicy, SegmentHistory, SnapshotId,
    SnapshotKind, SnapshotPage, SnapshotQuery, SnapshotRecord, SnapshotWriteRequest, StorageLayout,
    WindowedFileHistory,
};
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, PrimitiveDateTime};

const DISPLAY_SNAPSHOT_ID_PREFIX_LEN: usize = 12;
const AMBIGUOUS_SNAPSHOT_ID_SUGGESTION_LIMIT: usize = 10;

#[derive(Debug, Parser)]
#[command(
    name = "local-history",
    about = "CLI recovery interface for zed-local-history"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum HistoryCommands {
    Hour {
        project_root: PathBuf,
        #[arg(long)]
        hour: String,
        #[arg(long)]
        json: bool,
    },
    Segment {
        project_root: PathBuf,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum RenderMarkdownCommands {
    Hour {
        project_root: PathBuf,
        #[arg(long)]
        hour: String,
    },
    Segment {
        project_root: PathBuf,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },
}

#[derive(Debug, Clone, Args, Default)]
struct SnapshotFilterArgs {
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    to: Option<String>,
    #[arg(long)]
    hour: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Status {
        project_root: PathBuf,
        #[arg(long)]
        json: bool,
    },
    ViewRoot {
        project_root: PathBuf,
    },
    Snapshot {
        project_root: PathBuf,
        #[arg(long)]
        file: PathBuf,
    },
    Show {
        snapshot_id: String,
        #[arg(long)]
        json: bool,
    },
    Diff {
        snapshot_id: String,
    },
    History {
        #[command(subcommand)]
        command: HistoryCommands,
    },
    Recent {
        project_root: PathBuf,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[command(flatten)]
        filters: SnapshotFilterArgs,
        #[arg(long)]
        json: bool,
    },
    List {
        project_root: PathBuf,
        #[arg(long, default_value_t = 1)]
        page: usize,
        #[arg(long, default_value_t = 20)]
        page_size: usize,
        #[command(flatten)]
        filters: SnapshotFilterArgs,
        #[arg(long)]
        json: bool,
    },
    Restore {
        snapshot_id: Option<String>,
        #[arg(long)]
        project_root: Option<PathBuf>,
        #[arg(long)]
        recent: Option<usize>,
    },
    UndoRestore {
        project_root: PathBuf,
    },
    RestoreLastSafety {
        project_root: PathBuf,
    },
    SafetyList {
        project_root: PathBuf,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[command(flatten)]
        filters: SnapshotFilterArgs,
        #[arg(long)]
        json: bool,
    },
    Browse {
        project_root: PathBuf,
        #[arg(long, default_value_t = 10)]
        page_size: usize,
        #[command(flatten)]
        filters: SnapshotFilterArgs,
    },
    RebuildMarkdownView {
        project_root: PathBuf,
    },
    Prune {
        project_root: PathBuf,
        #[arg(long)]
        json: bool,
    },
    RenderMarkdown {
        #[command(subcommand)]
        command: RenderMarkdownCommands,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::Status { project_root, json } => print_status(&project_root, json),
        Commands::ViewRoot { project_root } => {
            let layout = layout_for(&project_root);
            println!("{}", layout.view_dir.display());
            Ok(())
        }
        Commands::Snapshot { project_root, file } => snapshot_file(&project_root, &file),
        Commands::Show { snapshot_id, json } => show_snapshot(&snapshot_id, json),
        Commands::Diff { snapshot_id } => diff_snapshot(&snapshot_id),
        Commands::History { command } => run_history_command(command),
        Commands::Recent {
            project_root,
            limit,
            filters,
            json,
        } => print_recent(&project_root, limit, &filters, json),
        Commands::List {
            project_root,
            page,
            page_size,
            filters,
            json,
        } => print_list(&project_root, page, page_size, &filters, json),
        Commands::Restore {
            snapshot_id,
            project_root,
            recent,
        } => restore_command(snapshot_id, project_root.as_deref(), recent),
        Commands::UndoRestore { project_root } => undo_restore(&project_root),
        Commands::RestoreLastSafety { project_root } => restore_last_safety(&project_root),
        Commands::SafetyList {
            project_root,
            limit,
            filters,
            json,
        } => print_safety_list(&project_root, limit, &filters, json),
        Commands::Browse {
            project_root,
            page_size,
            filters,
        } => browse_snapshots(&project_root, page_size, &filters),
        Commands::RebuildMarkdownView { project_root } => rebuild_markdown_view(&project_root),
        Commands::Prune { project_root, json } => prune_history(&project_root, json),
        Commands::RenderMarkdown { command } => run_render_markdown_command(command),
    }
}

fn snapshot_file(project_root: &Path, relative_path: &Path) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let relative_path = normalize_cli_relative_path(relative_path)?;
    let absolute_path = project_root.join(&relative_path);
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

    println!("stored snapshot");
    println!("id={}", snapshot.id);
    println!("path={}", snapshot.relative_path.display());
    println!("kind={}", snapshot.kind.as_str());
    println!("timestamp={}", human_timestamp(&snapshot.timestamp));
    println!("blob_hash={}", snapshot.blob_hash);
    println!("size_bytes={}", snapshot.size_bytes);

    Ok(())
}

fn run_history_command(command: HistoryCommands) -> Result<(), String> {
    match command {
        HistoryCommands::Hour {
            project_root,
            hour,
            json,
        } => print_hour_history(&project_root, &hour, json),
        HistoryCommands::Segment {
            project_root,
            from,
            to,
            json,
        } => print_segment_history(&project_root, &from, &to, json),
    }
}

fn run_render_markdown_command(command: RenderMarkdownCommands) -> Result<(), String> {
    match command {
        RenderMarkdownCommands::Hour { project_root, hour } => {
            render_hour_markdown(&project_root, &hour)
        }
        RenderMarkdownCommands::Segment {
            project_root,
            from,
            to,
        } => render_segment_markdown(&project_root, &from, &to),
    }
}

fn render_hour_markdown(project_root: &Path, hour: &str) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let entry = store
        .render_hour_markdown(hour, &current_timestamp()?)
        .map_err(|error| error.to_string())?;
    let layout = layout_for(project_root);

    println!(
        "{}",
        layout.view_dir.join(entry.relative_markdown_path).display()
    );

    Ok(())
}

fn render_segment_markdown(project_root: &Path, from: &str, to: &str) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let entry = store
        .render_segment_markdown(from, to, &current_timestamp()?)
        .map_err(|error| error.to_string())?;
    let layout = layout_for(project_root);

    println!(
        "{}",
        layout.view_dir.join(entry.relative_markdown_path).display()
    );

    Ok(())
}

fn rebuild_markdown_view(project_root: &Path) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let entries = store
        .rebuild_markdown_view(&current_timestamp()?)
        .map_err(|error| error.to_string())?;
    let layout = layout_for(project_root);

    println!("rebuild complete");
    println!("view_root={}", layout.view_dir.display());
    println!("generated_entries={}", entries.len());
    println!("index={}", layout.view_dir.join("README.md").display());

    Ok(())
}

fn prune_history(project_root: &Path, json_output: bool) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let policy = store.retention_policy();
    let report = store
        .prune(&policy, &current_timestamp()?)
        .map_err(|error| error.to_string())?;

    if json_output {
        return print_json_value(&prune_report_json(&store, &policy, &report));
    }

    println!("prune complete");
    println!("project_root={}", store.project().root.display());
    println!("project_id={}", store.project().id.as_str());
    println!("pruned_at={}", human_timestamp(&report.pruned_at));
    println!(
        "deleted_restore_operations={} deleted_snapshots={} deleted_blobs={} deleted_blob_bytes={}",
        report.deleted_restore_operation_count,
        report.deleted_snapshot_count,
        report.deleted_blob_count,
        report.deleted_blob_bytes
    );
    println!(
        "remaining_snapshots={} remaining_referenced_blob_bytes={}",
        report.remaining_snapshot_count, report.remaining_referenced_blob_bytes
    );
    println!(
        "pruned_for_age={} pruned_for_file_count={} pruned_for_storage={}",
        report.pruned_for_age_count, report.pruned_for_file_count, report.pruned_for_storage_count
    );
    println!("protected_snapshots={}", report.protected_snapshot_count);
    println!("rebuilt_markdown_view={}", report.rebuilt_markdown_view);
    println!(
        "policy=max_snapshots_per_file:{} max_project_storage_bytes:{} max_file_size_bytes:{} max_snapshot_age_days:{}",
        policy.max_snapshots_per_file,
        policy.max_project_storage_bytes,
        policy.max_file_size_bytes,
        policy.max_snapshot_age_days
    );

    Ok(())
}

fn show_snapshot(snapshot_id_input: &str, json_output: bool) -> Result<(), String> {
    let snapshot_id = resolve_snapshot_id(snapshot_id_input)?;
    let store = LocalHistoryStore::open_default_for_snapshot(&snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;
    let snapshot = store
        .snapshot(&snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;
    let contents = store
        .read_snapshot_content(&snapshot_id)
        .map_err(|error| error.to_string())?;
    let preview = render_snapshot_preview(&snapshot, &contents);

    if json_output {
        return print_json_value(&json!({
            "snapshot": snapshot_json(&snapshot),
            "project_id": store.project().id.as_str(),
            "project_root": store.project().root.display().to_string(),
            "content_preview": preview,
        }));
    }

    println!("snapshot_id={}", snapshot.id);
    println!("project_id={}", snapshot.project_id);
    println!("project_root={}", store.project().root.display());
    println!("relative_path={}", snapshot.relative_path.display());
    println!("kind={}", snapshot.kind.as_str());
    println!("captures_missing_file={}", snapshot.captures_missing_file);
    println!("timestamp={}", human_timestamp(&snapshot.timestamp));
    println!("size_bytes={}", snapshot.size_bytes);
    println!("blob_hash={}", snapshot.blob_hash);
    println!("content_preview:");
    println!("{preview}");

    Ok(())
}

fn diff_snapshot(snapshot_id_input: &str) -> Result<(), String> {
    let snapshot_id = resolve_snapshot_id(snapshot_id_input)?;
    let store = LocalHistoryStore::open_default_for_snapshot(&snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;
    let snapshot = store
        .snapshot(&snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;
    let snapshot_contents = store
        .read_snapshot_content(&snapshot_id)
        .map_err(|error| error.to_string())?;
    let live_path = store.project().root.join(&snapshot.relative_path);

    print!(
        "{}",
        snapshot_to_current_unified_diff(&snapshot, &snapshot_contents, &live_path)
            .map_err(|error| error.to_string())?
    );
    Ok(())
}

fn print_hour_history(project_root: &Path, hour: &str, json_output: bool) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let history = store
        .history_for_hour(hour)
        .map_err(|error| error.to_string())?;

    if json_output {
        return print_json_value(&hour_history_json(&store, &history));
    }

    println!(
        "Hour history {} -> {}",
        human_timestamp(&history.hour.from),
        human_timestamp(&history.hour.to)
    );
    println!();

    for segment in &history.segments {
        print_segment_history_text(segment);
    }

    Ok(())
}

fn print_segment_history(
    project_root: &Path,
    from: &str,
    to: &str,
    json_output: bool,
) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let history = store
        .history_for_segment(from, to)
        .map_err(|error| error.to_string())?;

    if json_output {
        return print_json_value(&segment_history_json(&store, &history));
    }

    print_segment_history_text(&history);
    Ok(())
}

fn print_segment_history_text(history: &SegmentHistory) {
    println!(
        "[{}] {} -> {}",
        history.segment.label,
        human_timestamp(&history.segment.from),
        human_timestamp(&history.segment.to)
    );

    if history.affected_files.is_empty() {
        println!("No snapshots in this segment.");
        println!();
        return;
    }

    for file_history in &history.affected_files {
        println!(
            "{} ({} snapshot{})",
            file_history.relative_path.display(),
            file_history.snapshot_count,
            if file_history.snapshot_count == 1 {
                ""
            } else {
                "s"
            }
        );

        for snapshot in &file_history.snapshots {
            println!(
                "  {}  {}",
                human_timestamp(&snapshot.timestamp),
                short_id(snapshot.id.as_str())
            );
        }
    }

    println!();
}

fn print_recent(
    project_root: &Path,
    limit: usize,
    filters: &SnapshotFilterArgs,
    json_output: bool,
) -> Result<(), String> {
    let query = snapshot_query_from_filters(filters, Some(SnapshotKind::Raw), 1, limit)?;
    print_snapshot_query(project_root, &query, json_output, "Latest raw snapshots")
}

fn print_list(
    project_root: &Path,
    page: usize,
    page_size: usize,
    filters: &SnapshotFilterArgs,
    json_output: bool,
) -> Result<(), String> {
    let query = snapshot_query_from_filters(filters, Some(SnapshotKind::Raw), page, page_size)?;
    print_snapshot_query(project_root, &query, json_output, "Raw snapshot history")
}

fn print_safety_list(
    project_root: &Path,
    limit: usize,
    filters: &SnapshotFilterArgs,
    json_output: bool,
) -> Result<(), String> {
    let query = snapshot_query_from_filters(filters, Some(SnapshotKind::Safety), 1, limit)?;
    print_snapshot_query(project_root, &query, json_output, "Latest safety snapshots")
}

fn print_snapshot_query(
    project_root: &Path,
    query: &SnapshotQuery,
    json_output: bool,
    heading: &str,
) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let page = store
        .query_snapshots(query)
        .map_err(|error| error.to_string())?;

    if json_output {
        return print_json_value(&snapshot_page_json(&store, query, &page));
    }

    println!("{heading}");
    println!();

    if page.items.is_empty() {
        println!("No snapshots found.");
        return Ok(());
    }

    println!(
        "page={}/{} page_size={} total_items={}",
        page.page,
        std::cmp::max(page.total_pages, 1),
        page.page_size,
        page.total_items
    );
    println!();

    for (index, snapshot) in page.items.iter().enumerate() {
        println!("{}", format_recent_line(index + 1, snapshot));
    }
    println!();
    println!(
        "Use `local-history restore --project-root {} --recent <index>` to restore by list number.",
        project_root.display()
    );
    println!("The table shows short snapshot ID prefixes; use `--json` for full snapshot IDs.");

    Ok(())
}

fn browse_snapshots(
    project_root: &Path,
    page_size: usize,
    filters: &SnapshotFilterArgs,
) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let mut current_page = 1usize;

    loop {
        let query =
            snapshot_query_from_filters(filters, Some(SnapshotKind::Raw), current_page, page_size)?;
        let page = store
            .query_snapshots(&query)
            .map_err(|error| error.to_string())?;

        println!("Raw snapshot browse");
        println!(
            "page={}/{} page_size={} total_items={}",
            page.page,
            std::cmp::max(page.total_pages, 1),
            page.page_size,
            page.total_items
        );
        println!();

        if page.items.is_empty() {
            println!("No snapshots found for the current filters.");
        } else {
            for (index, snapshot) in page.items.iter().enumerate() {
                println!("{}", format_recent_line(index + 1, snapshot));
            }
        }

        println!();
        println!("Commands: n=next, p=previous, <number>=preview, q=quit");

        let input = prompt("browse> ")?;

        match input.as_str() {
            "q" | "quit" => return Ok(()),
            "n" | "next" => {
                if current_page < std::cmp::max(page.total_pages, 1) {
                    current_page += 1;
                } else {
                    println!("Already on the last page.");
                }
            }
            "p" | "prev" | "previous" => {
                if current_page > 1 {
                    current_page -= 1;
                } else {
                    println!("Already on the first page.");
                }
            }
            _ => match input.parse::<usize>() {
                Ok(selection) => preview_and_maybe_restore(&store, &page, selection)?,
                Err(_) => println!("Expected `n`, `p`, `q`, or a snapshot number."),
            },
        }

        println!();
    }
}

fn preview_and_maybe_restore(
    store: &LocalHistoryStore,
    page: &SnapshotPage,
    selection: usize,
) -> Result<(), String> {
    if selection == 0 || selection > page.items.len() {
        return Err(format!(
            "snapshot [{selection}] is not available on this page"
        ));
    }

    let snapshot = &page.items[selection - 1];
    let contents = store
        .read_snapshot_content(&snapshot.id)
        .map_err(|error| error.to_string())?;

    println!("selected snapshot");
    println!("id={}", snapshot.id);
    println!("path={}", snapshot.relative_path.display());
    println!("timestamp={}", human_timestamp(&snapshot.timestamp));
    println!("size_bytes={}", snapshot.size_bytes);
    println!("content_preview:");
    println!("{}", render_snapshot_preview(snapshot, &contents));

    if confirm("Restore this snapshot? [y/N]: ")? {
        let outcome = store
            .restore_snapshot(&snapshot.id, &current_timestamp()?)
            .map_err(|error| error.to_string())?;
        print_restore_outcome("restored snapshot", &outcome);
    }

    Ok(())
}

fn restore_command(
    snapshot_id: Option<String>,
    project_root: Option<&Path>,
    recent: Option<usize>,
) -> Result<(), String> {
    match (snapshot_id, project_root, recent) {
        (Some(snapshot_id), None, None) => restore_snapshot_by_id(&snapshot_id),
        (None, Some(project_root), Some(index)) => restore_snapshot_by_recent_index(project_root, index),
        _ => Err(
            "use either `local-history restore <snapshot-id>` or `local-history restore --project-root <path> --recent <index>`"
                .to_string(),
        ),
    }
}

fn restore_snapshot_by_id(snapshot_id_input: &str) -> Result<(), String> {
    let snapshot_id = resolve_snapshot_id(snapshot_id_input)?;
    let store = LocalHistoryStore::open_default_for_snapshot(&snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;
    let outcome = store
        .restore_snapshot(&snapshot_id, &current_timestamp()?)
        .map_err(|error| error.to_string())?;

    print_restore_outcome("restored snapshot", &outcome);
    Ok(())
}

fn restore_snapshot_by_recent_index(project_root: &Path, index: usize) -> Result<(), String> {
    if index == 0 {
        return Err("recent index must be 1 or greater".to_string());
    }

    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let snapshots = store
        .recent_raw_snapshots(index)
        .map_err(|error| error.to_string())?;
    let snapshot = snapshots
        .get(index - 1)
        .ok_or_else(|| format!("recent snapshot [{index}] not found"))?;
    let outcome = store
        .restore_snapshot(&snapshot.id, &current_timestamp()?)
        .map_err(|error| error.to_string())?;

    print_restore_outcome("restored snapshot", &outcome);
    Ok(())
}

fn undo_restore(project_root: &Path) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let outcome = store
        .undo_last_restore(&current_timestamp()?)
        .map_err(|error| error.to_string())?;

    print_restore_outcome("undid last restore", &outcome);
    Ok(())
}

fn restore_last_safety(project_root: &Path) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let outcome = store
        .restore_last_safety_snapshot(&current_timestamp()?)
        .map_err(|error| error.to_string())?;

    print_restore_outcome("restored last safety snapshot", &outcome);
    Ok(())
}

fn print_restore_outcome(label: &str, outcome: &RestoreOutcome) {
    println!("{label}");
    println!("restored_snapshot_id={}", outcome.restored_snapshot.id);
    println!("path={}", outcome.restored_path.display());
    println!(
        "restored_snapshot_timestamp={}",
        human_timestamp(&outcome.restored_snapshot.timestamp)
    );
    println!("safety_snapshot_id={}", outcome.safety_snapshot.id);
    println!(
        "safety_snapshot_timestamp={}",
        human_timestamp(&outcome.safety_snapshot.timestamp)
    );
    println!(
        "previous_file_existed={}",
        outcome.operation.previous_file_existed
    );
    println!("restore_operation_id={}", outcome.operation.id);
}

fn print_status(project_root: &Path, json_output: bool) -> Result<(), String> {
    let project_id = project_id_for_root(project_root);
    let layout = StorageLayout::for_project(default_data_dir(), project_id.clone());
    let retention_policy = RetentionPolicy::default();

    if json_output {
        return print_json_value(&json!({
            "project_root": project_root.display().to_string(),
            "project_id": project_id.as_str(),
            "data_dir": layout.project_dir.display().to_string(),
            "database": layout.database_path.display().to_string(),
            "view_root": layout.view_dir.display().to_string(),
            "retention_policy": retention_policy_json(&retention_policy),
        }));
    }

    println!("project_root={}", project_root.display());
    println!("project_id={project_id}");
    println!("data_dir={}", layout.project_dir.display());
    println!("database={}", layout.database_path.display());
    println!("view_root={}", layout.view_dir.display());
    println!(
        "retention_policy=max_snapshots_per_file:{} max_project_storage_bytes:{} max_file_size_bytes:{} max_snapshot_age_days:{}",
        retention_policy.max_snapshots_per_file,
        retention_policy.max_project_storage_bytes,
        retention_policy.max_file_size_bytes,
        retention_policy.max_snapshot_age_days
    );

    Ok(())
}

fn layout_for(project_root: &Path) -> StorageLayout {
    let project_id = project_id_for_root(project_root);
    StorageLayout::for_project(default_data_dir(), project_id)
}

fn snapshot_query_from_filters(
    filters: &SnapshotFilterArgs,
    kind: Option<SnapshotKind>,
    page: usize,
    page_size: usize,
) -> Result<SnapshotQuery, String> {
    let (from_timestamp, to_timestamp) =
        resolve_time_filters(&filters.from, &filters.to, &filters.hour)?;

    Ok(SnapshotQuery {
        relative_path: filters
            .file
            .as_ref()
            .map(|path| normalize_cli_relative_path(path))
            .transpose()?,
        from_timestamp,
        to_timestamp,
        kind,
        page,
        page_size,
    })
}

fn resolve_time_filters(
    from: &Option<String>,
    to: &Option<String>,
    hour: &Option<String>,
) -> Result<(Option<String>, Option<String>), String> {
    if let Some(hour) = hour {
        if from.is_some() || to.is_some() {
            return Err("`--hour` cannot be combined with `--from` or `--to`".to_string());
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

fn normalize_cli_relative_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Err(format!(
            "expected a relative path under the project root, got {}",
            path.display()
        ));
    }

    Ok(path.to_path_buf())
}

fn current_timestamp() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("failed to format timestamp: {error}"))
}

fn resolve_snapshot_id(input: &str) -> Result<SnapshotId, String> {
    let snapshot_id = SnapshotId::new(input);

    if LocalHistoryStore::open_default_for_snapshot(&snapshot_id)
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Ok(snapshot_id);
    }

    let matches = LocalHistoryStore::find_default_snapshot_ids_by_prefix(input)
        .map_err(|error| error.to_string())?;

    match matches.as_slice() {
        [] => Err(format!("snapshot not found: {input}")),
        [snapshot_id] => Ok(snapshot_id.clone()),
        _ => Err(ambiguous_snapshot_prefix_error(input, &matches)),
    }
}

fn ambiguous_snapshot_prefix_error(prefix: &str, matches: &[SnapshotId]) -> String {
    let mut message = format!(
        "snapshot prefix `{prefix}` is ambiguous; use a longer prefix or full snapshot ID:"
    );

    for snapshot_id in matches.iter().take(AMBIGUOUS_SNAPSHOT_ID_SUGGESTION_LIMIT) {
        message.push_str("\n  ");
        message.push_str(snapshot_id.as_str());
    }

    if matches.len() > AMBIGUOUS_SNAPSHOT_ID_SUGGESTION_LIMIT {
        message.push_str(&format!(
            "\n  ... {} more matches",
            matches.len() - AMBIGUOUS_SNAPSHOT_ID_SUGGESTION_LIMIT
        ));
    }

    message
}

fn human_timestamp(raw: &str) -> String {
    OffsetDateTime::parse(raw, &Rfc3339)
        .ok()
        .and_then(|timestamp| {
            timestamp
                .format(&time::macros::format_description!(
                    "[year]-[month]-[day] [hour]:[minute]:[second]"
                ))
                .ok()
        })
        .unwrap_or_else(|| raw.to_string())
}

fn format_recent_line(index: usize, snapshot: &SnapshotRecord) -> String {
    let kind_suffix = match snapshot.kind {
        SnapshotKind::Raw => "",
        SnapshotKind::Safety => "  [safety]",
    };
    let missing_suffix = if snapshot.captures_missing_file {
        "  [missing]"
    } else {
        ""
    };

    format!(
        "[{index}] {}  {:<40}  {}{}{}",
        human_timestamp(&snapshot.timestamp),
        snapshot.relative_path.display(),
        short_id(snapshot.id.as_str()),
        kind_suffix,
        missing_suffix
    )
}

fn short_id(value: &str) -> &str {
    &value[..std::cmp::min(value.len(), DISPLAY_SNAPSHOT_ID_PREFIX_LEN)]
}

fn render_preview(contents: &[u8]) -> String {
    if contents.is_empty() {
        return "<empty file>".to_string();
    }

    match std::str::from_utf8(contents) {
        Ok(text) => {
            let mut lines = text.lines();
            let mut preview = lines.by_ref().take(20).collect::<Vec<_>>().join("\n");

            if lines.next().is_some() {
                preview.push_str("\n... (truncated)");
            }

            if preview.is_empty() {
                "<empty file>".to_string()
            } else {
                preview
            }
        }
        Err(_) => "[binary content omitted]".to_string(),
    }
}

fn render_snapshot_preview(snapshot: &SnapshotRecord, contents: &[u8]) -> String {
    if snapshot.captures_missing_file {
        "[missing file state]".to_string()
    } else {
        render_preview(contents)
    }
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

fn snapshot_page_json(
    store: &LocalHistoryStore,
    query: &SnapshotQuery,
    page: &SnapshotPage,
) -> Value {
    json!({
        "project_id": store.project().id.as_str(),
        "project_root": store.project().root.display().to_string(),
        "query": {
            "relative_path": query.relative_path.as_ref().map(|path| path.display().to_string()),
            "from_timestamp": query.from_timestamp.clone(),
            "to_timestamp": query.to_timestamp.clone(),
            "kind": query.kind.as_ref().map(SnapshotKind::as_str),
            "page": page.page,
            "page_size": page.page_size,
        },
        "total_items": page.total_items,
        "total_pages": page.total_pages,
        "items": page.items.iter().map(snapshot_json).collect::<Vec<_>>(),
    })
}

fn windowed_file_history_json(file_history: &WindowedFileHistory) -> Value {
    json!({
        "relative_path": file_history.relative_path.display().to_string(),
        "snapshot_count": file_history.snapshot_count,
        "snapshots": file_history.snapshots.iter().map(snapshot_json).collect::<Vec<_>>(),
    })
}

fn segment_history_json(store: &LocalHistoryStore, history: &SegmentHistory) -> Value {
    json!({
        "project_id": store.project().id.as_str(),
        "project_root": store.project().root.display().to_string(),
        "segment": {
            "label": &history.segment.label,
            "from": &history.segment.from,
            "to": &history.segment.to,
        },
        "affected_files": history
            .affected_files
            .iter()
            .map(windowed_file_history_json)
            .collect::<Vec<_>>(),
    })
}

fn hour_history_json(store: &LocalHistoryStore, history: &HourHistory) -> Value {
    json!({
        "project_id": store.project().id.as_str(),
        "project_root": store.project().root.display().to_string(),
        "hour": {
            "from": &history.hour.from,
            "to": &history.hour.to,
        },
        "segments": history
            .segments
            .iter()
            .map(|segment| {
                json!({
                    "segment": {
                        "label": &segment.segment.label,
                        "from": &segment.segment.from,
                        "to": &segment.segment.to,
                    },
                    "affected_files": segment
                        .affected_files
                        .iter()
                        .map(windowed_file_history_json)
                        .collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn retention_policy_json(policy: &RetentionPolicy) -> Value {
    json!({
        "max_snapshots_per_file": policy.max_snapshots_per_file,
        "max_project_storage_bytes": policy.max_project_storage_bytes,
        "max_file_size_bytes": policy.max_file_size_bytes,
        "max_snapshot_age_days": policy.max_snapshot_age_days,
    })
}

fn prune_report_json(
    store: &LocalHistoryStore,
    policy: &RetentionPolicy,
    report: &PruneReport,
) -> Value {
    json!({
        "project_id": store.project().id.as_str(),
        "project_root": store.project().root.display().to_string(),
        "policy": retention_policy_json(policy),
        "report": {
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
        }
    })
}

fn print_json_value(value: &Value) -> Result<(), String> {
    let rendered = serde_json::to_string_pretty(value)
        .map_err(|error| format!("failed to render JSON: {error}"))?;
    println!("{rendered}");
    Ok(())
}

fn prompt(label: &str) -> Result<String, String> {
    print!("{label}");
    io::stdout()
        .flush()
        .map_err(|error| format!("failed to flush stdout: {error}"))?;

    let mut buffer = String::new();
    io::stdin()
        .read_line(&mut buffer)
        .map_err(|error| format!("failed to read stdin: {error}"))?;

    Ok(buffer.trim().to_string())
}

fn confirm(label: &str) -> Result<bool, String> {
    let answer = prompt(label)?;

    Ok(matches!(answer.as_str(), "y" | "Y" | "yes" | "YES"))
}

#[cfg(test)]
mod tests {
    use super::{
        ambiguous_snapshot_prefix_error, format_recent_line, human_timestamp, render_preview,
        render_snapshot_preview, resolve_time_filters, short_id, Cli, Commands, HistoryCommands,
        RenderMarkdownCommands, SnapshotFilterArgs,
    };
    use clap::Parser;
    use local_history_core::{ContentHash, ProjectId, SnapshotId, SnapshotKind, SnapshotRecord};
    use std::path::PathBuf;

    #[test]
    fn parses_snapshot_command_with_file_flag() {
        let cli = Cli::try_parse_from(["local-history", "snapshot", ".", "--file", "src/lib.rs"])
            .expect("CLI parse must succeed");

        match cli.command {
            Commands::Snapshot { project_root, file } => {
                assert_eq!(project_root, PathBuf::from("."));
                assert_eq!(file, PathBuf::from("src/lib.rs"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_recent_limit() {
        let cli = Cli::try_parse_from(["local-history", "recent", ".", "--limit", "25"])
            .expect("CLI parse must succeed");

        match cli.command {
            Commands::Recent {
                project_root,
                limit,
                ..
            } => {
                assert_eq!(project_root, PathBuf::from("."));
                assert_eq!(limit, 25);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_restore_recent_mode() {
        let cli = Cli::try_parse_from([
            "local-history",
            "restore",
            "--project-root",
            ".",
            "--recent",
            "2",
        ])
        .expect("CLI parse must succeed");

        match cli.command {
            Commands::Restore {
                snapshot_id,
                project_root,
                recent,
            } => {
                assert_eq!(snapshot_id, None);
                assert_eq!(project_root, Some(PathBuf::from(".")));
                assert_eq!(recent, Some(2));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_undo_restore_command() {
        let cli = Cli::try_parse_from(["local-history", "undo-restore", "."])
            .expect("CLI parse must succeed");

        match cli.command {
            Commands::UndoRestore { project_root } => {
                assert_eq!(project_root, PathBuf::from("."));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_diff_command() {
        let cli = Cli::try_parse_from(["local-history", "diff", "abc123"])
            .expect("CLI parse must succeed");

        match cli.command {
            Commands::Diff { snapshot_id } => {
                assert_eq!(snapshot_id, "abc123");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_list_pagination_and_json_flag() {
        let cli = Cli::try_parse_from([
            "local-history",
            "list",
            ".",
            "--page",
            "2",
            "--page-size",
            "50",
            "--json",
        ])
        .expect("CLI parse must succeed");

        match cli.command {
            Commands::List {
                project_root,
                page,
                page_size,
                json,
                ..
            } => {
                assert_eq!(project_root, PathBuf::from("."));
                assert_eq!(page, 2);
                assert_eq!(page_size, 50);
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_history_hour_command() {
        let cli = Cli::try_parse_from([
            "local-history",
            "history",
            "hour",
            ".",
            "--hour",
            "2026-05-02T14",
            "--json",
        ])
        .expect("CLI parse must succeed");

        match cli.command {
            Commands::History { command } => match command {
                HistoryCommands::Hour {
                    project_root,
                    hour,
                    json,
                } => {
                    assert_eq!(project_root, PathBuf::from("."));
                    assert_eq!(hour, "2026-05-02T14");
                    assert!(json);
                }
                other => panic!("unexpected history command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_history_segment_command() {
        let cli = Cli::try_parse_from([
            "local-history",
            "history",
            "segment",
            ".",
            "--from",
            "2026-05-02T14:10:00Z",
            "--to",
            "2026-05-02T14:20:00Z",
        ])
        .expect("CLI parse must succeed");

        match cli.command {
            Commands::History { command } => match command {
                HistoryCommands::Segment {
                    project_root,
                    from,
                    to,
                    json,
                } => {
                    assert_eq!(project_root, PathBuf::from("."));
                    assert_eq!(from, "2026-05-02T14:10:00Z");
                    assert_eq!(to, "2026-05-02T14:20:00Z");
                    assert!(!json);
                }
                other => panic!("unexpected history command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_render_markdown_hour_command() {
        let cli = Cli::try_parse_from([
            "local-history",
            "render-markdown",
            "hour",
            ".",
            "--hour",
            "2026-05-02T14",
        ])
        .expect("CLI parse must succeed");

        match cli.command {
            Commands::RenderMarkdown { command } => match command {
                RenderMarkdownCommands::Hour { project_root, hour } => {
                    assert_eq!(project_root, PathBuf::from("."));
                    assert_eq!(hour, "2026-05-02T14");
                }
                other => panic!("unexpected render-markdown command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_rebuild_markdown_view_command() {
        let cli = Cli::try_parse_from(["local-history", "rebuild-markdown-view", "."])
            .expect("CLI parse must succeed");

        match cli.command {
            Commands::RebuildMarkdownView { project_root } => {
                assert_eq!(project_root, PathBuf::from("."));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_prune_command() {
        let cli = Cli::try_parse_from(["local-history", "prune", ".", "--json"])
            .expect("CLI parse must succeed");

        match cli.command {
            Commands::Prune { project_root, json } => {
                assert_eq!(project_root, PathBuf::from("."));
                assert!(json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn renders_binary_preview_without_utf8_garbage() {
        assert_eq!(
            render_preview(&[0, 159, 146, 150]),
            "[binary content omitted]"
        );
    }

    #[test]
    fn renders_missing_snapshot_preview_marker() {
        let snapshot = SnapshotRecord {
            id: SnapshotId::new("snapshot-1"),
            project_id: ProjectId::new("project-1"),
            relative_path: PathBuf::from("src/missing.txt"),
            blob_hash: ContentHash::new("hash"),
            size_bytes: 0,
            timestamp: "2026-05-02T14:18:51Z".to_string(),
            kind: SnapshotKind::Safety,
            captures_missing_file: true,
        };

        assert_eq!(
            render_snapshot_preview(&snapshot, &[]),
            "[missing file state]"
        );
    }

    #[test]
    fn recent_lines_show_twelve_character_snapshot_prefixes() {
        let snapshot = SnapshotRecord {
            id: SnapshotId::new("1234567890abcdef12345678"),
            project_id: ProjectId::new("project-1"),
            relative_path: PathBuf::from("src/lib.rs"),
            blob_hash: ContentHash::new("hash"),
            size_bytes: 3,
            timestamp: "2026-05-02T14:18:51Z".to_string(),
            kind: SnapshotKind::Raw,
            captures_missing_file: false,
        };

        let line = format_recent_line(1, &snapshot);

        assert!(line.contains("1234567890ab"));
        assert!(!line.contains("1234567890abc"));
        assert_eq!(short_id(snapshot.id.as_str()), "1234567890ab");
    }

    #[test]
    fn ambiguous_snapshot_prefix_error_lists_full_matching_ids() {
        let matches = vec![
            SnapshotId::new("abcdef1234567890abcdef12"),
            SnapshotId::new("abcdef999999999999999999"),
        ];
        let message = ambiguous_snapshot_prefix_error("abcdef", &matches);

        assert!(message.contains("snapshot prefix `abcdef` is ambiguous"));
        assert!(message.contains("abcdef1234567890abcdef12"));
        assert!(message.contains("abcdef999999999999999999"));
    }

    #[test]
    fn resolves_hour_filter_to_closed_open_range() {
        let filters = SnapshotFilterArgs {
            hour: Some("2026-05-02T14".to_string()),
            ..SnapshotFilterArgs::default()
        };
        let (from, to) =
            resolve_time_filters(&filters.from, &filters.to, &filters.hour).expect("hour parse");

        assert_eq!(from.as_deref(), Some("2026-05-02T14:00:00Z"));
        assert_eq!(to.as_deref(), Some("2026-05-02T15:00:00Z"));
    }

    #[test]
    fn renders_human_timestamp_when_input_is_rfc3339() {
        assert_eq!(
            human_timestamp("2026-05-02T14:18:51Z"),
            "2026-05-02 14:18:51"
        );
    }
}
