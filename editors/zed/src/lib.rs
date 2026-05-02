use std::fs;

use zed::process::Command as ProcessCommand;
use zed::serde_json::Value;
use zed::{Architecture, DownloadedFileType, GithubRelease, Os};
use zed_extension_api as zed;

const RELEASE_REPOSITORY: &str = "neuroborus/zed-local-history";
const SIDECAR_BINARY_STEM: &str = "local-history-sidecar";

struct LocalHistoryExtension;

impl zed::Extension for LocalHistoryExtension {
    fn new() -> Self {
        Self
    }

    fn run_slash_command(
        &self,
        command: zed::SlashCommand,
        args: Vec<String>,
        worktree: Option<&zed::Worktree>,
    ) -> Result<zed::SlashCommandOutput, String> {
        let worktree = worktree.ok_or_else(|| {
            "local-history slash commands must run inside an opened Zed worktree".to_string()
        })?;
        let text = match command.name.as_str() {
            "local-history-status" => {
                expect_no_args(&command.name, &args)?;
                let binary = resolve_sidecar_binary(worktree)?;
                let value =
                    run_sidecar_json(worktree, vec!["status".to_string(), worktree.root_path()])?;
                render_status_output(&binary, &value)
            }
            "local-history-start-watcher" => {
                expect_no_args(&command.name, &args)?;
                let value = run_sidecar_json(
                    worktree,
                    vec!["ensure-daemon".to_string(), worktree.root_path()],
                )?;
                render_start_watcher_output(&value)
            }
            "local-history-view" => {
                expect_no_args(&command.name, &args)?;
                let value = run_sidecar_json(
                    worktree,
                    vec!["view-root".to_string(), worktree.root_path()],
                )?;
                render_view_root_output(&value)
            }
            "local-history-current-hour" => {
                expect_no_args(&command.name, &args)?;
                let value = run_sidecar_json(
                    worktree,
                    vec![
                        "render-markdown".to_string(),
                        "current-hour".to_string(),
                        worktree.root_path(),
                    ],
                )?;
                render_markdown_output("current hour", &value)
            }
            "local-history-previous-hour" => {
                expect_no_args(&command.name, &args)?;
                let value = run_sidecar_json(
                    worktree,
                    vec![
                        "render-markdown".to_string(),
                        "previous-hour".to_string(),
                        worktree.root_path(),
                    ],
                )?;
                render_markdown_output("previous hour", &value)
            }
            "local-history-hour" => {
                let hour = expect_single_arg(&command.name, &args, "<YYYY-MM-DDTHH>")?;
                let value = run_sidecar_json(
                    worktree,
                    vec![
                        "render-markdown".to_string(),
                        "hour".to_string(),
                        worktree.root_path(),
                        "--hour".to_string(),
                        hour.to_string(),
                    ],
                )?;
                render_markdown_output("selected hour", &value)
            }
            "local-history-restore" => {
                let snapshot_id = expect_single_arg(&command.name, &args, "<snapshot-id>")?;
                let value = run_sidecar_json(
                    worktree,
                    vec!["restore".to_string(), snapshot_id.to_string()],
                )?;
                render_restore_output(&value)
            }
            _ => {
                return Err(format!(
                    "unknown local-history slash command: {}",
                    command.name
                ))
            }
        };

        Ok(zed::SlashCommandOutput {
            text,
            sections: Vec::new(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReleaseTarget {
    platform_label: &'static str,
    asset_stem: &'static str,
    archive_name: &'static str,
    binary_name: &'static str,
    file_type: DownloadedFileType,
}

fn expect_no_args(command_name: &str, args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(format!("{command_name} does not accept arguments"))
    }
}

fn expect_single_arg<'a>(
    command_name: &str,
    args: &'a [String],
    usage: &str,
) -> Result<&'a str, String> {
    match args {
        [value] => Ok(value.as_str()),
        _ => Err(format!("usage: /{command_name} {usage}")),
    }
}

fn run_sidecar_json(worktree: &zed::Worktree, args: Vec<String>) -> Result<Value, String> {
    let binary = resolve_sidecar_binary(worktree)?;
    let mut command = ProcessCommand::new(binary.clone())
        .args(args.iter().cloned())
        .envs(worktree.shell_env());
    let output = command
        .output()
        .map_err(|error| format!("failed to execute `{binary}`: {error}"))?;

    if output.status != Some(0) {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            "sidecar command failed without stderr output".to_string()
        } else {
            stderr
        };

        return Err(format!("`{binary} {}` failed: {message}", args.join(" ")));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("sidecar output was not valid UTF-8: {error}"))?;

    zed::serde_json::from_str(&stdout)
        .map_err(|error| format!("failed to parse sidecar JSON output: {error}"))
}

fn resolve_sidecar_binary(worktree: &zed::Worktree) -> Result<String, String> {
    let (os, architecture) = zed::current_platform();

    if let Some(path) = sidecar_on_path(worktree, os) {
        return Ok(path);
    }

    let target = release_target(os, architecture)?;
    let cached_path = cached_sidecar_path(target);

    if binary_exists(&cached_path) {
        ensure_binary_executable(target, &cached_path)?;
        return Ok(cached_path);
    }

    install_sidecar_release(target)?;
    ensure_binary_executable(target, &cached_path)?;

    if binary_exists(&cached_path) {
        Ok(cached_path)
    } else {
        Err(format!(
            "downloaded `{}` for {}, but `{cached_path}` was not found after extraction",
            target.archive_name, target.platform_label,
        ))
    }
}

fn install_sidecar_release(target: ReleaseTarget) -> Result<(), String> {
    let release =
        zed::github_release_by_tag_name(RELEASE_REPOSITORY, &release_tag()).map_err(|error| {
            format!(
                "failed to resolve GitHub release {} from {RELEASE_REPOSITORY}: {error}",
                release_tag()
            )
        })?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == target.archive_name)
        .ok_or_else(|| missing_asset_error(target, &release))?;
    let install_dir = install_directory_name();

    zed::download_file(&asset.download_url, &install_dir, target.file_type)
        .map_err(|error| format!("failed to download {}: {error}", asset.name))?;
    cleanup_old_installs(&install_dir)?;

    Ok(())
}

fn sidecar_on_path(worktree: &zed::Worktree, os: Os) -> Option<String> {
    worktree.which(SIDECAR_BINARY_STEM).or_else(|| {
        if matches!(os, Os::Windows) {
            worktree.which("local-history-sidecar.exe")
        } else {
            None
        }
    })
}

fn ensure_binary_executable(target: ReleaseTarget, binary_path: &str) -> Result<(), String> {
    if matches!(target.file_type, DownloadedFileType::Zip) {
        return Ok(());
    }

    zed::make_file_executable(binary_path).map_err(|error| {
        format!("failed to make downloaded sidecar executable at {binary_path}: {error}")
    })
}

fn binary_exists(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

fn cleanup_old_installs(current_install_dir: &str) -> Result<(), String> {
    let entries = fs::read_dir(".")
        .map_err(|error| format!("failed to list extension work directory: {error}"))?;

    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read workdir entry: {error}"))?;
        let file_name = entry.file_name().to_string_lossy().into_owned();

        if file_name.starts_with("local-history-sidecar-")
            && file_name != current_install_dir
            && entry.path().is_dir()
        {
            fs::remove_dir_all(entry.path()).map_err(|error| {
                format!(
                    "failed to remove stale sidecar install {}: {error}",
                    file_name
                )
            })?;
        }
    }

    Ok(())
}

fn missing_asset_error(target: ReleaseTarget, release: &GithubRelease) -> String {
    let available = release
        .assets
        .iter()
        .map(|asset| asset.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "release {} for {} does not contain `{}`. Available assets: [{}]",
        release.version, target.platform_label, target.archive_name, available
    )
}

fn render_status_output(binary: &str, value: &Value) -> String {
    let active = json_bool_at(value, &["watcher", "active"]).unwrap_or(false);
    let pid = json_u64_at(value, &["watcher", "pid"])
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    let project_root = json_string(value, "project_root").unwrap_or("unknown");
    let project_id = json_string(value, "project_id").unwrap_or("unknown");
    let view_root = json_string(value, "view_root").unwrap_or("unknown");
    let database = json_string(value, "database").unwrap_or("unknown");

    format!(
        "Local History status\n\nsidecar: {binary}\nproject_root: {project_root}\nproject_id: {project_id}\nwatcher_active: {active}\npid: {pid}\ndatabase: {database}\nview_root: {view_root}"
    )
}

fn render_start_watcher_output(value: &Value) -> String {
    let started = json_bool(value, "started").unwrap_or(false);
    let active = json_bool_at(value, &["watcher", "active"]).unwrap_or(false);
    let pid = json_u64(value, "spawned_pid")
        .map(|value| value.to_string())
        .or_else(|| json_u64_at(value, &["watcher", "pid"]).map(|value| value.to_string()))
        .unwrap_or_else(|| "n/a".to_string());
    let view_root = json_string_at(value, &["watcher", "view_root"]).unwrap_or("unknown");
    let log_path = json_string_at(value, &["watcher", "log_path"]).unwrap_or("unknown");

    format!(
        "Local History watcher\n\nstarted_new_process: {started}\nwatcher_active: {active}\npid: {pid}\nview_root: {view_root}\nlog_path: {log_path}"
    )
}

fn render_view_root_output(value: &Value) -> String {
    let project_root = json_string(value, "project_root").unwrap_or("unknown");
    let view_root = json_string(value, "view_root").unwrap_or("unknown");

    format!(
        "Local History view root\n\nproject_root: {project_root}\nview_root: {view_root}\n\nUse `zed: open file` if Zed does not expose this path directly."
    )
}

fn render_markdown_output(label: &str, value: &Value) -> String {
    let markdown_path = json_string(value, "markdown_path").unwrap_or("unknown");
    let view_root = json_string(value, "view_root").unwrap_or("unknown");
    let project_root = json_string(value, "project_root").unwrap_or("unknown");
    let scope = json_string(value, "scope").unwrap_or("unknown");

    format!(
        "Local History {label}\n\nscope: {scope}\nproject_root: {project_root}\nmarkdown_path: {markdown_path}\nview_root: {view_root}\n\nThe file is generated on disk. Use `zed: open file` if Zed does not expose this path directly."
    )
}

fn render_restore_output(value: &Value) -> String {
    let restored_snapshot_id = json_string(value, "restored_snapshot_id").unwrap_or("unknown");
    let safety_snapshot_id = json_string(value, "safety_snapshot_id").unwrap_or("unknown");
    let restored_path = json_string(value, "restored_path").unwrap_or("unknown");
    let restore_operation_id = json_string(value, "restore_operation_id").unwrap_or("unknown");

    format!(
        "Local History restore\n\nrestored_snapshot_id: {restored_snapshot_id}\nsafety_snapshot_id: {safety_snapshot_id}\nrestored_path: {restored_path}\nrestore_operation_id: {restore_operation_id}"
    )
}

fn release_tag() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

fn install_directory_name() -> String {
    format!("local-history-sidecar-{}", env!("CARGO_PKG_VERSION"))
}

fn cached_sidecar_path(target: ReleaseTarget) -> String {
    format!(
        "{}/{}/{}",
        install_directory_name(),
        target.asset_stem,
        target.binary_name
    )
}

fn release_target(os: Os, architecture: Architecture) -> Result<ReleaseTarget, String> {
    match (os, architecture) {
        (Os::Mac, Architecture::Aarch64) => Ok(ReleaseTarget {
            platform_label: "macOS aarch64",
            asset_stem: "local-history-sidecar-aarch64-apple-darwin",
            archive_name: "local-history-sidecar-aarch64-apple-darwin.tar.gz",
            binary_name: "local-history-sidecar",
            file_type: DownloadedFileType::GzipTar,
        }),
        (Os::Mac, Architecture::X8664) => Ok(ReleaseTarget {
            platform_label: "macOS x86_64",
            asset_stem: "local-history-sidecar-x86_64-apple-darwin",
            archive_name: "local-history-sidecar-x86_64-apple-darwin.tar.gz",
            binary_name: "local-history-sidecar",
            file_type: DownloadedFileType::GzipTar,
        }),
        (Os::Linux, Architecture::X8664) => Ok(ReleaseTarget {
            platform_label: "Linux x86_64",
            asset_stem: "local-history-sidecar-x86_64-unknown-linux-gnu",
            archive_name: "local-history-sidecar-x86_64-unknown-linux-gnu.tar.gz",
            binary_name: "local-history-sidecar",
            file_type: DownloadedFileType::GzipTar,
        }),
        (Os::Windows, Architecture::X8664) => Ok(ReleaseTarget {
            platform_label: "Windows x86_64",
            asset_stem: "local-history-sidecar-x86_64-pc-windows-msvc",
            archive_name: "local-history-sidecar-x86_64-pc-windows-msvc.zip",
            binary_name: "local-history-sidecar.exe",
            file_type: DownloadedFileType::Zip,
        }),
        _ => Err(format!(
            "local-history sidecar bootstrap is not defined for {}",
            platform_label(os, architecture)
        )),
    }
}

fn platform_label(os: Os, architecture: Architecture) -> &'static str {
    match (os, architecture) {
        (Os::Mac, Architecture::Aarch64) => "macOS aarch64",
        (Os::Mac, Architecture::X86) => "macOS x86",
        (Os::Mac, Architecture::X8664) => "macOS x86_64",
        (Os::Linux, Architecture::Aarch64) => "Linux aarch64",
        (Os::Linux, Architecture::X86) => "Linux x86",
        (Os::Linux, Architecture::X8664) => "Linux x86_64",
        (Os::Windows, Architecture::Aarch64) => "Windows aarch64",
        (Os::Windows, Architecture::X86) => "Windows x86",
        (Os::Windows, Architecture::X8664) => "Windows x86_64",
    }
}

fn json_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn json_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key)?.as_bool()
}

fn json_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key)?.as_u64()
}

fn json_string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    json_value_at(value, path)?.as_str()
}

fn json_bool_at(value: &Value, path: &[&str]) -> Option<bool> {
    json_value_at(value, path)?.as_bool()
}

fn json_u64_at(value: &Value, path: &[&str]) -> Option<u64> {
    json_value_at(value, path)?.as_u64()
}

fn json_value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;

    for segment in path {
        current = current.get(*segment)?;
    }

    Some(current)
}

#[cfg(test)]
mod tests {
    use super::{install_directory_name, release_tag, release_target, ReleaseTarget};
    use zed_extension_api::{Architecture, DownloadedFileType, Os};

    #[test]
    fn maps_linux_release_target() {
        assert_eq!(
            release_target(Os::Linux, Architecture::X8664).expect("linux target must exist"),
            ReleaseTarget {
                platform_label: "Linux x86_64",
                asset_stem: "local-history-sidecar-x86_64-unknown-linux-gnu",
                archive_name: "local-history-sidecar-x86_64-unknown-linux-gnu.tar.gz",
                binary_name: "local-history-sidecar",
                file_type: DownloadedFileType::GzipTar,
            }
        );
    }

    #[test]
    fn version_helpers_follow_package_version() {
        assert_eq!(release_tag(), "v0.1.0");
        assert_eq!(install_directory_name(), "local-history-sidecar-0.1.0");
    }
}

zed::register_extension!(LocalHistoryExtension);
