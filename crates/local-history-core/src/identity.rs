use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::ProjectId;

pub fn project_id_for_root(project_root: &Path) -> ProjectId {
    project_id_from_root_and_salt(project_root, &machine_salt())
}

pub fn project_id_from_root_and_salt(project_root: &Path, machine_salt: &str) -> ProjectId {
    let normalized_root = normalize_project_root(project_root);

    let mut hasher = Sha256::new();
    hasher.update(machine_salt.as_bytes());
    hasher.update([0]);
    hasher.update(normalized_root.to_string_lossy().as_bytes());

    let digest = hasher.finalize();
    ProjectId::new(hex_string(&digest[..16]))
}

pub fn normalize_project_root(project_root: &Path) -> PathBuf {
    project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf())
}

pub fn machine_salt() -> String {
    env::var("LOCAL_HISTORY_MACHINE_SALT")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(best_effort_machine_salt)
}

fn best_effort_machine_salt() -> String {
    for candidate in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(contents) = fs::read_to_string(candidate) {
            let trimmed = contents.trim();

            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }

    let host = env::var("HOSTNAME")
        .or_else(|_| env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-host".to_string());
    let user = env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown-user".to_string());
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_default();

    format!("fallback:{host}:{user}:{}", home.display())
}

fn hex_string(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        use std::fmt::Write as _;

        let _ = write!(&mut hex, "{byte:02x}");
    }

    hex
}

#[cfg(test)]
mod tests {
    use super::{normalize_project_root, project_id_from_root_and_salt};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn project_id_is_stable_for_same_root_and_salt() {
        let temp_dir = create_temp_dir("stable");

        let first = project_id_from_root_and_salt(&temp_dir, "machine-salt");
        let second = project_id_from_root_and_salt(&temp_dir, "machine-salt");

        assert_eq!(first, second);

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn project_id_changes_when_salt_changes() {
        let temp_dir = create_temp_dir("salt-change");

        let first = project_id_from_root_and_salt(&temp_dir, "salt-a");
        let second = project_id_from_root_and_salt(&temp_dir, "salt-b");

        assert_ne!(first, second);

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn project_id_does_not_leak_project_name() {
        let temp_dir = create_temp_dir("Private Project Name");
        let project_id = project_id_from_root_and_salt(&temp_dir, "machine-salt");

        assert!(!project_id.as_str().contains("private"));
        assert_eq!(project_id.as_str().len(), 32);

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn normalize_project_root_prefers_canonical_path() {
        let temp_dir = create_temp_dir("canonical");
        let nested = temp_dir.join("..").join(
            temp_dir
                .file_name()
                .expect("temp dir name must exist for test"),
        );

        assert_eq!(normalize_project_root(&nested), temp_dir);

        cleanup_temp_dir(&temp_dir);
    }

    fn create_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be after UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("zed-local-history-{name}-{unique}"));

        fs::create_dir_all(&path).expect("temp dir must be created");
        path
    }

    fn cleanup_temp_dir(path: &PathBuf) {
        fs::remove_dir_all(path).expect("temp dir must be removed");
    }
}
