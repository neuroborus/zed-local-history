mod diff;
mod error;
mod hashing;
mod identity;
mod ignore;
mod layout;
mod model;
mod storage;

pub use diff::{snapshot_to_current_unified_diff, SnapshotDiffError};
pub use error::StorageError;
pub use identity::{
    machine_salt, normalize_project_root, project_id_for_root, project_id_from_root_and_salt,
};
pub use ignore::{
    matches_default_ignored_path, IgnorePolicy, DEFAULT_IGNORED_PATTERNS, LOCAL_HISTORY_IGNORE_FILE,
};
pub use layout::{default_data_dir, StorageLayout};
pub use model::{
    segment_label, CompressionKind, ContentBlobRecord, ContentHash, GeneratedMarkdownViewEntry,
    HourBucket, HourHistory, ProjectId, ProjectRecord, PruneReport, RestoreOperationRecord,
    RestoreOutcome, RetentionPolicy, SegmentHistory, SnapshotId, SnapshotKind, SnapshotRecord,
    TimeSegment, TrackedFileRecord, WindowedFileHistory,
};
pub use storage::{LocalHistoryStore, SnapshotPage, SnapshotQuery, SnapshotWriteRequest};
