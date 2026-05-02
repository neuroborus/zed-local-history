use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use local_history_core::{
    default_data_dir, project_id_for_root, LocalHistoryStore, RestoreOutcome, SnapshotId,
    SnapshotKind, SnapshotRecord, SnapshotWriteRequest, StorageLayout,
};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

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
enum Commands {
    Status {
        project_root: PathBuf,
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
    },
    Recent {
        project_root: PathBuf,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    List {
        project_root: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
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
    },
    RenderMarkdown {
        scope: String,
        project_root: PathBuf,
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
        Commands::Status { project_root } => {
            print_status(&project_root);
            Ok(())
        }
        Commands::ViewRoot { project_root } => {
            let layout = layout_for(&project_root);
            println!("{}", layout.view_dir.display());
            Ok(())
        }
        Commands::Snapshot { project_root, file } => snapshot_file(&project_root, &file),
        Commands::Show { snapshot_id } => show_snapshot(&SnapshotId::new(snapshot_id)),
        Commands::Recent {
            project_root,
            limit,
        } => print_recent(&project_root, limit),
        Commands::List {
            project_root,
            limit,
        } => print_recent(&project_root, limit),
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
        } => print_safety_list(&project_root, limit),
        Commands::RenderMarkdown {
            scope,
            project_root,
        } => {
            let layout = layout_for(&project_root);
            let target = layout.view_dir.join(scope).with_extension("md");
            println!("{}", target.display());
            Ok(())
        }
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

fn show_snapshot(snapshot_id: &SnapshotId) -> Result<(), String> {
    let store = LocalHistoryStore::open_default_for_snapshot(snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;
    let snapshot = store
        .snapshot(snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;
    let contents = store
        .read_snapshot_content(snapshot_id)
        .map_err(|error| error.to_string())?;

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
    println!("{}", render_snapshot_preview(&snapshot, &contents));

    Ok(())
}

fn print_recent(project_root: &Path, limit: usize) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let snapshots = store
        .recent_raw_snapshots(limit)
        .map_err(|error| error.to_string())?;

    println!("Latest raw snapshots");
    println!();

    if snapshots.is_empty() {
        println!("No snapshots found.");
        return Ok(());
    }

    for (index, snapshot) in snapshots.iter().enumerate() {
        println!("{}", format_recent_line(index + 1, snapshot));
    }

    Ok(())
}

fn print_safety_list(project_root: &Path, limit: usize) -> Result<(), String> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let snapshots = store
        .safety_snapshots(limit)
        .map_err(|error| error.to_string())?;

    println!("Latest safety snapshots");
    println!();

    if snapshots.is_empty() {
        println!("No safety snapshots found.");
        return Ok(());
    }

    for (index, snapshot) in snapshots.iter().enumerate() {
        println!("{}", format_recent_line(index + 1, snapshot));
    }

    Ok(())
}

fn restore_command(
    snapshot_id: Option<String>,
    project_root: Option<&Path>,
    recent: Option<usize>,
) -> Result<(), String> {
    match (snapshot_id, project_root, recent) {
        (Some(snapshot_id), None, None) => restore_snapshot_by_id(&SnapshotId::new(snapshot_id)),
        (None, Some(project_root), Some(index)) => restore_snapshot_by_recent_index(project_root, index),
        _ => Err(
            "use either `local-history restore <snapshot-id>` or `local-history restore --project-root <path> --recent <index>`"
                .to_string(),
        ),
    }
}

fn restore_snapshot_by_id(snapshot_id: &SnapshotId) -> Result<(), String> {
    let store = LocalHistoryStore::open_default_for_snapshot(snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;
    let outcome = store
        .restore_snapshot(snapshot_id, &current_timestamp()?)
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

fn layout_for(project_root: &Path) -> StorageLayout {
    let project_id = project_id_for_root(project_root);
    StorageLayout::for_project(default_data_dir(), project_id)
}

fn print_status(project_root: &Path) {
    let project_id = project_id_for_root(project_root);
    let layout = StorageLayout::for_project(default_data_dir(), project_id.clone());

    println!("project_root={}", project_root.display());
    println!("project_id={project_id}");
    println!("data_dir={}", layout.project_dir.display());
    println!("database={}", layout.database_path.display());
    println!("view_root={}", layout.view_dir.display());
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
    &value[..std::cmp::min(value.len(), 8)]
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

#[cfg(test)]
mod tests {
    use super::{human_timestamp, render_preview, Cli, Commands};
    use clap::Parser;
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
    fn renders_binary_preview_without_utf8_garbage() {
        assert_eq!(
            render_preview(&[0, 159, 146, 150]),
            "[binary content omitted]"
        );
    }

    #[test]
    fn renders_human_timestamp_when_input_is_rfc3339() {
        assert_eq!(
            human_timestamp("2026-05-02T14:18:51Z"),
            "2026-05-02 14:18:51"
        );
    }
}
