use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SQLite error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("invalid relative path: {0}")]
    InvalidRelativePath(String),

    #[error("snapshot not found: {0}")]
    SnapshotNotFound(String),

    #[error("content blob not found: {0}")]
    BlobNotFound(String),

    #[error("restore operation not found for project: {0}")]
    RestoreOperationNotFound(String),

    #[error("safety snapshot not found for project: {0}")]
    SafetySnapshotNotFound(String),

    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),

    #[error("invalid time window: {0}")]
    InvalidTimeWindow(String),

    #[error("incompatible storage schema: {0}")]
    IncompatibleSchema(String),

    #[error("local-history store was opened read-only")]
    ReadOnlyStore,

    #[error("snapshot size {size_bytes} bytes exceeds retention limit of {max_bytes} bytes")]
    SnapshotTooLarge { size_bytes: u64, max_bytes: u64 },

    #[error(
        "snapshot ID prefix is too short: got {prefix_len} characters, minimum is {min_len}; use a longer unique prefix or the full snapshot ID"
    )]
    SnapshotPrefixTooShort { prefix_len: usize, min_len: usize },
}
