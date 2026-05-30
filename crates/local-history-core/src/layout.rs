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
    pub fn for_project(base_dir: impl AsRef<Path>, project_id: impl AsRef<str>) -> Self {
        let project_dir = base_dir.as_ref().join("projects").join(project_id.as_ref());

        Self {
            database_path: project_dir.join("metadata.sqlite"),
            blobs_dir: project_dir.join("blobs"),
            view_dir: project_dir.join("view"),
            logs_dir: project_dir.join("logs"),
            project_dir,
        }
    }
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

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::{default_data_dir, StorageLayout};
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
    fn default_data_dir_targets_local_history_directory() {
        assert!(default_data_dir().ends_with("local-history"));
    }
}
