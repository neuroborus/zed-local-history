use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use rusqlite::{named_params, params, Connection, OptionalExtension};

use crate::error::StorageError;
use crate::hashing::sha256_hex;
use crate::identity::{normalize_project_root, project_id_for_root};
use crate::layout::{default_data_dir, StorageLayout};
use crate::model::{
    CompressionKind, ContentBlobRecord, ContentHash, ProjectId, ProjectRecord,
    RestoreOperationRecord, RestoreOutcome, SnapshotId, SnapshotKind, SnapshotRecord,
    TrackedFileRecord,
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

    pub fn store_snapshot(
        &self,
        request: SnapshotWriteRequest,
    ) -> Result<SnapshotRecord, StorageError> {
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
            size_bytes: request.contents.len() as u64,
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
    use super::{LocalHistoryStore, SnapshotQuery, SnapshotWriteRequest, SCHEMA_SQL};
    use crate::{SnapshotId, SnapshotKind};
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
