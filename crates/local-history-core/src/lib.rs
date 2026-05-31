mod diff;
mod error;
mod hashing;
mod identity;
mod ignore;
mod layout;
mod model;
mod storage;
mod time_format;

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
    segment_label, snapshot_id_display_prefix, CompressionKind, ContentBlobRecord, ContentHash,
    GeneratedMarkdownViewEntry, HourBucket, HourHistory, ProjectId, ProjectRecord, PruneReport,
    RestoreOperationRecord, RestoreOutcome, RetentionPolicy, SegmentHistory, SnapshotId,
    SnapshotKind, SnapshotRecord, TimeSegment, TrackedFileRecord, WindowedFileHistory,
    DISPLAY_SNAPSHOT_ID_PREFIX_LEN, MIN_SNAPSHOT_ID_PREFIX_LEN, SNAPSHOT_ID_LEN,
};
pub use storage::{LocalHistoryStore, SnapshotPage, SnapshotQuery, SnapshotWriteRequest};
pub use time_format::{
    format_timestamp_local, format_timestamp_with_offset, init_local_offset_detection,
};
