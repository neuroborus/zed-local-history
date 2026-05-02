use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageLayout {
    pub project_dir: PathBuf,
    pub database_path: PathBuf,
    pub blobs_dir: PathBuf,
    pub view_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl StorageLayout {
    pub fn for_project(base_dir: impl AsRef<Path>, project_id: &str) -> Self {
        let project_dir = base_dir.as_ref().join("projects").join(project_id);

        Self {
            database_path: project_dir.join("metadata.sqlite"),
            blobs_dir: project_dir.join("blobs"),
            view_dir: project_dir.join("view"),
            logs_dir: project_dir.join("logs"),
            project_dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotKind {
    Raw,
    Safety,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub id: String,
    pub relative_path: PathBuf,
    pub timestamp: String,
    pub kind: SnapshotKind,
}

pub fn default_data_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home_dir()
            .join("Library")
            .join("Application Support")
            .join("local-history")
    } else if cfg!(target_os = "windows") {
        env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home_dir)
            .join("local-history")
    } else {
        let base_dir = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".local").join("share"));

        base_dir.join("local-history")
    }
}

pub fn placeholder_project_id(project_root: &Path) -> String {
    let raw_name = project_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("project");

    raw_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
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

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::{placeholder_project_id, segment_label, StorageLayout};
    use std::path::Path;

    #[test]
    fn builds_storage_layout_under_projects_directory() {
        let layout = StorageLayout::for_project("/tmp/local-history", "demo-project");

        assert_eq!(
            layout.project_dir,
            Path::new("/tmp/local-history")
                .join("projects")
                .join("demo-project")
        );
        assert_eq!(
            layout.database_path,
            layout.project_dir.join("metadata.sqlite")
        );
        assert_eq!(layout.blobs_dir, layout.project_dir.join("blobs"));
        assert_eq!(layout.view_dir, layout.project_dir.join("view"));
        assert_eq!(layout.logs_dir, layout.project_dir.join("logs"));
    }

    #[test]
    fn derives_a_placeholder_project_id_from_directory_name() {
        let project_id = placeholder_project_id(Path::new("/tmp/My Project"));

        assert_eq!(project_id, "my-project");
    }

    #[test]
    fn maps_minutes_to_fixed_ten_minute_segments() {
        assert_eq!(segment_label(14, 0).as_deref(), Some("14-00__14-10"));
        assert_eq!(segment_label(14, 14).as_deref(), Some("14-10__14-20"));
        assert_eq!(segment_label(14, 59).as_deref(), Some("14-50__15-00"));
        assert!(segment_label(24, 0).is_none());
    }
}
