use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use local_history_core::{
    matches_default_ignored_path, normalize_project_root, LocalHistoryStore, SnapshotId,
    SnapshotKind, SnapshotWriteRequest, StorageLayout,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, Time};

const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(750);
const WATCHER_STALE_AFTER: u64 = 5;
const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_START_POLL_INTERVAL: Duration = Duration::from_millis(100);

type RuntimeResult<T> = Result<T, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileState {
    contents: Vec<u8>,
    is_binary: bool,
}

impl FileState {
    fn from_contents(contents: Vec<u8>) -> Self {
        Self {
            is_binary: std::str::from_utf8(&contents).is_err(),
            contents,
        }
    }
}

type ProjectState = BTreeMap<PathBuf, FileState>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotCandidate {
    relative_path: PathBuf,
    previous_state: FileState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WatcherStatusRecord {
    project_root: PathBuf,
    project_id: String,
    pid: u32,
    started_at: String,
    heartbeat_at: String,
    heartbeat_unix_seconds: u64,
    watched_files: usize,
    data_dir: PathBuf,
    database_path: PathBuf,
    view_root: PathBuf,
    log_path: PathBuf,
    last_error: Option<String>,
}

pub fn health_value() -> Value {
    json!({
        "status": "ok",
        "service": "local-history-sidecar",
        "watch_mode": "polling",
    })
}

pub fn watch(project_root: &Path) -> RuntimeResult<()> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let project_root = store.project().root.clone();
    let mut state = scan_project(&project_root)?;
    let started_at = current_timestamp()?;

    write_status(
        status_path(store.layout()),
        &WatcherStatusRecord {
            project_root: project_root.clone(),
            project_id: store.project().id.as_str().to_string(),
            pid: std::process::id(),
            started_at: started_at.clone(),
            heartbeat_at: started_at.clone(),
            heartbeat_unix_seconds: current_unix_seconds(),
            watched_files: state.len(),
            data_dir: store.layout().project_dir.clone(),
            database_path: store.layout().database_path.clone(),
            view_root: store.layout().view_dir.clone(),
            log_path: log_path(store.layout()),
            last_error: None,
        },
    )?;

    println!(
        "{}",
        serde_json::to_string(&watcher_status_value(
            store.layout(),
            &project_root,
            store.project().id.as_str(),
            read_status(status_path(store.layout()))?,
        ))
        .map_err(|error| format!("failed to serialize watcher startup state: {error}"))?
    );

    loop {
        let current = match scan_project(&project_root) {
            Ok(current) => current,
            Err(error) => {
                update_status_error(store.layout(), &project_root, Some(error.clone()))?;
                thread::sleep(WATCH_POLL_INTERVAL);
                continue;
            }
        };

        if let Err(error) = reconcile_project_state(&store, &state, &current) {
            update_status_error(store.layout(), &project_root, Some(error.clone()))?;
            thread::sleep(WATCH_POLL_INTERVAL);
            continue;
        }

        state = current;
        update_status_error(store.layout(), &project_root, None)?;
        update_status_watched_files(store.layout(), &project_root, state.len())?;
        thread::sleep(WATCH_POLL_INTERVAL);
    }
}

pub fn ensure_daemon(project_root: &Path) -> RuntimeResult<Value> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let project_root = store.project().root.clone();
    let status_file = status_path(store.layout());
    let existing = read_status(&status_file)?;

    if let Some(record) = existing.as_ref() {
        if status_is_fresh(record) {
            return Ok(json!({
                "status": "ok",
                "started": false,
                "watcher": watcher_record_json(record, true),
            }));
        }
    }

    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate sidecar executable: {error}"))?;
    let log_file = log_path(store.layout());
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .map_err(|error| format!("failed to open watcher log {}: {error}", log_file.display()))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("failed to clone watcher log handle: {error}"))?;

    let child = Command::new(executable)
        .arg("watch")
        .arg(&project_root)
        .arg("--daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|error| format!("failed to start watcher: {error}"))?;

    let started = wait_for_status(&status_file, DAEMON_START_TIMEOUT)?;

    Ok(json!({
        "status": "ok",
        "started": true,
        "spawned_pid": child.id(),
        "watcher": watcher_record_json(&started, status_is_fresh(&started)),
    }))
}

pub fn status(project_root: &Path) -> RuntimeResult<Value> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let project_root = store.project().root.clone();

    Ok(watcher_status_value(
        store.layout(),
        &project_root,
        store.project().id.as_str(),
        read_status(status_path(store.layout()))?,
    ))
}

pub fn view_root(project_root: &Path) -> RuntimeResult<Value> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let project_root = store.project().root.clone();

    Ok(json!({
        "status": "ok",
        "project_root": project_root.display().to_string(),
        "project_id": store.project().id.as_str(),
        "view_root": store.layout().view_dir.display().to_string(),
    }))
}

pub fn render_current_hour_markdown(project_root: &Path) -> RuntimeResult<Value> {
    let current_hour = current_hour_string(0)?;
    render_hour_markdown(project_root, &current_hour)
}

pub fn render_previous_hour_markdown(project_root: &Path) -> RuntimeResult<Value> {
    let previous_hour = current_hour_string(-1)?;
    render_hour_markdown(project_root, &previous_hour)
}

pub fn render_current_segment_markdown(project_root: &Path) -> RuntimeResult<Value> {
    let (from, to) = segment_bounds_at(OffsetDateTime::now_utc())?;
    render_segment_markdown(project_root, &from, &to)
}

pub fn render_segment_at_markdown(project_root: &Path, at: &str) -> RuntimeResult<Value> {
    let timestamp = OffsetDateTime::parse(at, &Rfc3339)
        .map_err(|error| format!("failed to parse segment timestamp `{at}`: {error}"))?;
    let (from, to) = segment_bounds_at(timestamp)?;
    render_segment_markdown(project_root, &from, &to)
}

pub fn render_hour_markdown(project_root: &Path, hour: &str) -> RuntimeResult<Value> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let project_root = store.project().root.clone();
    let entry = store
        .render_hour_markdown(hour, &current_timestamp()?)
        .map_err(|error| error.to_string())?;
    let markdown_path = store.layout().view_dir.join(&entry.relative_markdown_path);

    Ok(json!({
        "status": "ok",
        "scope": "hour",
        "hour": hour,
        "project_root": project_root.display().to_string(),
        "project_id": store.project().id.as_str(),
        "view_root": store.layout().view_dir.display().to_string(),
        "markdown_path": markdown_path.display().to_string(),
        "relative_markdown_path": entry.relative_markdown_path.display().to_string(),
    }))
}

pub fn render_segment_markdown(project_root: &Path, from: &str, to: &str) -> RuntimeResult<Value> {
    let store = LocalHistoryStore::open_default(project_root).map_err(|error| error.to_string())?;
    let project_root = store.project().root.clone();
    let entry = store
        .render_segment_markdown(from, to, &current_timestamp()?)
        .map_err(|error| error.to_string())?;
    let markdown_path = store.layout().view_dir.join(&entry.relative_markdown_path);

    Ok(json!({
        "status": "ok",
        "scope": "segment",
        "from": from,
        "to": to,
        "project_root": project_root.display().to_string(),
        "project_id": store.project().id.as_str(),
        "view_root": store.layout().view_dir.display().to_string(),
        "markdown_path": markdown_path.display().to_string(),
        "relative_markdown_path": entry.relative_markdown_path.display().to_string(),
    }))
}

pub fn restore_snapshot(snapshot_id: &str) -> RuntimeResult<Value> {
    let snapshot_id = SnapshotId::new(snapshot_id.to_string());
    let store = LocalHistoryStore::open_default_for_snapshot(&snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("snapshot not found: {}", snapshot_id.as_str()))?;
    let project_root = store.project().root.clone();
    let outcome = store
        .restore_snapshot(&snapshot_id, &current_timestamp()?)
        .map_err(|error| error.to_string())?;

    Ok(json!({
        "status": "ok",
        "project_root": project_root.display().to_string(),
        "project_id": store.project().id.as_str(),
        "restored_snapshot_id": outcome.restored_snapshot.id.as_str(),
        "restored_snapshot_timestamp": outcome.restored_snapshot.timestamp,
        "restored_path": outcome.restored_path.display().to_string(),
        "safety_snapshot_id": outcome.safety_snapshot.id.as_str(),
        "safety_snapshot_timestamp": outcome.safety_snapshot.timestamp,
        "previous_file_existed": outcome.operation.previous_file_existed,
        "restore_operation_id": outcome.operation.id,
    }))
}

fn watcher_status_value(
    layout: &StorageLayout,
    project_root: &Path,
    project_id: &str,
    status: Option<WatcherStatusRecord>,
) -> Value {
    match status {
        Some(record) => json!({
            "status": "ok",
            "project_root": project_root.display().to_string(),
            "project_id": record.project_id.clone(),
            "watcher": watcher_record_json(&record, status_is_fresh(&record)),
            "data_dir": layout.project_dir.display().to_string(),
            "database": layout.database_path.display().to_string(),
            "view_root": layout.view_dir.display().to_string(),
        }),
        None => json!({
            "status": "ok",
            "project_root": project_root.display().to_string(),
            "project_id": project_id,
            "watcher": {
                "active": false,
            },
            "data_dir": layout.project_dir.display().to_string(),
            "database": layout.database_path.display().to_string(),
            "view_root": layout.view_dir.display().to_string(),
        }),
    }
}

fn watcher_record_json(record: &WatcherStatusRecord, active: bool) -> Value {
    json!({
        "active": active,
        "pid": record.pid,
        "started_at": record.started_at.clone(),
        "heartbeat_at": record.heartbeat_at.clone(),
        "heartbeat_unix_seconds": record.heartbeat_unix_seconds,
        "watched_files": record.watched_files,
        "project_root": record.project_root.display().to_string(),
        "project_id": record.project_id.clone(),
        "data_dir": record.data_dir.display().to_string(),
        "database": record.database_path.display().to_string(),
        "view_root": record.view_root.display().to_string(),
        "log_path": record.log_path.display().to_string(),
        "last_error": record.last_error.clone(),
    })
}

fn wait_for_status(status_file: &Path, timeout: Duration) -> RuntimeResult<WatcherStatusRecord> {
    let started_at = SystemTime::now();

    loop {
        if let Some(record) = read_status(status_file)? {
            if status_is_fresh(&record) {
                return Ok(record);
            }
        }

        if started_at.elapsed().unwrap_or_default() >= timeout {
            return Err(format!(
                "watcher did not publish a fresh heartbeat at {} in time",
                status_file.display()
            ));
        }

        thread::sleep(DAEMON_START_POLL_INTERVAL);
    }
}

fn status_is_fresh(record: &WatcherStatusRecord) -> bool {
    current_unix_seconds().saturating_sub(record.heartbeat_unix_seconds) <= WATCHER_STALE_AFTER
}

fn update_status_watched_files(
    layout: &StorageLayout,
    project_root: &Path,
    watched_files: usize,
) -> RuntimeResult<()> {
    let mut record =
        read_status(status_path(layout))?.unwrap_or_else(|| blank_status(layout, project_root));
    record.heartbeat_at = current_timestamp()?;
    record.heartbeat_unix_seconds = current_unix_seconds();
    record.watched_files = watched_files;

    write_status(status_path(layout), &record)
}

fn update_status_error(
    layout: &StorageLayout,
    project_root: &Path,
    last_error: Option<String>,
) -> RuntimeResult<()> {
    let mut record =
        read_status(status_path(layout))?.unwrap_or_else(|| blank_status(layout, project_root));
    record.heartbeat_at = current_timestamp()?;
    record.heartbeat_unix_seconds = current_unix_seconds();
    record.last_error = last_error;

    write_status(status_path(layout), &record)
}

fn blank_status(layout: &StorageLayout, project_root: &Path) -> WatcherStatusRecord {
    WatcherStatusRecord {
        project_root: project_root.to_path_buf(),
        project_id: layout
            .project_dir
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown-project".to_string()),
        pid: std::process::id(),
        started_at: "unknown".to_string(),
        heartbeat_at: "unknown".to_string(),
        heartbeat_unix_seconds: current_unix_seconds(),
        watched_files: 0,
        data_dir: layout.project_dir.clone(),
        database_path: layout.database_path.clone(),
        view_root: layout.view_dir.clone(),
        log_path: log_path(layout),
        last_error: None,
    }
}

fn reconcile_project_state(
    store: &LocalHistoryStore,
    previous: &ProjectState,
    current: &ProjectState,
) -> RuntimeResult<()> {
    for candidate in diff_project_states(previous, current) {
        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: candidate.relative_path,
                contents: candidate.previous_state.contents,
                timestamp: current_timestamp()?,
                kind: SnapshotKind::Raw,
                is_binary: candidate.previous_state.is_binary,
                captures_missing_file: false,
            })
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn diff_project_states(previous: &ProjectState, current: &ProjectState) -> Vec<SnapshotCandidate> {
    let mut candidates = Vec::new();

    for (relative_path, previous_state) in previous {
        match current.get(relative_path) {
            Some(current_state) if current_state.contents != previous_state.contents => {
                candidates.push(SnapshotCandidate {
                    relative_path: relative_path.clone(),
                    previous_state: previous_state.clone(),
                });
            }
            None => {
                candidates.push(SnapshotCandidate {
                    relative_path: relative_path.clone(),
                    previous_state: previous_state.clone(),
                });
            }
            _ => {}
        }
    }

    candidates
}

fn scan_project(project_root: &Path) -> RuntimeResult<ProjectState> {
    let project_root = normalize_project_root(project_root);
    let mut state = BTreeMap::new();

    scan_directory(&project_root, &project_root, &mut state)?;

    Ok(state)
}

fn scan_directory(
    project_root: &Path,
    directory: &Path,
    state: &mut ProjectState,
) -> RuntimeResult<()> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read directory {}: {error}", directory.display()))?;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        let relative_path = match path.strip_prefix(project_root) {
            Ok(relative_path) => relative_path,
            Err(_) => continue,
        };

        if relative_path.as_os_str().is_empty() || matches_default_ignored_path(relative_path) {
            continue;
        }

        if file_type.is_dir() {
            scan_directory(project_root, &path, state)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let contents = match fs::read(&path) {
            Ok(contents) => contents,
            Err(_) => continue,
        };

        state.insert(
            relative_path.to_path_buf(),
            FileState::from_contents(contents),
        );
    }

    Ok(())
}

fn status_path(layout: &StorageLayout) -> PathBuf {
    layout.logs_dir.join("watcher-status.json")
}

fn log_path(layout: &StorageLayout) -> PathBuf {
    layout.logs_dir.join("watcher.log")
}

fn write_status(path: impl AsRef<Path>, record: &WatcherStatusRecord) -> RuntimeResult<()> {
    let path = path.as_ref();
    let bytes = serde_json::to_vec_pretty(record)
        .map_err(|error| format!("failed to serialize watcher status: {error}"))?;
    fs::write(path, bytes)
        .map_err(|error| format!("failed to write watcher status {}: {error}", path.display()))
}

fn read_status(path: impl AsRef<Path>) -> RuntimeResult<Option<WatcherStatusRecord>> {
    let path = path.as_ref();

    if !path.is_file() {
        return Ok(None);
    }

    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read watcher status {}: {error}", path.display()))?;

    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("failed to parse watcher status {}: {error}", path.display()))
}

fn current_timestamp() -> RuntimeResult<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("failed to format timestamp: {error}"))
}

fn current_hour_string(hour_offset: i64) -> RuntimeResult<String> {
    let timestamp = OffsetDateTime::now_utc() + time::Duration::hours(hour_offset);

    timestamp
        .format(&time::macros::format_description!(
            "[year]-[month]-[day]T[hour]"
        ))
        .map_err(|error| format!("failed to format ISO hour: {error}"))
}

fn segment_bounds_at(timestamp: OffsetDateTime) -> RuntimeResult<(String, String)> {
    let segment_minute = (timestamp.minute() / 10) * 10;
    let segment_time = Time::from_hms(timestamp.hour(), segment_minute, 0)
        .map_err(|error| format!("failed to build segment time: {error}"))?;
    let from = timestamp.replace_time(segment_time);
    let to = from + time::Duration::minutes(10);

    Ok((
        from.format(&Rfc3339)
            .map_err(|error| format!("failed to format segment start: {error}"))?,
        to.format(&Rfc3339)
            .map_err(|error| format!("failed to format segment end: {error}"))?,
    ))
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{
        diff_project_states, reconcile_project_state, scan_project, segment_bounds_at,
        status_is_fresh, FileState, ProjectState, WatcherStatusRecord,
    };
    use local_history_core::{LocalHistoryStore, SnapshotKind};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    #[test]
    fn scan_project_skips_ignored_paths() {
        let project_root = create_test_project("scan");

        fs::create_dir_all(project_root.join("src")).expect("src dir must exist");
        fs::create_dir_all(project_root.join("node_modules").join("demo"))
            .expect("ignored dir must exist");
        fs::write(project_root.join("src/lib.rs"), b"fn main() {}\n").expect("file must exist");
        fs::write(project_root.join(".env"), b"SECRET=1\n").expect("env file must exist");
        fs::write(project_root.join("node_modules/demo/index.js"), b"ignored")
            .expect("ignored file must exist");

        let state = scan_project(&project_root).expect("scan must succeed");

        assert_eq!(state.len(), 1);
        assert!(state.contains_key(Path::new("src/lib.rs")));

        cleanup_test_project(&project_root);
    }

    #[test]
    fn diff_project_states_reports_modified_and_deleted_files() {
        let mut previous = ProjectState::new();
        previous.insert(
            PathBuf::from("src/lib.rs"),
            FileState::from_contents(b"before".to_vec()),
        );
        previous.insert(
            PathBuf::from("src/deleted.rs"),
            FileState::from_contents(b"gone".to_vec()),
        );

        let mut current = ProjectState::new();
        current.insert(
            PathBuf::from("src/lib.rs"),
            FileState::from_contents(b"after".to_vec()),
        );
        current.insert(
            PathBuf::from("src/new.rs"),
            FileState::from_contents(b"new".to_vec()),
        );

        let changes = diff_project_states(&previous, &current);

        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].relative_path, PathBuf::from("src/deleted.rs"));
        assert_eq!(changes[1].relative_path, PathBuf::from("src/lib.rs"));
    }

    #[test]
    fn reconcile_project_state_stores_previous_contents_for_modified_file() {
        let root = create_test_root("modified");
        let base_dir = root.join("data");
        let project_root = root.join("project");

        fs::create_dir_all(&base_dir).expect("base dir must exist");
        fs::create_dir_all(&project_root).expect("project dir must exist");

        let store = LocalHistoryStore::open(&base_dir, &project_root).expect("store must open");
        let previous = state_with_file("src/lib.rs", b"before");
        let current = state_with_file("src/lib.rs", b"after");

        reconcile_project_state(&store, &previous, &current).expect("reconcile must succeed");

        let snapshots = store
            .recent_raw_snapshots(10)
            .expect("snapshot query must succeed");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].kind, SnapshotKind::Raw);
        assert_eq!(
            store
                .read_snapshot_content(&snapshots[0].id)
                .expect("snapshot contents must exist"),
            b"before"
        );

        cleanup_test_project(&root);
    }

    #[test]
    fn reconcile_project_state_stores_previous_contents_for_deleted_file() {
        let root = create_test_root("deleted");
        let base_dir = root.join("data");
        let project_root = root.join("project");

        fs::create_dir_all(&base_dir).expect("base dir must exist");
        fs::create_dir_all(&project_root).expect("project dir must exist");

        let store = LocalHistoryStore::open(&base_dir, &project_root).expect("store must open");
        let previous = state_with_file("src/deleted.rs", b"before delete");
        let current = ProjectState::new();

        reconcile_project_state(&store, &previous, &current).expect("reconcile must succeed");

        let snapshots = store
            .recent_raw_snapshots(10)
            .expect("snapshot query must succeed");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            store
                .read_snapshot_content(&snapshots[0].id)
                .expect("snapshot contents must exist"),
            b"before delete"
        );

        cleanup_test_project(&root);
    }

    #[test]
    fn status_freshness_uses_recent_heartbeat() {
        let record = WatcherStatusRecord {
            project_root: PathBuf::from("/tmp/project"),
            project_id: "project-id".to_string(),
            pid: 42,
            started_at: "2026-05-02T14:00:00Z".to_string(),
            heartbeat_at: "2026-05-02T14:00:00Z".to_string(),
            heartbeat_unix_seconds: super::current_unix_seconds(),
            watched_files: 3,
            data_dir: PathBuf::from("/tmp/data"),
            database_path: PathBuf::from("/tmp/data/metadata.sqlite"),
            view_root: PathBuf::from("/tmp/data/view"),
            log_path: PathBuf::from("/tmp/data/watcher.log"),
            last_error: None,
        };

        assert!(status_is_fresh(&record));
    }

    #[test]
    fn segment_bounds_round_down_to_fixed_ten_minute_window() {
        let timestamp =
            OffsetDateTime::parse("2026-05-02T14:14:28Z", &Rfc3339).expect("timestamp must parse");

        let (from, to) = segment_bounds_at(timestamp).expect("segment bounds must resolve");

        assert_eq!(from, "2026-05-02T14:10:00Z");
        assert_eq!(to, "2026-05-02T14:20:00Z");
    }

    fn state_with_file(relative_path: &str, contents: &[u8]) -> ProjectState {
        let mut state = ProjectState::new();
        state.insert(
            PathBuf::from(relative_path),
            FileState::from_contents(contents.to_vec()),
        );
        state
    }

    fn create_test_project(label: &str) -> PathBuf {
        let root = create_test_root(label);
        fs::create_dir_all(&root).expect("project dir must exist");
        root
    }

    fn create_test_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be after UNIX_EPOCH")
            .as_nanos();

        std::env::temp_dir().join(format!("zed-local-history-sidecar-{label}-{unique}"))
    }

    fn cleanup_test_project(path: &Path) {
        if path.exists() {
            fs::remove_dir_all(path).expect("test directory must be removed");
        }
    }
}
