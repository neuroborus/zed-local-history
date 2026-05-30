use std::path::{Component, Path};

pub const LOCAL_HISTORY_IGNORE_FILE: &str = ".local-history-ignore";

pub const DEFAULT_IGNORED_PATTERNS: &[&str] = &[
    ".git/",
    "node_modules/",
    "target/",
    "dist/",
    "build/",
    ".next/",
    ".cache/",
    "coverage/",
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.sqlite",
    "*.db",
    "*.log",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnorePolicy {
    pub local_ignore_file_name: &'static str,
    pub default_patterns: &'static [&'static str],
    pub max_file_size_bytes: Option<u64>,
}

impl Default for IgnorePolicy {
    fn default() -> Self {
        Self {
            local_ignore_file_name: LOCAL_HISTORY_IGNORE_FILE,
            default_patterns: DEFAULT_IGNORED_PATTERNS,
            max_file_size_bytes: None,
        }
    }
}

pub fn matches_default_ignored_path(relative_path: &Path) -> bool {
    for component in relative_path.components() {
        if let Component::Normal(value) = component {
            let name = value.to_string_lossy();

            if matches!(
                name.as_ref(),
                ".git"
                    | "node_modules"
                    | "target"
                    | "dist"
                    | "build"
                    | ".next"
                    | ".cache"
                    | "coverage"
            ) {
                return true;
            }
        }
    }

    let Some(file_name) = relative_path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };

    file_name == ".env"
        || file_name.starts_with(".env.")
        || [".pem", ".key", ".p12", ".pfx", ".sqlite", ".db", ".log"]
            .iter()
            .any(|suffix| file_name.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::{
        matches_default_ignored_path, IgnorePolicy, DEFAULT_IGNORED_PATTERNS,
        LOCAL_HISTORY_IGNORE_FILE,
    };
    use std::path::Path;

    #[test]
    fn matches_ignored_directories_anywhere_in_the_path() {
        assert!(matches_default_ignored_path(Path::new(
            "frontend/node_modules/react/index.js"
        )));
        assert!(matches_default_ignored_path(Path::new(
            "workspace/.git/config"
        )));
        assert!(matches_default_ignored_path(Path::new(
            "nested/coverage/index.html"
        )));
    }

    #[test]
    fn matches_ignored_environment_and_secret_files() {
        assert!(matches_default_ignored_path(Path::new(".env")));
        assert!(matches_default_ignored_path(Path::new(".env.production")));
        assert!(matches_default_ignored_path(Path::new(
            "config/private-key.pem"
        )));
        assert!(matches_default_ignored_path(Path::new("db/history.sqlite")));
    }

    #[test]
    fn does_not_ignore_normal_source_files() {
        assert!(!matches_default_ignored_path(Path::new("src/lib.rs")));
        assert!(!matches_default_ignored_path(Path::new(
            "docs/history-overview.md"
        )));
    }

    #[test]
    fn default_ignore_policy_matches_repository_contract() {
        let policy = IgnorePolicy::default();

        assert_eq!(policy.local_ignore_file_name, LOCAL_HISTORY_IGNORE_FILE);
        assert_eq!(policy.default_patterns, DEFAULT_IGNORED_PATTERNS);
        assert_eq!(policy.max_file_size_bytes, None);
    }
}
