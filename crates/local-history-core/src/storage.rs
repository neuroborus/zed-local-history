use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use rusqlite::{named_params, params, Connection, OptionalExtension};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, PrimitiveDateTime, UtcOffset};

use crate::error::StorageError;
use crate::hashing::sha256_hex;
use crate::identity::{normalize_project_root, project_id_for_root};
use crate::layout::{default_data_dir, StorageLayout};
use crate::model::{
    segment_label, CompressionKind, ContentBlobRecord, ContentHash, GeneratedMarkdownViewEntry,
    HourBucket, HourHistory, ProjectId, ProjectRecord, PruneReport, RestoreOperationRecord,
    RestoreOutcome, RetentionPolicy, SegmentHistory, SnapshotId, SnapshotKind, SnapshotRecord,
    TimeSegment, TrackedFileRecord, WindowedFileHistory,
};

const SCHEMA_SQL: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    root TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS tracked_files (
    project_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    current_content_hash TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    is_binary INTEGER NOT NULL,
    PRIMARY KEY (project_id, relative_path),
    FOREIGN KEY (project_id) REFERENCES projects(id)
);

CREATE TABLE IF NOT EXISTS content_blobs (
    content_hash TEXT PRIMARY KEY,
    size_bytes INTEGER NOT NULL,
    compression TEXT NOT NULL,
    storage_path TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS snapshots (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    blob_hash TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    timestamp TEXT NOT NULL,
    kind TEXT NOT NULL,
    captures_missing_file INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (project_id) REFERENCES projects(id),
    FOREIGN KEY (blob_hash) REFERENCES content_blobs(content_hash)
);

CREATE TABLE IF NOT EXISTS restore_operations (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    restored_snapshot_id TEXT NOT NULL,
    safety_snapshot_id TEXT NOT NULL,
    previous_file_existed INTEGER NOT NULL DEFAULT 1,
    previous_content_hash TEXT NOT NULL,
    restored_content_hash TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(id),
    FOREIGN KEY (restored_snapshot_id) REFERENCES snapshots(id),
    FOREIGN KEY (safety_snapshot_id) REFERENCES snapshots(id)
);

CREATE TABLE IF NOT EXISTS generated_markdown_views (
    project_id TEXT NOT NULL,
    relative_markdown_path TEXT NOT NULL,
    title TEXT NOT NULL,
    generated_at TEXT NOT NULL,
    PRIMARY KEY (project_id, relative_markdown_path),
    FOREIGN KEY (project_id) REFERENCES projects(id)
);

CREATE INDEX IF NOT EXISTS snapshots_project_path_timestamp_idx
ON snapshots(project_id, relative_path, timestamp DESC);

CREATE INDEX IF NOT EXISTS restore_operations_project_timestamp_idx
ON restore_operations(project_id, timestamp DESC);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotWriteRequest {
    pub relative_path: PathBuf,
    pub contents: Vec<u8>,
    pub timestamp: String,
    pub kind: SnapshotKind,
    pub is_binary: bool,
    pub captures_missing_file: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotQuery {
    pub relative_path: Option<PathBuf>,
    pub from_timestamp: Option<String>,
    pub to_timestamp: Option<String>,
    pub kind: Option<SnapshotKind>,
    pub page: usize,
    pub page_size: usize,
}

impl Default for SnapshotQuery {
    fn default() -> Self {
        Self {
            relative_path: None,
            from_timestamp: None,
            to_timestamp: None,
            kind: None,
            page: 1,
            page_size: 20,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotPage {
    pub page: usize,
    pub page_size: usize,
    pub total_items: usize,
    pub total_pages: usize,
    pub items: Vec<SnapshotRecord>,
}

#[derive(Debug)]
pub struct LocalHistoryStore {
    connection: Connection,
    layout: StorageLayout,
    project: ProjectRecord,
}

#[derive(Debug, Clone)]
struct CurrentFileState {
    contents: Vec<u8>,
    is_binary: bool,
    existed: bool,
}

#[derive(Debug, Clone)]
struct BlobReferenceStats {
    ref_counts: HashMap<ContentHash, usize>,
    total_bytes: u64,
}

impl LocalHistoryStore {
    pub fn open_default(project_root: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::open(default_data_dir(), project_root)
    }

    pub fn open_default_for_snapshot(
        snapshot_id: &SnapshotId,
    ) -> Result<Option<Self>, StorageError> {
        Self::open_for_snapshot_id(default_data_dir(), snapshot_id)
    }

    pub fn open(
        base_data_dir: impl AsRef<Path>,
        project_root: impl AsRef<Path>,
    ) -> Result<Self, StorageError> {
        let project_root = normalize_project_root(project_root.as_ref());
        let project_id = project_id_for_root(&project_root);
        let layout = StorageLayout::for_project(base_data_dir, project_id.as_str());

        fs::create_dir_all(&layout.project_dir)?;
        fs::create_dir_all(&layout.blobs_dir)?;
        fs::create_dir_all(&layout.view_dir)?;
        fs::create_dir_all(&layout.logs_dir)?;

        let connection = Connection::open(&layout.database_path)?;
        connection.execute_batch(SCHEMA_SQL)?;
        ensure_schema_compatibility(&connection)?;

        let project = ProjectRecord {
            id: project_id,
            root: project_root,
        };

        connection.execute(
            "INSERT INTO projects (id, root) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET root = excluded.root",
            params![project.id.as_str(), project.root.to_string_lossy().as_ref()],
        )?;

        Ok(Self {
            connection,
            layout,
            project,
        })
    }

    pub fn open_for_snapshot_id(
        base_data_dir: impl AsRef<Path>,
        snapshot_id: &SnapshotId,
    ) -> Result<Option<Self>, StorageError> {
        let base_data_dir = base_data_dir.as_ref();
        let projects_dir = base_data_dir.join("projects");

        if !projects_dir.exists() {
            return Ok(None);
        }

        for entry in fs::read_dir(&projects_dir)? {
            let entry = entry?;

            if !entry.file_type()?.is_dir() {
                continue;
            }

            let database_path = entry.path().join("metadata.sqlite");

            if !database_path.is_file() {
                continue;
            }

            let connection = Connection::open(&database_path)?;
            let project_root: Option<String> = connection
                .query_row(
                    "SELECT projects.root
                     FROM snapshots
                     JOIN projects ON projects.id = snapshots.project_id
                     WHERE snapshots.id = ?1
                     LIMIT 1",
                    params![snapshot_id.as_str()],
                    |row| row.get(0),
                )
                .optional()?;

            if let Some(project_root) = project_root {
                return Self::open(base_data_dir, project_root).map(Some);
            }
        }

        Ok(None)
    }

    pub fn project(&self) -> &ProjectRecord {
        &self.project
    }

    pub fn layout(&self) -> &StorageLayout {
        &self.layout
    }

    pub fn snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<Option<SnapshotRecord>, StorageError> {
        self.get_snapshot(snapshot_id)
    }

    pub fn history_for_hour(&self, hour: &str) -> Result<HourHistory, StorageError> {
        let hour_start = parse_iso_hour(hour)?;
        let hour_end = hour_start + Duration::hours(1);
        let mut segments = Vec::with_capacity(6);
        let mut segment_start = hour_start;

        for _ in 0..6 {
            let segment_end = segment_start + Duration::minutes(10);
            segments.push(self.history_for_segment_timestamps(segment_start, segment_end)?);
            segment_start = segment_end;
        }

        Ok(HourHistory {
            hour: HourBucket {
                from: format_timestamp(hour_start)?,
                to: format_timestamp(hour_end)?,
            },
            segments,
        })
    }

    pub fn history_for_segment(
        &self,
        from_timestamp: &str,
        to_timestamp: &str,
    ) -> Result<SegmentHistory, StorageError> {
        let from = parse_rfc3339(from_timestamp)?;
        let to = parse_rfc3339(to_timestamp)?;

        if from >= to {
            return Err(StorageError::InvalidTimeWindow(format!(
                "segment start must be earlier than end: {from_timestamp} .. {to_timestamp}"
            )));
        }

        self.history_for_segment_timestamps(from, to)
    }

    pub fn query_snapshots(&self, query: &SnapshotQuery) -> Result<SnapshotPage, StorageError> {
        let relative_path = query
            .relative_path
            .as_ref()
            .map(|path| normalize_relative_path(path))
            .transpose()?;
        let relative_path_string = relative_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        let page = std::cmp::max(query.page, 1);
        let page_size = std::cmp::max(query.page_size, 1);
        let offset = ((page - 1) * page_size) as i64;
        let kind = query.kind.as_ref().map(SnapshotKind::as_str);
        let relative_path_param = relative_path_string.as_deref();
        let from_timestamp = query.from_timestamp.as_deref();
        let to_timestamp = query.to_timestamp.as_deref();

        let total_items: usize = self.connection.query_row(
            "SELECT COUNT(*)
             FROM snapshots
             WHERE project_id = :project_id
               AND (:kind IS NULL OR kind = :kind)
               AND (:relative_path IS NULL OR relative_path = :relative_path)
               AND (:from_timestamp IS NULL OR unixepoch(timestamp) >= unixepoch(:from_timestamp))
               AND (:to_timestamp IS NULL OR unixepoch(timestamp) < unixepoch(:to_timestamp))",
            named_params! {
                ":project_id": self.project.id.as_str(),
                ":kind": kind,
                ":relative_path": relative_path_param,
                ":from_timestamp": from_timestamp,
                ":to_timestamp": to_timestamp,
            },
            |row| row.get::<_, i64>(0),
        )? as usize;
        let total_pages = if total_items == 0 {
            0
        } else {
            (total_items + page_size - 1) / page_size
        };
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, relative_path, blob_hash, size_bytes, timestamp, kind,
                    captures_missing_file
             FROM snapshots
             WHERE project_id = :project_id
               AND (:kind IS NULL OR kind = :kind)
               AND (:relative_path IS NULL OR relative_path = :relative_path)
               AND (:from_timestamp IS NULL OR unixepoch(timestamp) >= unixepoch(:from_timestamp))
               AND (:to_timestamp IS NULL OR unixepoch(timestamp) < unixepoch(:to_timestamp))
             ORDER BY unixepoch(timestamp) DESC, id DESC
             LIMIT :limit OFFSET :offset",
        )?;
        let rows = statement.query_map(
            named_params! {
                ":project_id": self.project.id.as_str(),
                ":kind": kind,
                ":relative_path": relative_path_param,
                ":from_timestamp": from_timestamp,
                ":to_timestamp": to_timestamp,
                ":limit": page_size as i64,
                ":offset": offset,
            },
            snapshot_row_mapper,
        )?;
        let items = rows.collect::<Result<Vec<_>, _>>()?;

        Ok(SnapshotPage {
            page,
            page_size,
            total_items,
            total_pages,
            items,
        })
    }

    pub fn recent_snapshots(&self, limit: usize) -> Result<Vec<SnapshotRecord>, StorageError> {
        Ok(self
            .query_snapshots(&SnapshotQuery {
                page_size: limit,
                ..SnapshotQuery::default()
            })?
            .items)
    }

    pub fn recent_raw_snapshots(&self, limit: usize) -> Result<Vec<SnapshotRecord>, StorageError> {
        Ok(self
            .query_snapshots(&SnapshotQuery {
                kind: Some(SnapshotKind::Raw),
                page_size: limit,
                ..SnapshotQuery::default()
            })?
            .items)
    }

    pub fn safety_snapshots(&self, limit: usize) -> Result<Vec<SnapshotRecord>, StorageError> {
        Ok(self
            .query_snapshots(&SnapshotQuery {
                kind: Some(SnapshotKind::Safety),
                page_size: limit,
                ..SnapshotQuery::default()
            })?
            .items)
    }

    pub fn retention_policy(&self) -> RetentionPolicy {
        RetentionPolicy::default()
    }

    pub fn prune(
        &self,
        policy: &RetentionPolicy,
        pruned_at: &str,
    ) -> Result<PruneReport, StorageError> {
        let pruned_at = parse_rfc3339(pruned_at)?.to_offset(UtcOffset::UTC);
        let all_snapshots = self.all_snapshots_newest_first()?;
        let latest_restore_operation = self.latest_restore_operation()?;
        let protected_snapshot_ids =
            protected_snapshot_ids_for_prune(latest_restore_operation.as_ref());
        let protected_snapshot_count = protected_snapshot_ids.len();
        let deleted_restore_operation_count =
            self.prune_restore_operations(latest_restore_operation.as_ref())?;
        let cutoff = pruned_at - Duration::days(policy.max_snapshot_age_days as i64);
        let mut kept_snapshot_ids = all_snapshots
            .iter()
            .map(|snapshot| snapshot.id.clone())
            .collect::<HashSet<_>>();
        let mut pruned_for_age_count = 0usize;
        let mut pruned_for_file_count = 0usize;
        let mut pruned_for_storage_count = 0usize;

        for snapshot in all_snapshots.iter().rev() {
            let snapshot_timestamp = parse_rfc3339(&snapshot.timestamp)?.to_offset(UtcOffset::UTC);

            if snapshot_timestamp < cutoff
                && !protected_snapshot_ids.contains(&snapshot.id)
                && kept_snapshot_ids.remove(&snapshot.id)
            {
                pruned_for_age_count += 1;
            }
        }

        let mut snapshots_by_path = BTreeMap::<PathBuf, Vec<&SnapshotRecord>>::new();

        for snapshot in &all_snapshots {
            if kept_snapshot_ids.contains(&snapshot.id) {
                snapshots_by_path
                    .entry(snapshot.relative_path.clone())
                    .or_default()
                    .push(snapshot);
            }
        }

        for snapshots in snapshots_by_path.values() {
            let mut kept_for_path = 0usize;

            for snapshot in snapshots {
                if !kept_snapshot_ids.contains(&snapshot.id) {
                    continue;
                }

                kept_for_path += 1;

                if kept_for_path <= policy.max_snapshots_per_file {
                    continue;
                }

                if protected_snapshot_ids.contains(&snapshot.id) {
                    continue;
                }

                if kept_snapshot_ids.remove(&snapshot.id) {
                    pruned_for_file_count += 1;
                }
            }
        }

        let blob_sizes = self.all_blob_sizes()?;
        let tracked_hashes = self.tracked_blob_hashes()?;
        let mut blob_stats = build_blob_reference_stats(
            &all_snapshots,
            &kept_snapshot_ids,
            &tracked_hashes,
            &blob_sizes,
        );

        if blob_stats.total_bytes > policy.max_project_storage_bytes {
            for snapshot in all_snapshots.iter().rev() {
                if !kept_snapshot_ids.contains(&snapshot.id)
                    || protected_snapshot_ids.contains(&snapshot.id)
                {
                    continue;
                }

                kept_snapshot_ids.remove(&snapshot.id);
                decrement_blob_reference(
                    &mut blob_stats,
                    &snapshot.blob_hash,
                    blob_sizes.get(&snapshot.blob_hash).copied().unwrap_or(0),
                );
                pruned_for_storage_count += 1;

                if blob_stats.total_bytes <= policy.max_project_storage_bytes {
                    break;
                }
            }
        }

        let deleted_snapshots = all_snapshots
            .iter()
            .filter(|snapshot| !kept_snapshot_ids.contains(&snapshot.id))
            .map(|snapshot| snapshot.id.clone())
            .collect::<Vec<_>>();

        for snapshot_id in &deleted_snapshots {
            self.connection.execute(
                "DELETE FROM snapshots WHERE id = ?1",
                params![snapshot_id.as_str()],
            )?;
        }

        let (deleted_blob_count, deleted_blob_bytes) = self.delete_orphaned_blobs()?;
        self.rebuild_markdown_view(&format_timestamp(pruned_at)?)?;
        let remaining_snapshot_count = self.snapshot_count()?;
        let remaining_referenced_blob_bytes = self.current_referenced_blob_bytes()?;

        Ok(PruneReport {
            pruned_at: format_timestamp(pruned_at)?,
            deleted_restore_operation_count,
            deleted_snapshot_count: deleted_snapshots.len(),
            deleted_blob_count,
            deleted_blob_bytes,
            remaining_snapshot_count,
            remaining_referenced_blob_bytes,
            protected_snapshot_count,
            pruned_for_age_count,
            pruned_for_file_count,
            pruned_for_storage_count,
            rebuilt_markdown_view: true,
        })
    }

    pub fn render_hour_markdown(
        &self,
        hour: &str,
        generated_at: &str,
    ) -> Result<GeneratedMarkdownViewEntry, StorageError> {
        let hour_start = parse_iso_hour(hour)?;
        let entries = self.render_hour_view(hour_start, generated_at)?;

        self.write_root_markdown_index(generated_at)?;

        entries
            .into_iter()
            .find(|entry| entry.relative_markdown_path == hour_markdown_relative_path(hour_start))
            .ok_or_else(|| {
                StorageError::InvalidRelativePath(format!(
                    "missing generated hour README for {hour}"
                ))
            })
    }

    pub fn render_segment_markdown(
        &self,
        from_timestamp: &str,
        to_timestamp: &str,
        generated_at: &str,
    ) -> Result<GeneratedMarkdownViewEntry, StorageError> {
        let from = parse_rfc3339(from_timestamp)?.to_offset(UtcOffset::UTC);
        let to = parse_rfc3339(to_timestamp)?.to_offset(UtcOffset::UTC);
        validate_fixed_ten_minute_segment(from, to, from_timestamp, to_timestamp)?;

        let hour_start = start_of_hour(from);
        let target_path = segment_markdown_relative_path(from)?;
        let entries = self.render_hour_view(hour_start, generated_at)?;

        self.write_root_markdown_index(generated_at)?;

        entries
            .into_iter()
            .find(|entry| entry.relative_markdown_path == target_path)
            .ok_or_else(|| {
                StorageError::InvalidRelativePath(format!(
                    "missing generated segment Markdown for {from_timestamp} .. {to_timestamp}"
                ))
            })
    }

    pub fn rebuild_markdown_view(
        &self,
        generated_at: &str,
    ) -> Result<Vec<GeneratedMarkdownViewEntry>, StorageError> {
        clear_directory_if_exists(&self.layout.view_dir)?;
        fs::create_dir_all(&self.layout.view_dir)?;
        self.clear_generated_markdown_index()?;

        let mut entries = Vec::new();

        for (hour, _) in self.raw_snapshot_hour_counts()? {
            let hour_start = parse_iso_hour(&hour)?;
            entries.extend(self.render_hour_view(hour_start, generated_at)?);
        }

        entries.push(self.write_root_markdown_index(generated_at)?);
        Ok(entries)
    }

    pub fn store_snapshot(
        &self,
        request: SnapshotWriteRequest,
    ) -> Result<SnapshotRecord, StorageError> {
        let policy = self.retention_policy();
        let size_bytes = request.contents.len() as u64;

        if size_bytes > policy.max_file_size_bytes {
            return Err(StorageError::SnapshotTooLarge {
                size_bytes,
                max_bytes: policy.max_file_size_bytes,
            });
        }

        let relative_path = normalize_relative_path(&request.relative_path)?;
        let blob = self.store_blob(&request.contents)?;
        let snapshot = SnapshotRecord {
            id: SnapshotId::new(snapshot_id(
                self.project.id.as_str(),
                &relative_path,
                &request.timestamp,
                &request.kind,
                blob.hash.as_str(),
            )),
            project_id: self.project.id.clone(),
            relative_path: relative_path.clone(),
            blob_hash: blob.hash.clone(),
            size_bytes,
            timestamp: request.timestamp,
            kind: request.kind,
            captures_missing_file: request.captures_missing_file,
        };

        if snapshot.captures_missing_file {
            self.delete_tracked_file(&relative_path)?;
        } else {
            self.upsert_tracked_file(TrackedFileRecord {
                project_id: self.project.id.clone(),
                relative_path: relative_path.clone(),
                current_content_hash: blob.hash,
                size_bytes: snapshot.size_bytes,
                is_binary: request.is_binary,
            })?;
        }

        self.connection.execute(
            "INSERT INTO snapshots (
                id,
                project_id,
                relative_path,
                blob_hash,
                size_bytes,
                timestamp,
                kind,
                captures_missing_file
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                snapshot.id.as_str(),
                snapshot.project_id.as_str(),
                snapshot.relative_path.to_string_lossy().as_ref(),
                snapshot.blob_hash.as_str(),
                snapshot.size_bytes as i64,
                snapshot.timestamp.as_str(),
                snapshot.kind.as_str(),
                snapshot.captures_missing_file,
            ],
        )?;

        Ok(snapshot)
    }

    pub fn read_snapshot_content(&self, snapshot_id: &SnapshotId) -> Result<Vec<u8>, StorageError> {
        let snapshot = self
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| StorageError::SnapshotNotFound(snapshot_id.as_str().to_string()))?;

        self.read_blob(&snapshot.blob_hash)
    }

    pub fn latest_restore_operation(&self) -> Result<Option<RestoreOperationRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id, project_id, relative_path, restored_snapshot_id, safety_snapshot_id,
                        previous_file_existed, previous_content_hash, restored_content_hash,
                        timestamp
                 FROM restore_operations
                 WHERE project_id = ?1
                 ORDER BY timestamp DESC, id DESC
                 LIMIT 1",
                params![self.project.id.as_str()],
                restore_operation_row_mapper,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn restore_snapshot(
        &self,
        snapshot_id: &SnapshotId,
        timestamp: &str,
    ) -> Result<RestoreOutcome, StorageError> {
        let snapshot = self
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| StorageError::SnapshotNotFound(snapshot_id.as_str().to_string()))?;
        let current_state = self.read_current_file_state(&snapshot.relative_path)?;
        let safety_snapshot = self.store_snapshot(SnapshotWriteRequest {
            relative_path: snapshot.relative_path.clone(),
            contents: current_state.contents.clone(),
            timestamp: timestamp.to_string(),
            kind: SnapshotKind::Safety,
            is_binary: current_state.is_binary,
            captures_missing_file: !current_state.existed,
        })?;
        let restore_path = self.apply_snapshot(&snapshot)?;
        let operation = self.record_restore_operation(
            &snapshot,
            &safety_snapshot,
            current_state.existed,
            timestamp,
        )?;

        Ok(RestoreOutcome {
            restored_snapshot: snapshot,
            safety_snapshot,
            operation,
            restored_path: restore_path,
        })
    }

    pub fn undo_last_restore(&self, timestamp: &str) -> Result<RestoreOutcome, StorageError> {
        let operation = self.latest_restore_operation()?.ok_or_else(|| {
            StorageError::RestoreOperationNotFound(self.project.id.as_str().to_string())
        })?;

        self.restore_snapshot(&operation.safety_snapshot_id, timestamp)
    }

    pub fn restore_last_safety_snapshot(
        &self,
        timestamp: &str,
    ) -> Result<RestoreOutcome, StorageError> {
        let snapshot = self
            .safety_snapshots(1)?
            .into_iter()
            .next()
            .ok_or_else(|| {
                StorageError::SafetySnapshotNotFound(self.project.id.as_str().to_string())
            })?;

        self.restore_snapshot(&snapshot.id, timestamp)
    }

    pub fn read_blob(&self, content_hash: &ContentHash) -> Result<Vec<u8>, StorageError> {
        let blob = self
            .get_blob(content_hash)?
            .ok_or_else(|| StorageError::BlobNotFound(content_hash.as_str().to_string()))?;
        let absolute_path = self.layout.project_dir.join(blob.storage_path);
        let compressed = fs::read(absolute_path)?;

        zstd::stream::decode_all(Cursor::new(compressed)).map_err(StorageError::from)
    }

    fn store_blob(&self, contents: &[u8]) -> Result<ContentBlobRecord, StorageError> {
        let hash = ContentHash::new(sha256_hex(contents));
        let storage_path = blob_storage_relative_path(hash.as_str());
        let absolute_storage_path = self.layout.project_dir.join(&storage_path);

        if !absolute_storage_path.exists() {
            let parent = absolute_storage_path.parent().ok_or_else(|| {
                StorageError::InvalidRelativePath(storage_path.display().to_string())
            })?;

            fs::create_dir_all(parent)?;
            let compressed = zstd::stream::encode_all(Cursor::new(contents), 0)?;
            fs::write(&absolute_storage_path, compressed)?;
        }

        let blob = ContentBlobRecord {
            hash,
            size_bytes: contents.len() as u64,
            compression: CompressionKind::Zstd,
            storage_path,
        };

        self.connection.execute(
            "INSERT OR IGNORE INTO content_blobs (
                content_hash,
                size_bytes,
                compression,
                storage_path
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                blob.hash.as_str(),
                blob.size_bytes as i64,
                blob.compression.as_str(),
                blob.storage_path.to_string_lossy().as_ref(),
            ],
        )?;

        Ok(blob)
    }

    fn read_current_file_state(
        &self,
        relative_path: &Path,
    ) -> Result<CurrentFileState, StorageError> {
        let absolute_path = self.project.root.join(relative_path);

        if !absolute_path.exists() {
            return Ok(CurrentFileState {
                contents: Vec::new(),
                is_binary: false,
                existed: false,
            });
        }

        let contents = fs::read(&absolute_path)?;

        Ok(CurrentFileState {
            is_binary: std::str::from_utf8(&contents).is_err(),
            contents,
            existed: true,
        })
    }

    fn render_hour_view(
        &self,
        hour_start: OffsetDateTime,
        generated_at: &str,
    ) -> Result<Vec<GeneratedMarkdownViewEntry>, StorageError> {
        let history = self.history_for_hour(&format_iso_hour(hour_start)?)?;
        let hour_prefix = hour_directory_relative_path(hour_start);
        let hour_dir = self.layout.view_dir.join(&hour_prefix);
        let snapshots_dir = hour_dir.join("snapshots");

        clear_directory_if_exists(&hour_dir)?;
        fs::create_dir_all(&snapshots_dir)?;
        self.clear_generated_markdown_entries_under(&hour_prefix)?;

        let mut entries = Vec::new();
        let mut snapshot_links = HashMap::<SnapshotId, PathBuf>::new();
        let mut written_snapshot_ids = HashSet::<SnapshotId>::new();

        for segment in &history.segments {
            for file_history in &segment.affected_files {
                for snapshot in &file_history.snapshots {
                    if !written_snapshot_ids.insert(snapshot.id.clone()) {
                        continue;
                    }

                    let snapshot_entry = self.write_snapshot_markdown(
                        snapshot,
                        generated_at,
                        &snapshots_dir,
                        &hour_prefix,
                    )?;
                    snapshot_links.insert(
                        snapshot.id.clone(),
                        snapshot_entry.relative_markdown_path.clone(),
                    );
                    entries.push(snapshot_entry);
                }
            }
        }

        for segment in &history.segments {
            entries.push(self.write_segment_markdown(
                segment,
                generated_at,
                &hour_dir,
                &hour_prefix,
                &snapshot_links,
            )?);
        }

        entries.push(self.write_hour_markdown(&history, generated_at, &hour_dir, &hour_prefix)?);

        Ok(entries)
    }

    fn history_for_segment_timestamps(
        &self,
        from: OffsetDateTime,
        to: OffsetDateTime,
    ) -> Result<SegmentHistory, StorageError> {
        let from_timestamp = format_timestamp(from)?;
        let to_timestamp = format_timestamp(to)?;
        let snapshots = self.raw_snapshots_in_window(&from_timestamp, &to_timestamp)?;
        let mut affected_files = BTreeMap::<PathBuf, Vec<SnapshotRecord>>::new();

        for snapshot in snapshots {
            affected_files
                .entry(snapshot.relative_path.clone())
                .or_default()
                .push(snapshot);
        }

        let label = segment_label(from.hour(), from.minute()).ok_or_else(|| {
            StorageError::InvalidTimeWindow(format!(
                "failed to derive 10-minute segment label for {}",
                from_timestamp
            ))
        })?;
        let affected_files = affected_files
            .into_iter()
            .map(|(relative_path, snapshots)| WindowedFileHistory {
                snapshot_count: snapshots.len(),
                relative_path,
                snapshots,
            })
            .collect();

        Ok(SegmentHistory {
            segment: TimeSegment {
                label,
                from: from_timestamp,
                to: to_timestamp,
            },
            affected_files,
        })
    }

    fn write_snapshot_markdown(
        &self,
        snapshot: &SnapshotRecord,
        generated_at: &str,
        snapshots_dir: &Path,
        hour_prefix: &Path,
    ) -> Result<GeneratedMarkdownViewEntry, StorageError> {
        let timestamp = parse_rfc3339(&snapshot.timestamp)?.to_offset(UtcOffset::UTC);
        let file_name =
            snapshot_markdown_file_name(timestamp, &snapshot.relative_path, &snapshot.id);
        let absolute_path = snapshots_dir.join(&file_name);
        let relative_path = hour_prefix.join("snapshots").join(&file_name);
        let contents = self.read_snapshot_content(&snapshot.id)?;
        let preview = render_snapshot_markdown_preview(snapshot, &contents);
        let title = format!(
            "{} {}",
            short_id(snapshot.id.as_str()),
            snapshot.relative_path.display()
        );
        let body = format!(
            "# Snapshot {}\n\n\
             - Snapshot ID: `{}`\n\
             - Project ID: `{}`\n\
             - Project Root: `{}`\n\
             - Relative Path: `{}`\n\
             - Timestamp: `{}`\n\
             - Kind: `{}`\n\
             - Content Hash: `{}`\n\
             - Size Bytes: `{}`\n\
             - Captures Missing File: `{}`\n\
             - Generated At: `{}`\n\n\
             ## Restore\n\n\
             ```bash\n\
             cargo run -p local-history-cli -- restore {}\n\
             ```\n\n\
             ## Preview\n\n\
             ```text\n\
             {}\n\
             ```\n",
            short_id(snapshot.id.as_str()),
            snapshot.id.as_str(),
            snapshot.project_id.as_str(),
            self.project.root.display(),
            snapshot.relative_path.display(),
            snapshot.timestamp,
            snapshot.kind.as_str(),
            snapshot.blob_hash.as_str(),
            snapshot.size_bytes,
            snapshot.captures_missing_file,
            generated_at,
            snapshot.id.as_str(),
            preview
        );

        fs::write(absolute_path, body)?;
        self.upsert_generated_markdown_entry(&relative_path, &title, generated_at)?;

        Ok(GeneratedMarkdownViewEntry {
            relative_markdown_path: relative_path,
            title,
            generated_at: generated_at.to_string(),
        })
    }

    fn write_segment_markdown(
        &self,
        history: &SegmentHistory,
        generated_at: &str,
        hour_dir: &Path,
        hour_prefix: &Path,
        snapshot_links: &HashMap<SnapshotId, PathBuf>,
    ) -> Result<GeneratedMarkdownViewEntry, StorageError> {
        let file_name = format!("{}.md", history.segment.label);
        let absolute_path = hour_dir.join(&file_name);
        let relative_path = hour_prefix.join(&file_name);
        let title = format!(
            "Segment {}",
            human_window_label(&history.segment.from, &history.segment.to)
        );
        let mut body = String::new();

        body.push_str(&format!("# {}\n\n", title));
        body.push_str(&format!(
            "- Project ID: `{}`\n- Project Root: `{}`\n- Window: `{}` -> `{}`\n- Generated At: `{}`\n\n",
            self.project.id.as_str(),
            self.project.root.display(),
            history.segment.from,
            history.segment.to,
            generated_at
        ));

        if history.affected_files.is_empty() {
            body.push_str("No raw snapshots were captured in this segment.\n");
        } else {
            body.push_str("## Affected Files\n\n");

            for file_history in &history.affected_files {
                body.push_str(&format!(
                    "### `{}`\n\n",
                    file_history.relative_path.display()
                ));
                body.push_str(&format!("- Snapshots: `{}`\n", file_history.snapshot_count));

                for snapshot in &file_history.snapshots {
                    let link = snapshot_links.get(&snapshot.id).ok_or_else(|| {
                        StorageError::SnapshotNotFound(snapshot.id.as_str().to_string())
                    })?;
                    let link_name =
                        path_to_slash_string(relative_path_from_hour_dir(&relative_path, link)?);

                    body.push_str(&format!(
                        "- [{} — {}](./{})\n",
                        human_timestamp(&snapshot.timestamp),
                        short_id(snapshot.id.as_str()),
                        link_name
                    ));
                }

                body.push('\n');
            }
        }

        body.push_str("## Restore Examples\n\n");

        if history.affected_files.is_empty() {
            body.push_str("No restore targets are available for this segment yet.\n");
        } else {
            let mut snapshot_ids = BTreeSet::new();

            for file_history in &history.affected_files {
                for snapshot in &file_history.snapshots {
                    snapshot_ids.insert(snapshot.id.as_str().to_string());
                }
            }

            for snapshot_id in snapshot_ids {
                body.push_str("```bash\n");
                body.push_str(&format!(
                    "cargo run -p local-history-cli -- restore {snapshot_id}\n"
                ));
                body.push_str("```\n\n");
            }
        }

        fs::write(absolute_path, body)?;
        self.upsert_generated_markdown_entry(&relative_path, &title, generated_at)?;

        Ok(GeneratedMarkdownViewEntry {
            relative_markdown_path: relative_path,
            title,
            generated_at: generated_at.to_string(),
        })
    }

    fn write_hour_markdown(
        &self,
        history: &HourHistory,
        generated_at: &str,
        hour_dir: &Path,
        hour_prefix: &Path,
    ) -> Result<GeneratedMarkdownViewEntry, StorageError> {
        let absolute_path = hour_dir.join("README.md");
        let relative_path = hour_prefix.join("README.md");
        let title = format!(
            "Hour {}",
            human_window_label(&history.hour.from, &history.hour.to)
        );
        let mut body = String::new();

        body.push_str(&format!("# {}\n\n", title));
        body.push_str(&format!(
            "- Project ID: `{}`\n- Project Root: `{}`\n- Window: `{}` -> `{}`\n- Generated At: `{}`\n\n",
            self.project.id.as_str(),
            self.project.root.display(),
            history.hour.from,
            history.hour.to,
            generated_at
        ));
        body.push_str("## Segments\n\n");

        for segment in &history.segments {
            let snapshot_count: usize = segment
                .affected_files
                .iter()
                .map(|file_history| file_history.snapshot_count)
                .sum();
            let file_count = segment.affected_files.len();

            body.push_str(&format!(
                "- [{}]({}.md) — {} snapshots across {} files\n",
                human_window_label(&segment.segment.from, &segment.segment.to),
                segment.segment.label,
                snapshot_count,
                file_count
            ));
        }

        body.push_str("\n## Rebuild\n\n```bash\n");
        body.push_str(&format!(
            "cargo run -p local-history-cli -- rebuild-markdown-view {}\n",
            self.project.root.display()
        ));
        body.push_str("```\n");

        fs::write(absolute_path, body)?;
        self.upsert_generated_markdown_entry(&relative_path, &title, generated_at)?;

        Ok(GeneratedMarkdownViewEntry {
            relative_markdown_path: relative_path,
            title,
            generated_at: generated_at.to_string(),
        })
    }

    fn write_root_markdown_index(
        &self,
        generated_at: &str,
    ) -> Result<GeneratedMarkdownViewEntry, StorageError> {
        fs::create_dir_all(&self.layout.view_dir)?;

        let absolute_path = self.layout.view_dir.join("README.md");
        let relative_path = PathBuf::from("README.md");
        let title = "Local History View".to_string();
        let mut body = String::new();

        body.push_str("# Local History View\n\n");
        body.push_str(&format!(
            "- Project ID: `{}`\n- Project Root: `{}`\n- Generated At: `{}`\n\n",
            self.project.id.as_str(),
            self.project.root.display(),
            generated_at
        ));
        body.push_str("## Hours\n\n");

        let hour_counts = self.raw_snapshot_hour_counts()?;

        if hour_counts.is_empty() {
            body.push_str("No raw snapshots have been captured yet.\n");
        } else {
            for (hour, count) in hour_counts {
                let hour_start = parse_iso_hour(&hour)?;
                let link = path_to_slash_string(hour_markdown_relative_path(hour_start));
                body.push_str(&format!(
                    "- [{}](./{}) — {} snapshots\n",
                    human_hour_label(hour_start),
                    link,
                    count
                ));
            }
        }

        body.push_str("\n## Commands\n\n```bash\n");
        body.push_str(&format!(
            "cargo run -p local-history-cli -- view-root {}\n",
            self.project.root.display()
        ));
        body.push_str(&format!(
            "cargo run -p local-history-cli -- rebuild-markdown-view {}\n",
            self.project.root.display()
        ));
        body.push_str("```\n");

        fs::write(absolute_path, body)?;
        self.upsert_generated_markdown_entry(&relative_path, &title, generated_at)?;

        Ok(GeneratedMarkdownViewEntry {
            relative_markdown_path: relative_path,
            title,
            generated_at: generated_at.to_string(),
        })
    }

    fn upsert_generated_markdown_entry(
        &self,
        relative_path: &Path,
        title: &str,
        generated_at: &str,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO generated_markdown_views (
                project_id,
                relative_markdown_path,
                title,
                generated_at
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project_id, relative_markdown_path)
             DO UPDATE SET
                title = excluded.title,
                generated_at = excluded.generated_at",
            params![
                self.project.id.as_str(),
                path_to_slash_string(relative_path),
                title,
                generated_at,
            ],
        )?;

        Ok(())
    }

    fn clear_generated_markdown_index(&self) -> Result<(), StorageError> {
        self.connection.execute(
            "DELETE FROM generated_markdown_views WHERE project_id = ?1",
            params![self.project.id.as_str()],
        )?;

        Ok(())
    }

    fn clear_generated_markdown_entries_under(&self, prefix: &Path) -> Result<(), StorageError> {
        let prefix = path_to_slash_string(prefix);
        let like_pattern = format!("{prefix}/%");

        self.connection.execute(
            "DELETE FROM generated_markdown_views
             WHERE project_id = ?1
               AND (relative_markdown_path = ?2 OR relative_markdown_path LIKE ?3)",
            params![self.project.id.as_str(), prefix, like_pattern],
        )?;

        Ok(())
    }

    fn raw_snapshot_hour_counts(&self) -> Result<Vec<(String, usize)>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT timestamp
             FROM snapshots
             WHERE project_id = ?1 AND kind = ?2
             ORDER BY unixepoch(timestamp) DESC, id DESC",
        )?;
        let rows = statement.query_map(
            params![self.project.id.as_str(), SnapshotKind::Raw.as_str()],
            |row| row.get::<_, String>(0),
        )?;
        let mut counts = BTreeMap::<String, usize>::new();

        for timestamp in rows {
            let timestamp = timestamp?;
            let hour_key = format_iso_hour(parse_rfc3339(&timestamp)?.to_offset(UtcOffset::UTC))?;
            *counts.entry(hour_key).or_default() += 1;
        }

        let mut items = counts.into_iter().collect::<Vec<_>>();
        items.sort_by(|left, right| right.0.cmp(&left.0));
        Ok(items)
    }

    fn raw_snapshots_in_window(
        &self,
        from_timestamp: &str,
        to_timestamp: &str,
    ) -> Result<Vec<SnapshotRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, relative_path, blob_hash, size_bytes, timestamp, kind,
                    captures_missing_file
             FROM snapshots
             WHERE project_id = :project_id
               AND kind = :kind
               AND unixepoch(timestamp) >= unixepoch(:from_timestamp)
               AND unixepoch(timestamp) < unixepoch(:to_timestamp)
             ORDER BY unixepoch(timestamp) DESC, id DESC",
        )?;
        let rows = statement.query_map(
            named_params! {
                ":project_id": self.project.id.as_str(),
                ":kind": SnapshotKind::Raw.as_str(),
                ":from_timestamp": from_timestamp,
                ":to_timestamp": to_timestamp,
            },
            snapshot_row_mapper,
        )?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    fn apply_snapshot(&self, snapshot: &SnapshotRecord) -> Result<PathBuf, StorageError> {
        let restore_path = self.project.root.join(&snapshot.relative_path);

        if snapshot.captures_missing_file {
            if restore_path.exists() {
                fs::remove_file(&restore_path)?;
            }

            self.delete_tracked_file(&snapshot.relative_path)?;
            return Ok(restore_path);
        }

        let contents = self.read_blob(&snapshot.blob_hash)?;

        if let Some(parent) = restore_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&restore_path, &contents)?;
        self.upsert_tracked_file(TrackedFileRecord {
            project_id: self.project.id.clone(),
            relative_path: snapshot.relative_path.clone(),
            current_content_hash: snapshot.blob_hash.clone(),
            size_bytes: snapshot.size_bytes,
            is_binary: std::str::from_utf8(&contents).is_err(),
        })?;

        Ok(restore_path)
    }

    fn record_restore_operation(
        &self,
        restored_snapshot: &SnapshotRecord,
        safety_snapshot: &SnapshotRecord,
        previous_file_existed: bool,
        timestamp: &str,
    ) -> Result<RestoreOperationRecord, StorageError> {
        let operation = RestoreOperationRecord {
            id: restore_operation_id(
                self.project.id.as_str(),
                restored_snapshot.id.as_str(),
                safety_snapshot.id.as_str(),
                timestamp,
            ),
            project_id: self.project.id.clone(),
            relative_path: restored_snapshot.relative_path.clone(),
            restored_snapshot_id: restored_snapshot.id.clone(),
            safety_snapshot_id: safety_snapshot.id.clone(),
            previous_file_existed,
            previous_content_hash: safety_snapshot.blob_hash.clone(),
            restored_content_hash: restored_snapshot.blob_hash.clone(),
            timestamp: timestamp.to_string(),
        };

        self.connection.execute(
            "INSERT INTO restore_operations (
                id,
                project_id,
                relative_path,
                restored_snapshot_id,
                safety_snapshot_id,
                previous_file_existed,
                previous_content_hash,
                restored_content_hash,
                timestamp
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                operation.id.as_str(),
                operation.project_id.as_str(),
                operation.relative_path.to_string_lossy().as_ref(),
                operation.restored_snapshot_id.as_str(),
                operation.safety_snapshot_id.as_str(),
                operation.previous_file_existed,
                operation.previous_content_hash.as_str(),
                operation.restored_content_hash.as_str(),
                operation.timestamp.as_str(),
            ],
        )?;

        Ok(operation)
    }

    fn upsert_tracked_file(&self, tracked_file: TrackedFileRecord) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO tracked_files (
                project_id,
                relative_path,
                current_content_hash,
                size_bytes,
                is_binary
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(project_id, relative_path) DO UPDATE SET
                current_content_hash = excluded.current_content_hash,
                size_bytes = excluded.size_bytes,
                is_binary = excluded.is_binary",
            params![
                tracked_file.project_id.as_str(),
                tracked_file.relative_path.to_string_lossy().as_ref(),
                tracked_file.current_content_hash.as_str(),
                tracked_file.size_bytes as i64,
                tracked_file.is_binary
            ],
        )?;

        Ok(())
    }

    fn delete_tracked_file(&self, relative_path: &Path) -> Result<(), StorageError> {
        self.connection.execute(
            "DELETE FROM tracked_files
             WHERE project_id = ?1 AND relative_path = ?2",
            params![
                self.project.id.as_str(),
                relative_path.to_string_lossy().as_ref(),
            ],
        )?;

        Ok(())
    }

    fn get_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<Option<SnapshotRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id, project_id, relative_path, blob_hash, size_bytes, timestamp, kind,
                        captures_missing_file
                 FROM snapshots
                 WHERE id = ?1",
                params![snapshot_id.as_str()],
                snapshot_row_mapper,
            )
            .optional()
            .map_err(StorageError::from)
    }

    fn get_blob(
        &self,
        content_hash: &ContentHash,
    ) -> Result<Option<ContentBlobRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT content_hash, size_bytes, compression, storage_path
                 FROM content_blobs
                 WHERE content_hash = ?1",
                params![content_hash.as_str()],
                |row| {
                    let compression: String = row.get(2)?;

                    Ok(ContentBlobRecord {
                        hash: ContentHash::new(row.get::<_, String>(0)?),
                        size_bytes: row.get::<_, i64>(1)? as u64,
                        compression: CompressionKind::from_db_value(&compression),
                        storage_path: PathBuf::from(row.get::<_, String>(3)?),
                    })
                },
            )
            .optional()
            .map_err(StorageError::from)
    }

    fn all_snapshots_newest_first(&self) -> Result<Vec<SnapshotRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, relative_path, blob_hash, size_bytes, timestamp, kind,
                    captures_missing_file
             FROM snapshots
             WHERE project_id = ?1
             ORDER BY unixepoch(timestamp) DESC, id DESC",
        )?;
        let rows = statement.query_map(params![self.project.id.as_str()], snapshot_row_mapper)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    fn prune_restore_operations(
        &self,
        latest_operation: Option<&RestoreOperationRecord>,
    ) -> Result<usize, StorageError> {
        let deleted = if let Some(latest_operation) = latest_operation {
            self.connection.execute(
                "DELETE FROM restore_operations
                 WHERE project_id = ?1 AND id <> ?2",
                params![self.project.id.as_str(), latest_operation.id.as_str()],
            )?
        } else {
            self.connection.execute(
                "DELETE FROM restore_operations WHERE project_id = ?1",
                params![self.project.id.as_str()],
            )?
        };

        Ok(deleted)
    }

    fn tracked_blob_hashes(&self) -> Result<HashSet<ContentHash>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT current_content_hash
             FROM tracked_files
             WHERE project_id = ?1",
        )?;
        let rows = statement.query_map(params![self.project.id.as_str()], |row| {
            Ok(ContentHash::new(row.get::<_, String>(0)?))
        })?;
        let mut hashes = HashSet::new();

        for row in rows {
            hashes.insert(row?);
        }

        Ok(hashes)
    }

    fn all_blob_sizes(&self) -> Result<HashMap<ContentHash, u64>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT content_hash, size_bytes
             FROM content_blobs",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                ContentHash::new(row.get::<_, String>(0)?),
                row.get::<_, i64>(1)? as u64,
            ))
        })?;
        let mut sizes = HashMap::new();

        for row in rows {
            let (hash, size_bytes) = row?;
            sizes.insert(hash, size_bytes);
        }

        Ok(sizes)
    }

    fn snapshot_count(&self) -> Result<usize, StorageError> {
        self.connection
            .query_row(
                "SELECT COUNT(*)
                 FROM snapshots
                 WHERE project_id = ?1",
                params![self.project.id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count as usize)
            .map_err(StorageError::from)
    }

    fn current_referenced_blob_bytes(&self) -> Result<u64, StorageError> {
        let blob_sizes = self.all_blob_sizes()?;
        let tracked_hashes = self.tracked_blob_hashes()?;
        let snapshots = self.all_snapshots_newest_first()?;
        let kept_snapshot_ids = snapshots
            .iter()
            .map(|snapshot| snapshot.id.clone())
            .collect::<HashSet<_>>();

        Ok(
            build_blob_reference_stats(
                &snapshots,
                &kept_snapshot_ids,
                &tracked_hashes,
                &blob_sizes,
            )
            .total_bytes,
        )
    }

    fn delete_orphaned_blobs(&self) -> Result<(usize, u64), StorageError> {
        let referenced_hashes = self.referenced_blob_hashes()?;
        let mut statement = self.connection.prepare(
            "SELECT content_hash, size_bytes, compression, storage_path
             FROM content_blobs",
        )?;
        let rows = statement.query_map([], |row| {
            let compression: String = row.get(2)?;

            Ok(ContentBlobRecord {
                hash: ContentHash::new(row.get::<_, String>(0)?),
                size_bytes: row.get::<_, i64>(1)? as u64,
                compression: CompressionKind::from_db_value(&compression),
                storage_path: PathBuf::from(row.get::<_, String>(3)?),
            })
        })?;
        let mut deleted_blob_count = 0usize;
        let mut deleted_blob_bytes = 0u64;

        for row in rows {
            let blob = row?;

            if referenced_hashes.contains(&blob.hash) {
                continue;
            }

            let absolute_path = self.layout.project_dir.join(&blob.storage_path);

            if absolute_path.exists() {
                fs::remove_file(&absolute_path)?;

                if let Some(parent) = absolute_path.parent() {
                    remove_directory_if_empty(parent)?;
                }
            }

            self.connection.execute(
                "DELETE FROM content_blobs WHERE content_hash = ?1",
                params![blob.hash.as_str()],
            )?;
            deleted_blob_count += 1;
            deleted_blob_bytes += blob.size_bytes;
        }

        Ok((deleted_blob_count, deleted_blob_bytes))
    }

    fn referenced_blob_hashes(&self) -> Result<HashSet<ContentHash>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT blob_hash
             FROM snapshots
             WHERE project_id = ?1",
        )?;
        let rows = statement.query_map(params![self.project.id.as_str()], |row| {
            Ok(ContentHash::new(row.get::<_, String>(0)?))
        })?;
        let mut hashes = self.tracked_blob_hashes()?;

        for row in rows {
            hashes.insert(row?);
        }

        Ok(hashes)
    }
}

fn build_blob_reference_stats(
    snapshots: &[SnapshotRecord],
    kept_snapshot_ids: &HashSet<SnapshotId>,
    tracked_hashes: &HashSet<ContentHash>,
    blob_sizes: &HashMap<ContentHash, u64>,
) -> BlobReferenceStats {
    let mut ref_counts = HashMap::<ContentHash, usize>::new();
    let mut total_bytes = 0u64;

    for content_hash in tracked_hashes {
        increment_blob_reference(&mut ref_counts, &mut total_bytes, content_hash, blob_sizes);
    }

    for snapshot in snapshots {
        if kept_snapshot_ids.contains(&snapshot.id) {
            increment_blob_reference(
                &mut ref_counts,
                &mut total_bytes,
                &snapshot.blob_hash,
                blob_sizes,
            );
        }
    }

    BlobReferenceStats {
        ref_counts,
        total_bytes,
    }
}

fn protected_snapshot_ids_for_prune(
    latest_operation: Option<&RestoreOperationRecord>,
) -> HashSet<SnapshotId> {
    let mut ids = HashSet::new();

    if let Some(latest_operation) = latest_operation {
        ids.insert(latest_operation.restored_snapshot_id.clone());
        ids.insert(latest_operation.safety_snapshot_id.clone());
    }

    ids
}

fn increment_blob_reference(
    ref_counts: &mut HashMap<ContentHash, usize>,
    total_bytes: &mut u64,
    content_hash: &ContentHash,
    blob_sizes: &HashMap<ContentHash, u64>,
) {
    let entry = ref_counts.entry(content_hash.clone()).or_default();

    if *entry == 0 {
        *total_bytes += blob_sizes.get(content_hash).copied().unwrap_or(0);
    }

    *entry += 1;
}

fn decrement_blob_reference(
    stats: &mut BlobReferenceStats,
    content_hash: &ContentHash,
    size_bytes: u64,
) {
    if let Some(count) = stats.ref_counts.get_mut(content_hash) {
        *count = count.saturating_sub(1);

        if *count == 0 {
            stats.ref_counts.remove(content_hash);
            stats.total_bytes = stats.total_bytes.saturating_sub(size_bytes);
        }
    }
}

fn normalize_relative_path(path: &Path) -> Result<PathBuf, StorageError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(StorageError::InvalidRelativePath(
            path.display().to_string(),
        ));
    }

    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(StorageError::InvalidRelativePath(
                    path.display().to_string(),
                ));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(StorageError::InvalidRelativePath(
            path.display().to_string(),
        ));
    }

    Ok(normalized)
}

fn blob_storage_relative_path(content_hash: &str) -> PathBuf {
    PathBuf::from("blobs")
        .join(&content_hash[..2])
        .join(format!("{content_hash}.zst"))
}

fn snapshot_id(
    project_id: &str,
    relative_path: &Path,
    timestamp: &str,
    kind: &SnapshotKind,
    blob_hash: &str,
) -> String {
    let input = format!(
        "{}\0{}\0{}\0{}\0{}",
        project_id,
        relative_path.display(),
        timestamp,
        kind.as_str(),
        blob_hash
    );
    let digest = sha256_hex(input.as_bytes());

    digest[..24].to_string()
}

fn restore_operation_id(
    project_id: &str,
    restored_snapshot_id: &str,
    safety_snapshot_id: &str,
    timestamp: &str,
) -> String {
    let input = format!("{project_id}\0{restored_snapshot_id}\0{safety_snapshot_id}\0{timestamp}");
    let digest = sha256_hex(input.as_bytes());

    digest[..24].to_string()
}

fn parse_rfc3339(timestamp: &str) -> Result<OffsetDateTime, StorageError> {
    OffsetDateTime::parse(timestamp, &Rfc3339)
        .map_err(|error| StorageError::InvalidTimestamp(format!("{timestamp}: {error}")))
}

fn parse_iso_hour(hour: &str) -> Result<OffsetDateTime, StorageError> {
    PrimitiveDateTime::parse(
        hour,
        &time::macros::format_description!("[year]-[month]-[day]T[hour]"),
    )
    .map(|value| value.assume_utc())
    .map_err(|error| StorageError::InvalidTimestamp(format!("{hour}: {error}")))
}

fn format_timestamp(timestamp: OffsetDateTime) -> Result<String, StorageError> {
    timestamp
        .format(&Rfc3339)
        .map_err(|error| StorageError::InvalidTimestamp(error.to_string()))
}

fn validate_fixed_ten_minute_segment(
    from: OffsetDateTime,
    to: OffsetDateTime,
    from_timestamp: &str,
    to_timestamp: &str,
) -> Result<(), StorageError> {
    if to - from != Duration::minutes(10)
        || from.minute() % 10 != 0
        || from.second() != 0
        || to.second() != 0
    {
        return Err(StorageError::InvalidTimeWindow(format!(
            "expected a fixed 10-minute segment: {from_timestamp} .. {to_timestamp}"
        )));
    }

    Ok(())
}

fn start_of_hour(timestamp: OffsetDateTime) -> OffsetDateTime {
    timestamp
        .replace_minute(0)
        .expect("valid minute replacement")
        .replace_second(0)
        .expect("valid second replacement")
        .replace_millisecond(0)
        .expect("valid millisecond replacement")
        .replace_microsecond(0)
        .expect("valid microsecond replacement")
        .replace_nanosecond(0)
        .expect("valid nanosecond replacement")
}

fn format_iso_hour(timestamp: OffsetDateTime) -> Result<String, StorageError> {
    timestamp
        .format(&time::macros::format_description!(
            "[year]-[month]-[day]T[hour]"
        ))
        .map_err(|error| StorageError::InvalidTimestamp(error.to_string()))
}

fn format_date_directory(timestamp: OffsetDateTime) -> String {
    timestamp
        .format(&time::macros::format_description!("[year]-[month]-[day]"))
        .expect("date directory format must be valid")
}

fn format_hour_directory(timestamp: OffsetDateTime) -> String {
    timestamp
        .format(&time::macros::format_description!("[hour]"))
        .expect("hour directory format must be valid")
}

fn hour_directory_relative_path(hour_start: OffsetDateTime) -> PathBuf {
    PathBuf::from(format_date_directory(hour_start)).join(format_hour_directory(hour_start))
}

fn hour_markdown_relative_path(hour_start: OffsetDateTime) -> PathBuf {
    hour_directory_relative_path(hour_start).join("README.md")
}

fn segment_markdown_relative_path(from: OffsetDateTime) -> Result<PathBuf, StorageError> {
    let from_timestamp = format_timestamp(from)?;
    let label = segment_label(from.hour(), from.minute()).ok_or_else(|| {
        StorageError::InvalidTimeWindow(format!(
            "failed to derive 10-minute segment label for {}",
            from_timestamp
        ))
    })?;

    Ok(hour_directory_relative_path(start_of_hour(from)).join(format!("{label}.md")))
}

fn snapshot_markdown_file_name(
    timestamp: OffsetDateTime,
    relative_path: &Path,
    snapshot_id: &SnapshotId,
) -> String {
    let time_prefix = timestamp
        .format(&time::macros::format_description!(
            "[hour]-[minute]-[second]"
        ))
        .expect("snapshot filename time format must be valid");
    let sanitized_path = sanitize_relative_path_for_file_name(relative_path);

    format!(
        "{time_prefix}__{sanitized_path}__{}.md",
        short_id(snapshot_id.as_str())
    )
}

fn sanitize_relative_path_for_file_name(relative_path: &Path) -> String {
    let joined = relative_path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("_");

    let sanitized = joined
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => character,
            _ => '_',
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "snapshot".to_string()
    } else {
        sanitized
    }
}

fn render_snapshot_markdown_preview(snapshot: &SnapshotRecord, contents: &[u8]) -> String {
    if snapshot.captures_missing_file {
        return "[missing file state]".to_string();
    }

    if contents.is_empty() {
        return "<empty file>".to_string();
    }

    match std::str::from_utf8(contents) {
        Ok(text) => {
            let mut lines = text.lines();
            let mut preview = lines.by_ref().take(40).collect::<Vec<_>>().join("\n");

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

fn human_timestamp(raw: &str) -> String {
    OffsetDateTime::parse(raw, &Rfc3339)
        .ok()
        .and_then(|timestamp| {
            timestamp
                .to_offset(UtcOffset::UTC)
                .format(&time::macros::format_description!(
                    "[year]-[month]-[day] [hour]:[minute]:[second] UTC"
                ))
                .ok()
        })
        .unwrap_or_else(|| raw.to_string())
}

fn human_window_label(from: &str, to: &str) -> String {
    format!("{} -> {}", human_timestamp(from), human_timestamp(to))
}

fn human_hour_label(hour_start: OffsetDateTime) -> String {
    let start = hour_start.to_offset(UtcOffset::UTC);
    let end = (hour_start + Duration::hours(1)).to_offset(UtcOffset::UTC);

    format!(
        "{} -> {}",
        start
            .format(&time::macros::format_description!(
                "[year]-[month]-[day] [hour]:[minute] UTC"
            ))
            .expect("hour label start format must be valid"),
        end.format(&time::macros::format_description!("[hour]:[minute] UTC"))
            .expect("hour label end format must be valid")
    )
}

fn clear_directory_if_exists(path: &Path) -> Result<(), StorageError> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }

    Ok(())
}

fn remove_directory_if_empty(path: &Path) -> Result<(), StorageError> {
    if path.is_dir() && fs::read_dir(path)?.next().is_none() {
        fs::remove_dir(path)?;
    }

    Ok(())
}

fn relative_path_from_hour_dir(from_path: &Path, to_path: &Path) -> Result<PathBuf, StorageError> {
    let base = from_path
        .parent()
        .ok_or_else(|| StorageError::InvalidRelativePath(from_path.display().to_string()))?;

    let to_components = normalize_relative_path(to_path)?
        .components()
        .map(component_to_string)
        .collect::<Vec<_>>();
    let base_components = normalize_relative_path(base)?
        .components()
        .map(component_to_string)
        .collect::<Vec<_>>();

    let shared_len = base_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = PathBuf::new();

    for _ in shared_len..base_components.len() {
        relative.push("..");
    }

    for component in to_components.into_iter().skip(shared_len) {
        relative.push(component);
    }

    Ok(relative)
}

fn path_to_slash_string(path: impl AsRef<Path>) -> String {
    path.as_ref()
        .components()
        .map(component_to_string)
        .collect::<Vec<_>>()
        .join("/")
}

fn component_to_string(component: Component<'_>) -> String {
    component.as_os_str().to_string_lossy().into_owned()
}

fn short_id(value: &str) -> &str {
    &value[..std::cmp::min(value.len(), 8)]
}

fn ensure_schema_compatibility(connection: &Connection) -> Result<(), StorageError> {
    if !table_has_column(connection, "snapshots", "captures_missing_file")? {
        connection.execute(
            "ALTER TABLE snapshots
             ADD COLUMN captures_missing_file INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }

    if !table_has_column(connection, "restore_operations", "previous_file_existed")? {
        connection.execute(
            "ALTER TABLE restore_operations
             ADD COLUMN previous_file_existed INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }

    Ok(())
}

fn table_has_column(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, StorageError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let mut rows = statement.query([])?;

    while let Some(row) = rows.next()? {
        let current_name: String = row.get(1)?;

        if current_name == column_name {
            return Ok(true);
        }
    }

    Ok(false)
}

fn snapshot_row_mapper(row: &rusqlite::Row<'_>) -> Result<SnapshotRecord, rusqlite::Error> {
    let kind: String = row.get(6)?;

    Ok(SnapshotRecord {
        id: SnapshotId::new(row.get::<_, String>(0)?),
        project_id: ProjectId::new(row.get::<_, String>(1)?),
        relative_path: PathBuf::from(row.get::<_, String>(2)?),
        blob_hash: ContentHash::new(row.get::<_, String>(3)?),
        size_bytes: row.get::<_, i64>(4)? as u64,
        timestamp: row.get(5)?,
        kind: SnapshotKind::from_db_value(&kind),
        captures_missing_file: row.get(7)?,
    })
}

fn restore_operation_row_mapper(
    row: &rusqlite::Row<'_>,
) -> Result<RestoreOperationRecord, rusqlite::Error> {
    Ok(RestoreOperationRecord {
        id: row.get(0)?,
        project_id: ProjectId::new(row.get::<_, String>(1)?),
        relative_path: PathBuf::from(row.get::<_, String>(2)?),
        restored_snapshot_id: SnapshotId::new(row.get::<_, String>(3)?),
        safety_snapshot_id: SnapshotId::new(row.get::<_, String>(4)?),
        previous_file_existed: row.get(5)?,
        previous_content_hash: ContentHash::new(row.get::<_, String>(6)?),
        restored_content_hash: ContentHash::new(row.get::<_, String>(7)?),
        timestamp: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{LocalHistoryStore, SnapshotQuery, SnapshotWriteRequest, StorageError, SCHEMA_SQL};
    use crate::{RetentionPolicy, SnapshotId, SnapshotKind};
    use rusqlite::params;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn open_initializes_directories_and_schema() {
        let (base_dir, project_root) = create_test_roots("schema");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");

        assert!(store.layout().database_path.exists());
        assert!(store.layout().blobs_dir.exists());
        assert!(store.layout().view_dir.exists());
        assert!(store.layout().logs_dir.exists());
        assert!(SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS snapshots"));

        let table_count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN (
                    'projects',
                    'tracked_files',
                    'content_blobs',
                    'snapshots',
                    'restore_operations',
                    'generated_markdown_views'
                )",
                [],
                |row| row.get(0),
            )
            .expect("schema query must work");
        assert_eq!(table_count, 6);

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn stores_snapshot_metadata_and_reads_exact_content_back() {
        let (base_dir, project_root) = create_test_roots("snapshot");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let request = SnapshotWriteRequest {
            relative_path: PathBuf::from("src/lib.rs"),
            contents: b"fn example() { println!(\"hello\"); }\n".to_vec(),
            timestamp: "2026-05-02T14:14:28+02:00".to_string(),
            kind: SnapshotKind::Raw,
            is_binary: false,
            captures_missing_file: false,
        };

        let snapshot = store
            .store_snapshot(request)
            .expect("snapshot must be stored");
        let round_tripped = store
            .read_snapshot_content(&snapshot.id)
            .expect("snapshot contents must round-trip");

        assert_eq!(round_tripped, b"fn example() { println!(\"hello\"); }\n");

        let db_snapshot_id: String = store
            .connection
            .query_row(
                "SELECT id FROM snapshots WHERE id = ?1",
                params![snapshot.id.as_str()],
                |row| row.get(0),
            )
            .expect("snapshot metadata must exist");
        assert_eq!(db_snapshot_id, snapshot.id.as_str());

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn deduplicates_blobs_and_persists_compressed_content() {
        let (base_dir, project_root) = create_test_roots("dedup");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let contents = vec![b'a'; 4_096];

        let first = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/first.rs"),
                contents: contents.clone(),
                timestamp: "2026-05-02T14:14:28+02:00".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("first snapshot must store");
        let second = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/second.rs"),
                contents: contents.clone(),
                timestamp: "2026-05-02T14:15:28+02:00".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("second snapshot must store");

        assert_eq!(first.blob_hash, second.blob_hash);

        let blob_count: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM content_blobs", [], |row| row.get(0))
            .expect("blob count query must work");
        assert_eq!(blob_count, 1);

        let compressed_path: String = store
            .connection
            .query_row(
                "SELECT storage_path FROM content_blobs WHERE content_hash = ?1",
                params![first.blob_hash.as_str()],
                |row| row.get(0),
            )
            .expect("blob metadata must exist");
        let compressed_metadata =
            fs::metadata(store.layout().project_dir.join(compressed_path)).expect("blob file");
        assert!(compressed_metadata.len() < contents.len() as u64);

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn rejects_non_relative_paths() {
        let (base_dir, project_root) = create_test_roots("relative");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let result = store.store_snapshot(SnapshotWriteRequest {
            relative_path: PathBuf::from("../outside.txt"),
            contents: b"oops".to_vec(),
            timestamp: "2026-05-02T14:14:28+02:00".to_string(),
            kind: SnapshotKind::Safety,
            is_binary: false,
            captures_missing_file: false,
        });

        assert!(result.is_err());

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn missing_snapshot_returns_explicit_error() {
        let (base_dir, project_root) = create_test_roots("missing");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let result = store.read_snapshot_content(&SnapshotId::new("missing"));

        assert!(result.is_err());

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn recent_snapshots_are_returned_newest_first() {
        let (base_dir, project_root) = create_test_roots("recent");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");

        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/first.rs"),
                contents: b"first".to_vec(),
                timestamp: "2026-05-02T14:10:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("first snapshot must store");
        let newest = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/second.rs"),
                contents: b"second".to_vec(),
                timestamp: "2026-05-02T14:20:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("second snapshot must store");

        let recent = store
            .recent_snapshots(10)
            .expect("recent snapshot query must work");

        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, newest.id);

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn restore_snapshot_creates_safety_snapshot_and_records_operation() {
        let (base_dir, project_root) = create_test_roots("restore");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/restored.txt"),
                contents: b"restored from snapshot".to_vec(),
                timestamp: "2026-05-02T14:20:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("snapshot must store");
        let live_path = store.project().root.join("src/restored.txt");

        fs::create_dir_all(
            live_path
                .parent()
                .expect("restored test path parent must exist"),
        )
        .expect("parent dir must exist");
        fs::write(&live_path, b"current live state").expect("live file must exist");

        let outcome = store
            .restore_snapshot(&snapshot.id, "2026-05-02T14:21:00Z")
            .expect("restore must succeed");

        assert_eq!(outcome.restored_snapshot.id, snapshot.id);
        assert_eq!(outcome.safety_snapshot.kind, SnapshotKind::Safety);
        assert!(!outcome.safety_snapshot.captures_missing_file);
        assert!(outcome.operation.previous_file_existed);
        assert_eq!(
            fs::read_to_string(outcome.restored_path).expect("restored file must exist"),
            "restored from snapshot"
        );
        assert_eq!(
            store
                .read_snapshot_content(&outcome.safety_snapshot.id)
                .expect("safety snapshot must be readable"),
            b"current live state"
        );

        let latest_operation = store
            .latest_restore_operation()
            .expect("restore operation lookup must succeed")
            .expect("restore operation must exist");
        assert_eq!(latest_operation.id, outcome.operation.id);
        assert_eq!(
            latest_operation.safety_snapshot_id,
            outcome.safety_snapshot.id
        );

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn undo_last_restore_restores_previous_contents() {
        let (base_dir, project_root) = create_test_roots("undo");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/undo.txt"),
                contents: b"target state".to_vec(),
                timestamp: "2026-05-02T14:20:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("snapshot must store");
        let live_path = store.project().root.join("src/undo.txt");

        fs::create_dir_all(live_path.parent().expect("undo parent dir must exist"))
            .expect("parent dir must exist");
        fs::write(&live_path, b"before restore").expect("live file must exist");
        store
            .restore_snapshot(&snapshot.id, "2026-05-02T14:21:00Z")
            .expect("initial restore must succeed");

        let undo_outcome = store
            .undo_last_restore("2026-05-02T14:22:00Z")
            .expect("undo restore must succeed");

        assert_eq!(undo_outcome.restored_snapshot.kind, SnapshotKind::Safety);
        assert_eq!(
            fs::read_to_string(&live_path).expect("live file must be restored"),
            "before restore"
        );

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn repeated_restore_chain_remains_recoverable() {
        let (base_dir, project_root) = create_test_roots("restore-chain");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let first_target = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/chain.txt"),
                contents: b"first target".to_vec(),
                timestamp: "2026-05-02T14:20:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("first target snapshot must store");
        let second_target = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/chain.txt"),
                contents: b"second target".to_vec(),
                timestamp: "2026-05-02T14:20:30Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("second target snapshot must store");
        let live_path = store.project().root.join("src/chain.txt");

        fs::create_dir_all(live_path.parent().expect("restore-chain parent must exist"))
            .expect("parent dir must exist");
        fs::write(&live_path, b"before first restore").expect("live file must exist");

        store
            .restore_snapshot(&first_target.id, "2026-05-02T14:21:00Z")
            .expect("first restore must succeed");
        assert_eq!(
            fs::read_to_string(&live_path).expect("file must reflect first restore"),
            "first target"
        );

        let second_restore = store
            .restore_snapshot(&second_target.id, "2026-05-02T14:22:00Z")
            .expect("second restore must succeed");
        assert_eq!(
            fs::read_to_string(&live_path).expect("file must reflect second restore"),
            "second target"
        );
        assert_eq!(
            store
                .read_snapshot_content(&second_restore.safety_snapshot.id)
                .expect("second restore safety snapshot must be readable"),
            b"first target"
        );

        let undo_outcome = store
            .undo_last_restore("2026-05-02T14:23:00Z")
            .expect("undo after second restore must succeed");
        assert_eq!(undo_outcome.restored_snapshot.kind, SnapshotKind::Safety);
        assert_eq!(
            undo_outcome.restored_snapshot.id,
            second_restore.safety_snapshot.id
        );
        assert_eq!(
            fs::read_to_string(&live_path).expect("file must return to first restore state"),
            "first target"
        );

        let redo_outcome = store
            .restore_last_safety_snapshot("2026-05-02T14:24:00Z")
            .expect("redo through latest safety snapshot must succeed");
        assert_eq!(redo_outcome.restored_snapshot.kind, SnapshotKind::Safety);
        assert_eq!(
            fs::read_to_string(&live_path).expect("file must return to second restore state"),
            "second target"
        );

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn restore_last_safety_snapshot_handles_missing_file_state() {
        let (base_dir, project_root) = create_test_roots("missing-safety");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/deleted.txt"),
                contents: b"restored text".to_vec(),
                timestamp: "2026-05-02T14:20:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("snapshot must store");
        let live_path = store.project().root.join("src/deleted.txt");

        store
            .restore_snapshot(&snapshot.id, "2026-05-02T14:21:00Z")
            .expect("restore must succeed");

        let safety_snapshot = store
            .safety_snapshots(1)
            .expect("safety list must work")
            .into_iter()
            .next()
            .expect("safety snapshot must exist");
        assert!(safety_snapshot.captures_missing_file);
        assert!(live_path.exists());

        store
            .restore_last_safety_snapshot("2026-05-02T14:22:00Z")
            .expect("restore last safety must succeed");

        assert!(!live_path.exists());

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn recent_raw_snapshots_exclude_safety_snapshots() {
        let (base_dir, project_root) = create_test_roots("recent-raw");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let raw_snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/visible.txt"),
                contents: b"raw".to_vec(),
                timestamp: "2026-05-02T14:20:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("raw snapshot must store");
        let live_path = store.project().root.join("src/visible.txt");

        fs::create_dir_all(live_path.parent().expect("recent-raw parent must exist"))
            .expect("parent dir must exist");
        fs::write(&live_path, b"current").expect("live file must exist");
        store
            .restore_snapshot(&raw_snapshot.id, "2026-05-02T14:21:00Z")
            .expect("restore must succeed");

        let raw_recent = store
            .recent_raw_snapshots(10)
            .expect("raw recent query must succeed");
        let safety_recent = store
            .safety_snapshots(10)
            .expect("safety recent query must succeed");

        assert_eq!(raw_recent.len(), 1);
        assert_eq!(raw_recent[0].id, raw_snapshot.id);
        assert_eq!(safety_recent.len(), 1);
        assert_eq!(safety_recent[0].kind, SnapshotKind::Safety);

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn query_snapshots_supports_pagination_and_file_filter() {
        let (base_dir, project_root) = create_test_roots("query-file");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");

        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/alpha.rs"),
                contents: b"alpha-1".to_vec(),
                timestamp: "2026-05-02T14:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("first snapshot must store");
        let latest_alpha = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/alpha.rs"),
                contents: b"alpha-2".to_vec(),
                timestamp: "2026-05-02T14:20:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("second snapshot must store");
        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/beta.rs"),
                contents: b"beta".to_vec(),
                timestamp: "2026-05-02T14:30:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("third snapshot must store");

        let page = store
            .query_snapshots(&SnapshotQuery {
                relative_path: Some(PathBuf::from("src/alpha.rs")),
                kind: Some(SnapshotKind::Raw),
                page: 1,
                page_size: 1,
                ..SnapshotQuery::default()
            })
            .expect("filtered query must succeed");

        assert_eq!(page.total_items, 2);
        assert_eq!(page.total_pages, 2);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, latest_alpha.id);

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn query_snapshots_supports_time_range_filtering() {
        let (base_dir, project_root) = create_test_roots("query-time");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");

        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/early.rs"),
                contents: b"early".to_vec(),
                timestamp: "2026-05-02T14:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("early snapshot must store");
        let matching_snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/mid.rs"),
                contents: b"mid".to_vec(),
                timestamp: "2026-05-02T14:30:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("mid snapshot must store");
        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/late.rs"),
                contents: b"late".to_vec(),
                timestamp: "2026-05-02T15:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("late snapshot must store");

        let page = store
            .query_snapshots(&SnapshotQuery {
                kind: Some(SnapshotKind::Raw),
                from_timestamp: Some("2026-05-02T14:15:00Z".to_string()),
                to_timestamp: Some("2026-05-02T14:45:00Z".to_string()),
                ..SnapshotQuery::default()
            })
            .expect("time-range query must succeed");

        assert_eq!(page.total_items, 1);
        assert_eq!(page.items[0].id, matching_snapshot.id);

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn history_for_segment_groups_snapshots_by_file() {
        let (base_dir, project_root) = create_test_roots("segment-history");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");

        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/alpha.rs"),
                contents: b"alpha-older".to_vec(),
                timestamp: "2026-05-02T14:12:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("first alpha snapshot must store");
        let latest_alpha = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/alpha.rs"),
                contents: b"alpha-newer".to_vec(),
                timestamp: "2026-05-02T14:15:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("second alpha snapshot must store");
        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/beta.rs"),
                contents: b"beta".to_vec(),
                timestamp: "2026-05-02T14:18:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("beta snapshot must store");

        let history = store
            .history_for_segment("2026-05-02T14:10:00Z", "2026-05-02T14:20:00Z")
            .expect("segment history must build");

        assert_eq!(history.segment.label, "14-10__14-20");
        assert_eq!(history.affected_files.len(), 2);
        assert_eq!(
            history.affected_files[0].relative_path,
            PathBuf::from("src/alpha.rs")
        );
        assert_eq!(history.affected_files[0].snapshot_count, 2);
        assert_eq!(history.affected_files[0].snapshots[0].id, latest_alpha.id);

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn history_for_hour_creates_six_fixed_segments() {
        let (base_dir, project_root) = create_test_roots("hour-history");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");

        let first_segment = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/first.rs"),
                contents: b"first".to_vec(),
                timestamp: "2026-05-02T14:05:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("first segment snapshot must store");
        let fourth_segment = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/fourth.rs"),
                contents: b"fourth".to_vec(),
                timestamp: "2026-05-02T14:33:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("fourth segment snapshot must store");

        let history = store
            .history_for_hour("2026-05-02T14")
            .expect("hour history must build");

        assert_eq!(history.hour.from, "2026-05-02T14:00:00Z");
        assert_eq!(history.hour.to, "2026-05-02T15:00:00Z");
        assert_eq!(history.segments.len(), 6);
        assert_eq!(history.segments[0].segment.label, "14-00__14-10");
        assert_eq!(history.segments[3].segment.label, "14-30__14-40");
        assert_eq!(
            history.segments[0].affected_files[0].snapshots[0].id,
            first_segment.id
        );
        assert_eq!(
            history.segments[3].affected_files[0].snapshots[0].id,
            fourth_segment.id
        );
        assert!(history.segments[1].affected_files.is_empty());

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn render_hour_markdown_writes_filesystem_browsable_view() {
        let (base_dir, project_root) = create_test_roots("render-hour");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/alpha.rs"),
                contents: b"fn alpha() {\n    println!(\"alpha\");\n}\n".to_vec(),
                timestamp: "2026-05-02T14:15:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("snapshot must store");

        let hour_entry = store
            .render_hour_markdown("2026-05-02T14", "2026-05-02T14:59:00Z")
            .expect("hour markdown render must succeed");
        let hour_readme_path = store
            .layout()
            .view_dir
            .join(&hour_entry.relative_markdown_path);
        let hour_readme =
            fs::read_to_string(hour_readme_path).expect("hour README must be readable");
        let root_index =
            fs::read_to_string(store.layout().view_dir.join("README.md")).expect("root index");
        let segment_markdown = fs::read_to_string(
            store
                .layout()
                .view_dir
                .join("2026-05-02")
                .join("14")
                .join("14-10__14-20.md"),
        )
        .expect("segment markdown must exist");
        let snapshot_dir = store
            .layout()
            .view_dir
            .join("2026-05-02")
            .join("14")
            .join("snapshots");
        let snapshot_pages = fs::read_dir(snapshot_dir)
            .expect("snapshot dir must exist")
            .collect::<Result<Vec<_>, _>>()
            .expect("snapshot dir entries must read");
        let snapshot_page = fs::read_to_string(snapshot_pages[0].path())
            .expect("snapshot markdown page must be readable");
        let generated_entries: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM generated_markdown_views WHERE project_id = ?1",
                params![store.project().id.as_str()],
                |row| row.get(0),
            )
            .expect("generated markdown entry count must work");

        assert_eq!(
            hour_entry.relative_markdown_path,
            PathBuf::from("2026-05-02/14/README.md")
        );
        assert!(hour_readme.contains("## Segments"));
        assert!(root_index.contains("./2026-05-02/14/README.md"));
        assert!(segment_markdown.contains(&format!(
            "cargo run -p local-history-cli -- restore {}",
            snapshot.id.as_str()
        )));
        assert!(snapshot_page.contains(snapshot.id.as_str()));
        assert!(snapshot_page.contains("println!(\"alpha\");"));
        assert!(generated_entries >= 8);

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn rebuild_markdown_view_restores_deleted_view_tree() {
        let (base_dir, project_root) = create_test_roots("rebuild-view");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");

        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/alpha.rs"),
                contents: b"alpha".to_vec(),
                timestamp: "2026-05-02T14:05:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("first snapshot must store");
        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/beta.rs"),
                contents: b"beta".to_vec(),
                timestamp: "2026-05-02T15:05:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("second snapshot must store");

        let first_entries = store
            .rebuild_markdown_view("2026-05-02T15:10:00Z")
            .expect("first rebuild must succeed");
        assert!(store.layout().view_dir.join("README.md").exists());

        fs::remove_dir_all(&store.layout().view_dir).expect("view dir removal must succeed");

        let second_entries = store
            .rebuild_markdown_view("2026-05-02T15:11:00Z")
            .expect("second rebuild must succeed");
        let root_index =
            fs::read_to_string(store.layout().view_dir.join("README.md")).expect("root index");

        assert!(!first_entries.is_empty());
        assert!(!second_entries.is_empty());
        assert!(root_index.contains("./2026-05-02/15/README.md"));
        assert!(root_index.contains("./2026-05-02/14/README.md"));

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn render_segment_markdown_requires_fixed_ten_minute_window() {
        let (base_dir, project_root) = create_test_roots("render-segment-window");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");

        let error = store
            .render_segment_markdown(
                "2026-05-02T14:12:00Z",
                "2026-05-02T14:22:00Z",
                "2026-05-02T14:22:30Z",
            )
            .expect_err("misaligned segment render must fail");

        assert!(matches!(error, StorageError::InvalidTimeWindow(_)));

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn can_locate_store_from_snapshot_id() {
        let (base_dir, project_root) = create_test_roots("locate");
        let store = LocalHistoryStore::open(&base_dir, &project_root).expect("store must open");
        let snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/found.rs"),
                contents: b"located".to_vec(),
                timestamp: "2026-05-02T14:25:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("snapshot must store");

        let located = LocalHistoryStore::open_for_snapshot_id(&base_dir, &snapshot.id)
            .expect("lookup must work")
            .expect("store must be found");

        assert_eq!(
            located.project().root,
            super::normalize_project_root(&project_root)
        );

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn rejects_snapshot_larger_than_retention_limit() {
        let (base_dir, project_root) = create_test_roots("large-file");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let oversized = vec![b'x'; (RetentionPolicy::default().max_file_size_bytes + 1) as usize];

        let error = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/huge.bin"),
                contents: oversized,
                timestamp: "2026-05-03T09:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: true,
                captures_missing_file: false,
            })
            .expect_err("oversized snapshot must fail");

        assert!(matches!(error, StorageError::SnapshotTooLarge { .. }));

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn prune_drops_oldest_snapshots_per_file_and_rebuilds_view() {
        let (base_dir, project_root) = create_test_roots("prune-count");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");

        let oldest = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/count.rs"),
                contents: b"oldest".to_vec(),
                timestamp: "2026-05-01T10:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("oldest snapshot must store");
        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/count.rs"),
                contents: b"middle".to_vec(),
                timestamp: "2026-05-02T10:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("middle snapshot must store");
        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/count.rs"),
                contents: b"newest".to_vec(),
                timestamp: "2026-05-03T10:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("newest snapshot must store");

        let report = store
            .prune(
                &RetentionPolicy {
                    max_snapshots_per_file: 2,
                    max_project_storage_bytes: u64::MAX,
                    max_file_size_bytes: RetentionPolicy::default().max_file_size_bytes,
                    max_snapshot_age_days: u16::MAX,
                },
                "2026-05-03T10:05:00Z",
            )
            .expect("prune must succeed");
        let remaining = store
            .recent_raw_snapshots(10)
            .expect("remaining snapshots must load");

        assert_eq!(report.deleted_snapshot_count, 1);
        assert_eq!(report.pruned_for_file_count, 1);
        assert_eq!(report.remaining_snapshot_count, 2);
        assert_eq!(report.deleted_blob_count, 1);
        assert!(store
            .snapshot(&oldest.id)
            .expect("lookup must work")
            .is_none());
        assert!(store.layout().view_dir.join("README.md").exists());
        assert_eq!(remaining.len(), 2);

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn prune_respects_restore_referenced_snapshots() {
        let (base_dir, project_root) = create_test_roots("prune-protected");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let snapshot = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/protected.txt"),
                contents: b"restore target".to_vec(),
                timestamp: "2026-05-02T14:20:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("snapshot must store");
        let live_path = store.project().root.join("src/protected.txt");

        fs::create_dir_all(live_path.parent().expect("parent must exist"))
            .expect("parent dir must exist");
        fs::write(&live_path, b"current state").expect("live file must exist");
        store
            .restore_snapshot(&snapshot.id, "2026-05-02T14:21:00Z")
            .expect("restore must succeed");

        let report = store
            .prune(
                &RetentionPolicy {
                    max_snapshots_per_file: 0,
                    max_project_storage_bytes: u64::MAX,
                    max_file_size_bytes: RetentionPolicy::default().max_file_size_bytes,
                    max_snapshot_age_days: u16::MAX,
                },
                "2026-05-03T10:00:00Z",
            )
            .expect("prune must succeed");

        assert_eq!(report.protected_snapshot_count, 2);
        assert_eq!(report.remaining_snapshot_count, 2);
        assert_eq!(report.deleted_snapshot_count, 0);

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn prune_drops_stale_restore_operations_and_only_keeps_latest_chain() {
        let (base_dir, project_root) = create_test_roots("prune-restore-chain");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");
        let live_path = store.project().root.join("src/chain.txt");

        fs::create_dir_all(live_path.parent().expect("parent must exist"))
            .expect("parent dir must exist");

        let first_target = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/chain.txt"),
                contents: b"first-target".to_vec(),
                timestamp: "2026-05-01T10:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("first target snapshot must store");
        fs::write(&live_path, b"live-before-first").expect("live file must exist");
        let first_restore = store
            .restore_snapshot(&first_target.id, "2026-05-01T10:10:00Z")
            .expect("first restore must succeed");

        let second_target = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/chain.txt"),
                contents: b"second-target".to_vec(),
                timestamp: "2026-05-02T10:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("second target snapshot must store");
        fs::write(&live_path, b"live-before-second").expect("live file must update");
        let second_restore = store
            .restore_snapshot(&second_target.id, "2026-05-02T10:10:00Z")
            .expect("second restore must succeed");

        let report = store
            .prune(
                &RetentionPolicy {
                    max_snapshots_per_file: 0,
                    max_project_storage_bytes: u64::MAX,
                    max_file_size_bytes: RetentionPolicy::default().max_file_size_bytes,
                    max_snapshot_age_days: u16::MAX,
                },
                "2026-05-03T10:00:00Z",
            )
            .expect("prune must succeed");
        let latest_operation = store
            .latest_restore_operation()
            .expect("latest restore operation lookup must succeed")
            .expect("latest restore operation must remain");
        let restore_operation_count: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM restore_operations", [], |row| {
                row.get(0)
            })
            .expect("restore operation count query must work");

        assert_eq!(report.deleted_restore_operation_count, 1);
        assert_eq!(report.protected_snapshot_count, 2);
        assert_eq!(restore_operation_count, 1);
        assert_eq!(latest_operation.id, second_restore.operation.id);
        assert!(
            store
                .snapshot(&first_target.id)
                .expect("lookup must work")
                .is_none(),
            "older restored snapshot should no longer be pinned"
        );
        assert!(
            store
                .snapshot(&first_restore.safety_snapshot.id)
                .expect("lookup must work")
                .is_none(),
            "older safety snapshot should no longer be pinned"
        );
        assert!(
            store
                .snapshot(&second_target.id)
                .expect("lookup must work")
                .is_some(),
            "latest restored snapshot must stay available for undo"
        );
        assert!(
            store
                .snapshot(&second_restore.safety_snapshot.id)
                .expect("lookup must work")
                .is_some(),
            "latest safety snapshot must stay available for undo"
        );

        cleanup_test_roots(&base_dir);
    }

    #[test]
    fn prune_reduces_estimated_storage_to_budget() {
        let (base_dir, project_root) = create_test_roots("prune-storage");
        let store = LocalHistoryStore::open(&base_dir, project_root).expect("store must open");

        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/history.txt"),
                contents: b"alpha-storage".to_vec(),
                timestamp: "2026-05-01T10:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("first snapshot must store");
        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/history.txt"),
                contents: b"beta-storage!".to_vec(),
                timestamp: "2026-05-02T10:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("second snapshot must store");
        store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/history.txt"),
                contents: b"gamma-storage".to_vec(),
                timestamp: "2026-05-03T10:00:00Z".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
                captures_missing_file: false,
            })
            .expect("third snapshot must store");

        let report = store
            .prune(
                &RetentionPolicy {
                    max_snapshots_per_file: usize::MAX,
                    max_project_storage_bytes: 20,
                    max_file_size_bytes: RetentionPolicy::default().max_file_size_bytes,
                    max_snapshot_age_days: u16::MAX,
                },
                "2026-05-03T10:05:00Z",
            )
            .expect("prune must succeed");

        assert!(report.pruned_for_storage_count >= 1);
        assert!(report.remaining_referenced_blob_bytes <= 20);

        cleanup_test_roots(&base_dir);
    }

    fn create_test_roots(label: &str) -> (PathBuf, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be after UNIX_EPOCH")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("zed-local-history-storage-{label}-{unique}"));
        let base_dir = root.join("data");
        let project_root = root.join("project");

        fs::create_dir_all(&base_dir).expect("base dir must exist");
        fs::create_dir_all(&project_root).expect("project dir must exist");

        (base_dir, project_root)
    }

    fn cleanup_test_roots(base_dir: &Path) {
        let root = base_dir
            .parent()
            .expect("test root parent must exist")
            .to_path_buf();

        if root.exists() {
            fs::remove_dir_all(root).expect("root dir must be removed");
        }
    }
}
