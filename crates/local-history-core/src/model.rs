use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ProjectId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnapshotId(String);

impl SnapshotId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: ProjectId,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedFileRecord {
    pub project_id: ProjectId,
    pub relative_path: PathBuf,
    pub current_content_hash: ContentHash,
    pub size_bytes: u64,
    pub is_binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotKind {
    Raw,
    Safety,
}

impl SnapshotKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Safety => "safety",
        }
    }

    pub fn from_db_value(value: &str) -> Self {
        match value {
            "safety" => Self::Safety,
            _ => Self::Raw,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub id: SnapshotId,
    pub project_id: ProjectId,
    pub relative_path: PathBuf,
    pub blob_hash: ContentHash,
    pub size_bytes: u64,
    pub timestamp: String,
    pub kind: SnapshotKind,
    pub captures_missing_file: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompressionKind {
    Zstd,
}

impl CompressionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zstd => "zstd",
        }
    }

    pub fn from_db_value(_value: &str) -> Self {
        Self::Zstd
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentBlobRecord {
    pub hash: ContentHash,
    pub size_bytes: u64,
    pub compression: CompressionKind,
    pub storage_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOperationRecord {
    pub id: String,
    pub project_id: ProjectId,
    pub relative_path: PathBuf,
    pub restored_snapshot_id: SnapshotId,
    pub safety_snapshot_id: SnapshotId,
    pub previous_file_existed: bool,
    pub previous_content_hash: ContentHash,
    pub restored_content_hash: ContentHash,
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub restored_snapshot: SnapshotRecord,
    pub safety_snapshot: SnapshotRecord,
    pub operation: RestoreOperationRecord,
    pub restored_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourBucket {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeSegment {
    pub label: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowedFileHistory {
    pub relative_path: PathBuf,
    pub snapshot_count: usize,
    pub snapshots: Vec<SnapshotRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentHistory {
    pub segment: TimeSegment,
    pub affected_files: Vec<WindowedFileHistory>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourHistory {
    pub hour: HourBucket,
    pub segments: Vec<SegmentHistory>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedMarkdownViewEntry {
    pub relative_markdown_path: PathBuf,
    pub title: String,
    pub generated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub max_snapshots_per_file: usize,
    pub max_project_storage_bytes: u64,
    pub max_file_size_bytes: u64,
    pub max_snapshot_age_days: u16,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_snapshots_per_file: 250,
            max_project_storage_bytes: 512 * 1024 * 1024,
            max_file_size_bytes: 4 * 1024 * 1024,
            max_snapshot_age_days: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneReport {
    pub pruned_at: String,
    pub deleted_restore_operation_count: usize,
    pub deleted_snapshot_count: usize,
    pub deleted_blob_count: usize,
    pub deleted_blob_bytes: u64,
    pub remaining_snapshot_count: usize,
    pub remaining_referenced_blob_bytes: u64,
    pub protected_snapshot_count: usize,
    pub pruned_for_age_count: usize,
    pub pruned_for_file_count: usize,
    pub pruned_for_storage_count: usize,
    pub rebuilt_markdown_view: bool,
}

pub fn segment_label(hour: u8, minute: u8) -> Option<String> {
    if hour > 23 || minute > 59 {
        return None;
    }

    let start_minute = (minute / 10) * 10;
    let mut end_hour = hour;
    let mut end_minute = start_minute + 10;

    if end_minute == 60 {
        end_hour = (hour + 1) % 24;
        end_minute = 0;
    }

    Some(format!(
        "{hour:02}-{start_minute:02}__{end_hour:02}-{end_minute:02}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        segment_label, CompressionKind, ContentBlobRecord, ContentHash, GeneratedMarkdownViewEntry,
        HourBucket, HourHistory, ProjectId, ProjectRecord, PruneReport, RestoreOperationRecord,
        RestoreOutcome, RetentionPolicy, SegmentHistory, SnapshotId, SnapshotKind, SnapshotRecord,
        TimeSegment, TrackedFileRecord, WindowedFileHistory,
    };
    use std::path::PathBuf;

    #[test]
    fn maps_minutes_to_fixed_ten_minute_segments() {
        assert_eq!(segment_label(14, 0).as_deref(), Some("14-00__14-10"));
        assert_eq!(segment_label(14, 14).as_deref(), Some("14-10__14-20"));
        assert_eq!(segment_label(14, 59).as_deref(), Some("14-50__15-00"));
        assert!(segment_label(24, 0).is_none());
    }

    #[test]
    fn core_entities_capture_stage_three_storage_shape() {
        let project_id = ProjectId::new("project-id");
        let content_hash = ContentHash::new("hash");
        let raw_snapshot_id = SnapshotId::new("raw-1");
        let safety_snapshot_id = SnapshotId::new("safety-1");
        let retention_policy = RetentionPolicy::default();

        let project = ProjectRecord {
            id: project_id.clone(),
            root: PathBuf::from("/workspace/demo"),
        };
        let tracked_file = TrackedFileRecord {
            project_id: project_id.clone(),
            relative_path: PathBuf::from("src/lib.rs"),
            current_content_hash: content_hash.clone(),
            size_bytes: 128,
            is_binary: false,
        };
        let snapshot = SnapshotRecord {
            id: raw_snapshot_id.clone(),
            project_id: project_id.clone(),
            relative_path: tracked_file.relative_path.clone(),
            blob_hash: content_hash.clone(),
            size_bytes: tracked_file.size_bytes,
            timestamp: "2026-05-02T14:14:28+02:00".to_string(),
            kind: SnapshotKind::Raw,
            captures_missing_file: false,
        };
        let blob = ContentBlobRecord {
            hash: content_hash.clone(),
            size_bytes: 128,
            compression: CompressionKind::Zstd,
            storage_path: PathBuf::from("blobs/ab/hash.zst"),
        };
        let restore_operation = RestoreOperationRecord {
            id: "restore-1".to_string(),
            project_id: project_id.clone(),
            relative_path: tracked_file.relative_path.clone(),
            restored_snapshot_id: raw_snapshot_id,
            safety_snapshot_id,
            previous_file_existed: true,
            previous_content_hash: ContentHash::new("before"),
            restored_content_hash: content_hash.clone(),
            timestamp: "2026-05-02T14:20:00+02:00".to_string(),
        };
        let restore_outcome = RestoreOutcome {
            restored_snapshot: snapshot.clone(),
            safety_snapshot: SnapshotRecord {
                id: SnapshotId::new("safety-2"),
                project_id: project_id.clone(),
                relative_path: tracked_file.relative_path.clone(),
                blob_hash: ContentHash::new("before"),
                size_bytes: 42,
                timestamp: "2026-05-02T14:19:59+02:00".to_string(),
                kind: SnapshotKind::Safety,
                captures_missing_file: false,
            },
            operation: restore_operation.clone(),
            restored_path: PathBuf::from("/workspace/demo/src/lib.rs"),
        };
        let hour_bucket = HourBucket {
            from: "2026-05-02T14:00:00+02:00".to_string(),
            to: "2026-05-02T15:00:00+02:00".to_string(),
        };
        let segment = TimeSegment {
            label: segment_label(14, 14).expect("segment label must exist"),
            from: "2026-05-02T14:10:00+02:00".to_string(),
            to: "2026-05-02T14:20:00+02:00".to_string(),
        };
        let file_history = WindowedFileHistory {
            relative_path: tracked_file.relative_path.clone(),
            snapshot_count: 1,
            snapshots: vec![snapshot.clone()],
        };
        let segment_history = SegmentHistory {
            segment: segment.clone(),
            affected_files: vec![file_history.clone()],
        };
        let hour_history = HourHistory {
            hour: hour_bucket.clone(),
            segments: vec![segment_history.clone()],
        };
        let view_entry = GeneratedMarkdownViewEntry {
            relative_markdown_path: PathBuf::from("2026-05-02/14/14-10__14-20.md"),
            title: "14:10-14:20".to_string(),
            generated_at: "2026-05-02T14:20:00+02:00".to_string(),
        };
        let prune_report = PruneReport {
            pruned_at: "2026-05-03T09:00:00Z".to_string(),
            deleted_restore_operation_count: 1,
            deleted_snapshot_count: 1,
            deleted_blob_count: 1,
            deleted_blob_bytes: 64,
            remaining_snapshot_count: 2,
            remaining_referenced_blob_bytes: 128,
            protected_snapshot_count: 1,
            pruned_for_age_count: 1,
            pruned_for_file_count: 0,
            pruned_for_storage_count: 0,
            rebuilt_markdown_view: true,
        };

        assert_eq!(project.id.as_str(), "project-id");
        assert_eq!(project.root, PathBuf::from("/workspace/demo"));
        assert_eq!(tracked_file.current_content_hash.as_str(), "hash");
        assert_eq!(snapshot.kind, SnapshotKind::Raw);
        assert!(!snapshot.captures_missing_file);
        assert_eq!(blob.compression, CompressionKind::Zstd);
        assert_eq!(restore_operation.relative_path, PathBuf::from("src/lib.rs"));
        assert!(restore_operation.previous_file_existed);
        assert_eq!(restore_outcome.operation.id, "restore-1");
        assert_eq!(file_history.snapshot_count, 1);
        assert_eq!(segment_history.affected_files.len(), 1);
        assert_eq!(hour_history.segments.len(), 1);
        assert_eq!(hour_bucket.to, "2026-05-02T15:00:00+02:00");
        assert_eq!(segment.label, "14-10__14-20");
        assert_eq!(
            view_entry.relative_markdown_path,
            PathBuf::from("2026-05-02/14/14-10__14-20.md")
        );
        assert_eq!(retention_policy.max_snapshots_per_file, 250);
        assert_eq!(
            retention_policy.max_project_storage_bytes,
            512 * 1024 * 1024
        );
        assert_eq!(retention_policy.max_file_size_bytes, 4 * 1024 * 1024);
        assert_eq!(retention_policy.max_snapshot_age_days, 30);
        assert_eq!(prune_report.deleted_restore_operation_count, 1);
        assert_eq!(prune_report.deleted_snapshot_count, 1);
        assert!(prune_report.rebuilt_markdown_view);
    }
}
