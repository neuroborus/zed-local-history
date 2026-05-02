use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::StorageError;
use crate::hashing::sha256_hex;
use crate::identity::{normalize_project_root, project_id_for_root};
use crate::layout::{default_data_dir, StorageLayout};
use crate::model::{
    CompressionKind, ContentBlobRecord, ContentHash, ProjectId, ProjectRecord, SnapshotId,
    SnapshotKind, SnapshotRecord, TrackedFileRecord,
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
    FOREIGN KEY (project_id) REFERENCES projects(id),
    FOREIGN KEY (blob_hash) REFERENCES content_blobs(content_hash)
);

CREATE TABLE IF NOT EXISTS restore_operations (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    restored_snapshot_id TEXT NOT NULL,
    safety_snapshot_id TEXT NOT NULL,
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
}

#[derive(Debug)]
pub struct LocalHistoryStore {
    connection: Connection,
    layout: StorageLayout,
    project: ProjectRecord,
}

impl LocalHistoryStore {
    pub fn open_default(project_root: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::open(default_data_dir(), project_root)
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

    pub fn project(&self) -> &ProjectRecord {
        &self.project
    }

    pub fn layout(&self) -> &StorageLayout {
        &self.layout
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
        };
        let tracked_file = TrackedFileRecord {
            project_id: self.project.id.clone(),
            relative_path,
            current_content_hash: blob.hash,
            size_bytes: snapshot.size_bytes,
            is_binary: request.is_binary,
        };

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

        self.connection.execute(
            "INSERT INTO snapshots (
                id,
                project_id,
                relative_path,
                blob_hash,
                size_bytes,
                timestamp,
                kind
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                snapshot.id.as_str(),
                snapshot.project_id.as_str(),
                snapshot.relative_path.to_string_lossy().as_ref(),
                snapshot.blob_hash.as_str(),
                snapshot.size_bytes as i64,
                snapshot.timestamp.as_str(),
                snapshot.kind.as_str(),
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

    fn get_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<Option<SnapshotRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id, project_id, relative_path, blob_hash, size_bytes, timestamp, kind
                 FROM snapshots
                 WHERE id = ?1",
                params![snapshot_id.as_str()],
                |row| {
                    let kind: String = row.get(6)?;

                    Ok(SnapshotRecord {
                        id: SnapshotId::new(row.get::<_, String>(0)?),
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        relative_path: PathBuf::from(row.get::<_, String>(2)?),
                        blob_hash: ContentHash::new(row.get::<_, String>(3)?),
                        size_bytes: row.get::<_, i64>(4)? as u64,
                        timestamp: row.get(5)?,
                        kind: SnapshotKind::from_db_value(&kind),
                    })
                },
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

#[cfg(test)]
mod tests {
    use super::{LocalHistoryStore, SnapshotWriteRequest, SCHEMA_SQL};
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
            })
            .expect("first snapshot must store");
        let second = store
            .store_snapshot(SnapshotWriteRequest {
                relative_path: PathBuf::from("src/second.rs"),
                contents: contents.clone(),
                timestamp: "2026-05-02T14:15:28+02:00".to_string(),
                kind: SnapshotKind::Raw,
                is_binary: false,
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
