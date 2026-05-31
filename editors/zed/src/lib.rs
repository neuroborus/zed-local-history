use std::fs;
use std::path::Path;

use zed::process::Command as ProcessCommand;
use zed::serde_json::Value;
use zed::{Architecture, DownloadedFileType, GithubRelease, Os};
use zed_extension_api as zed;

const RELEASE_REPOSITORY: &str = "neuroborus/zed-local-history";
const SIDECAR_BINARY_STEM: &str = "local-history-sidecar";
const MCP_BINARY_STEM: &str = "local-history-mcp";
const MCP_CONTEXT_SERVER_ID: &str = "local-history";
const MINIMUM_COMPATIBLE_SIDECAR_VERSION: &str = "0.1.0";
const MINIMUM_COMPATIBLE_MCP_VERSION: &str = "0.1.0";

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
                render_status_output(&binary.display_path, &value)
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
            "local-history-current-segment" => {
                expect_no_args(&command.name, &args)?;
                let value = run_sidecar_json(
                    worktree,
                    vec![
                        "render-markdown".to_string(),
                        "current-segment".to_string(),
                        worktree.root_path(),
                    ],
                )?;
                render_markdown_output("current segment", &value)
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
            "local-history-segment" => {
                let timestamp = expect_single_arg(&command.name, &args, "<YYYY-MM-DDTHH:MM:SSZ>")?;
                let value = run_sidecar_json(
                    worktree,
                    vec![
                        "render-markdown".to_string(),
                        "segment-at".to_string(),
                        worktree.root_path(),
                        "--at".to_string(),
                        timestamp.to_string(),
                    ],
                )?;
                render_markdown_output("selected segment", &value)
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

    fn context_server_command(
        &mut self,
        context_server_id: &zed::ContextServerId,
        _project: &zed::Project,
    ) -> Result<zed::Command, String> {
        if context_server_id.as_ref() != MCP_CONTEXT_SERVER_ID {
            return Err(format!(
                "unknown local-history context server: {context_server_id}"
            ));
        }

        let binary = resolve_mcp_binary()?;
        let env = binary.env(zed::current_platform().0, Vec::<(String, String)>::new());
        Ok(zed::Command {
            command: binary.command,
            args: Vec::new(),
            env,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct BinaryCommand {
    command: String,
    path_prefix: Option<String>,
    display_path: String,
}

impl BinaryCommand {
    fn lookup_name(command: impl Into<String>) -> Self {
        let command = command.into();
        Self {
            display_path: command.clone(),
            command,
            path_prefix: None,
        }
    }

    fn from_path(os: Os, path: &str, fallback_command: &str) -> Self {
        if let Some((directory, file_name)) = split_executable_path(os, path) {
            Self {
                command: file_name,
                path_prefix: Some(directory),
                display_path: path.to_string(),
            }
        } else {
            Self::lookup_name(fallback_command)
        }
    }

    fn env(
        &self,
        os: Os,
        base_env: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Vec<(String, String)> {
        let mut env = base_env
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect::<Vec<_>>();

        if let Some(path_prefix) = &self.path_prefix {
            prepend_path_env(os, &mut env, path_prefix);
        }

        env
    }
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
    run_sidecar_json_with_binary(worktree, &binary, args)
}

fn run_sidecar_json_with_binary(
    worktree: &zed::Worktree,
    binary: &BinaryCommand,
    args: Vec<String>,
) -> Result<Value, String> {
    let (os, _) = zed::current_platform();
    let binary_label = binary.display_path.clone();
    let mut command = ProcessCommand::new(binary.command.clone())
        .args(args.iter().cloned())
        .envs(binary.env(os, worktree.shell_env()));
    let output = command
        .output()
        .map_err(|error| format!("failed to execute `{binary_label}`: {error}"))?;

    if output.status != Some(0) {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            "sidecar command failed without stderr output".to_string()
        } else {
            stderr
        };

        return Err(format!(
            "`{binary_label} {}` failed: {message}",
            args.join(" ")
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("sidecar output was not valid UTF-8: {error}"))?;

    zed::serde_json::from_str(&stdout)
        .map_err(|error| format!("failed to parse sidecar JSON output: {error}"))
}

fn resolve_sidecar_binary(worktree: &zed::Worktree) -> Result<BinaryCommand, String> {
    let (os, architecture) = zed::current_platform();

    if let Some(binary) = sidecar_on_path(worktree, os) {
        if sidecar_is_compatible(worktree, &binary)? {
            return Ok(binary);
        }
    }

    let target = release_target(os, architecture)?;
    let cached_path = cached_sidecar_path(target);

    if binary_exists(&cached_path) {
        ensure_binary_executable(target, &cached_path)?;
        let binary = cached_binary_command(os, target, &cached_path)?;

        if sidecar_is_compatible(worktree, &binary)? {
            return Ok(binary);
        }
    }

    install_sidecar_release(target)?;
    ensure_binary_executable(target, &cached_path)?;

    if binary_exists(&cached_path) {
        let binary = cached_binary_command(os, target, &cached_path)?;
        let version = sidecar_version(worktree, &binary)?;

        if is_compatible_sidecar_version(&version)? {
            Ok(binary)
        } else {
            Err(incompatible_sidecar_error(&cached_path, &version))
        }
    } else {
        Err(format!(
            "downloaded `{}` for {}, but `{cached_path}` was not found after extraction",
            target.archive_name, target.platform_label,
        ))
    }
}

fn resolve_mcp_binary() -> Result<BinaryCommand, String> {
    let (os, architecture) = zed::current_platform();

    if let Some(binary) = mcp_on_path(os) {
        if mcp_is_compatible(&binary)? {
            if let Ok(resolved_path) = resolve_lookup_binary_path(os, &binary.command) {
                return Ok(BinaryCommand::from_path(
                    os,
                    &resolved_path,
                    &binary.command,
                ));
            }
        }
    }

    let target = mcp_release_target(os, architecture)?;
    let cached_path = cached_mcp_path(target);

    if binary_exists(&cached_path) {
        ensure_binary_executable(target, &cached_path)?;
        let binary = cached_binary_command(os, target, &cached_path)?;

        if mcp_is_compatible(&binary)? {
            return Ok(binary);
        }
    }

    install_mcp_release(target)?;
    ensure_binary_executable(target, &cached_path)?;

    if binary_exists(&cached_path) {
        let binary = cached_binary_command(os, target, &cached_path)?;
        let version = mcp_version(&binary)?;

        if is_compatible_mcp_version(&version)? {
            Ok(binary)
        } else {
            Err(incompatible_mcp_error(&cached_path, &version))
        }
    } else {
        Err(format!(
            "downloaded `{}` for {}, but `{cached_path}` was not found after extraction",
            target.archive_name, target.platform_label,
        ))
    }
}

fn install_sidecar_release(target: ReleaseTarget) -> Result<(), String> {
    install_release_asset(
        target,
        &sidecar_install_directory_name(),
        "local-history-sidecar-",
    )
}

fn install_mcp_release(target: ReleaseTarget) -> Result<(), String> {
    install_release_asset(target, &mcp_install_directory_name(), "local-history-mcp-")
}

fn install_release_asset(
    target: ReleaseTarget,
    install_dir: &str,
    cleanup_prefix: &str,
) -> Result<(), String> {
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

    zed::download_file(&asset.download_url, install_dir, target.file_type)
        .map_err(|error| format!("failed to download {}: {error}", asset.name))?;
    cleanup_old_installs(cleanup_prefix, install_dir)?;

    Ok(())
}

fn sidecar_is_compatible(worktree: &zed::Worktree, binary: &BinaryCommand) -> Result<bool, String> {
    match sidecar_version(worktree, binary) {
        Ok(version) => is_compatible_sidecar_version(&version),
        Err(_) => Ok(false),
    }
}

fn sidecar_version(worktree: &zed::Worktree, binary: &BinaryCommand) -> Result<String, String> {
    let value = run_sidecar_json_with_binary(worktree, binary, vec!["version".to_string()])?;
    json_string(&value, "sidecar_version")
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "`{} version` did not return `sidecar_version`",
                binary.display_path
            )
        })
}

fn mcp_is_compatible(binary: &BinaryCommand) -> Result<bool, String> {
    match mcp_version(binary) {
        Ok(version) => is_compatible_mcp_version(&version),
        Err(_) => Ok(false),
    }
}

fn mcp_version(binary: &BinaryCommand) -> Result<String, String> {
    let (os, _) = zed::current_platform();
    let binary_label = binary.display_path.clone();
    let output = ProcessCommand::new(binary.command.clone())
        .args(["--version"])
        .envs(binary.env(os, Vec::<(String, String)>::new()))
        .output()
        .map_err(|error| format!("failed to execute `{binary_label} --version`: {error}"))?;

    if output.status != Some(0) {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            "MCP binary version command failed without stderr output".to_string()
        } else {
            stderr
        };

        return Err(format!("`{binary_label} --version` failed: {message}"));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("MCP version output was not valid UTF-8: {error}"))?;
    let version = stdout.trim();

    if version.is_empty() {
        Err(format!(
            "`{binary_label} --version` returned an empty version"
        ))
    } else {
        Ok(version.to_string())
    }
}

fn sidecar_on_path(worktree: &zed::Worktree, os: Os) -> Option<BinaryCommand> {
    [SIDECAR_BINARY_STEM, "local-history-sidecar.exe"]
        .into_iter()
        .filter(|name| matches!(os, Os::Windows) || *name == SIDECAR_BINARY_STEM)
        .find_map(|name| {
            worktree
                .which(name)
                .map(|path| BinaryCommand::from_path(os, &path, name))
        })
}

fn mcp_on_path(os: Os) -> Option<BinaryCommand> {
    [MCP_BINARY_STEM, "local-history-mcp.exe"]
        .into_iter()
        .filter(|name| matches!(os, Os::Windows) || *name == MCP_BINARY_STEM)
        .find_map(|name| {
            let binary = BinaryCommand::lookup_name(name);
            if mcp_is_compatible(&binary).unwrap_or(false) {
                Some(binary)
            } else {
                None
            }
        })
}

fn cached_binary_command(
    os: Os,
    target: ReleaseTarget,
    cached_path: &str,
) -> Result<BinaryCommand, String> {
    let resolved_path = resolve_executable_path(cached_path)?;
    Ok(BinaryCommand::from_path(
        os,
        &resolved_path,
        target.binary_name,
    ))
}

fn split_executable_path(os: Os, path: &str) -> Option<(String, String)> {
    let split_at = if matches!(os, Os::Windows) {
        path.rfind(['\\', '/'])
    } else {
        path.rfind('/')
    }?;

    let directory = path[..split_at].to_string();
    let file_name = path[split_at + 1..].to_string();

    if directory.is_empty() || file_name.is_empty() {
        None
    } else {
        Some((directory, file_name))
    }
}

fn prepend_path_env(os: Os, env: &mut Vec<(String, String)>, path_prefix: &str) {
    let path_key_index = env.iter().position(|(key, _)| {
        if matches!(os, Os::Windows) {
            key.eq_ignore_ascii_case("PATH")
        } else {
            key == "PATH"
        }
    });

    let separator = if matches!(os, Os::Windows) { ";" } else { ":" };

    if let Some(index) = path_key_index {
        if env[index].1.is_empty() {
            env[index].1 = path_prefix.to_string();
        } else {
            env[index].1 = format!("{path_prefix}{separator}{}", env[index].1);
        }
    } else {
        env.push(("PATH".to_string(), path_prefix.to_string()));
    }
}

fn ensure_binary_executable(target: ReleaseTarget, binary_path: &str) -> Result<(), String> {
    if matches!(target.file_type, DownloadedFileType::Zip) {
        return Ok(());
    }

    zed::make_file_executable(binary_path).map_err(|error| {
        format!("failed to make downloaded binary executable at {binary_path}: {error}")
    })
}

fn binary_exists(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

fn resolve_executable_path(binary: &str) -> Result<String, String> {
    let path = Path::new(binary);

    if path.is_absolute() || !binary.contains('/') && !binary.contains('\\') {
        return Ok(binary.to_string());
    }

    fs::canonicalize(path)
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| format!("failed to resolve executable path `{binary}`: {error}"))
}

fn resolve_lookup_binary_path(os: Os, binary: &str) -> Result<String, String> {
    let path = Path::new(binary);

    if path.is_absolute() {
        return Ok(binary.to_string());
    }

    if binary.contains('/') || binary.contains('\\') {
        return resolve_executable_path(binary);
    }

    let (program, args) = path_lookup_command(os, binary);
    let output = ProcessCommand::new(program)
        .args(args.iter().cloned())
        .output()
        .map_err(|error| format!("failed to resolve `{binary}` on PATH: {error}"))?;

    if output.status != Some(0) {
        return Err(format!(
            "`{binary}` was found during compatibility checks, but could not be resolved to an absolute path"
        ));
    }

    let resolved = String::from_utf8(output.stdout)
        .map_err(|error| format!("PATH lookup output for `{binary}` was not valid UTF-8: {error}"))?
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();

    if resolved.is_empty() {
        return Err(format!(
            "`{binary}` was found during compatibility checks, but PATH lookup returned an empty path"
        ));
    }

    Ok(resolved)
}

fn path_lookup_command(os: Os, binary: &str) -> (&'static str, Vec<String>) {
    if matches!(os, Os::Windows) {
        ("where", vec![binary.to_string()])
    } else {
        ("sh", vec!["-c".to_string(), format!("command -v {binary}")])
    }
}

fn cleanup_old_installs(prefix: &str, current_install_dir: &str) -> Result<(), String> {
    let entries = fs::read_dir(".")
        .map_err(|error| format!("failed to list extension work directory: {error}"))?;

    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read workdir entry: {error}"))?;
        let file_name = entry.file_name().to_string_lossy().into_owned();

        if file_name.starts_with(prefix)
            && file_name != current_install_dir
            && entry.path().is_dir()
        {
            fs::remove_dir_all(entry.path()).map_err(|error| {
                format!(
                    "failed to remove stale binary install {}: {error}",
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

fn extension_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn sidecar_install_directory_name() -> String {
    format!("local-history-sidecar-{}", env!("CARGO_PKG_VERSION"))
}

fn mcp_install_directory_name() -> String {
    format!("local-history-mcp-{}", env!("CARGO_PKG_VERSION"))
}

fn cached_sidecar_path(target: ReleaseTarget) -> String {
    format!(
        "{}/{}/{}",
        sidecar_install_directory_name(),
        target.asset_stem,
        target.binary_name
    )
}

fn cached_mcp_path(target: ReleaseTarget) -> String {
    format!(
        "{}/{}/{}",
        mcp_install_directory_name(),
        target.asset_stem,
        target.binary_name
    )
}

fn mcp_release_target(os: Os, architecture: Architecture) -> Result<ReleaseTarget, String> {
    match (os, architecture) {
        (Os::Mac, Architecture::Aarch64) => Ok(ReleaseTarget {
            platform_label: "macOS aarch64",
            asset_stem: "local-history-mcp-aarch64-apple-darwin",
            archive_name: "local-history-mcp-aarch64-apple-darwin.tar.gz",
            binary_name: "local-history-mcp",
            file_type: DownloadedFileType::GzipTar,
        }),
        (Os::Mac, Architecture::X8664) => Ok(ReleaseTarget {
            platform_label: "macOS x86_64",
            asset_stem: "local-history-mcp-x86_64-apple-darwin",
            archive_name: "local-history-mcp-x86_64-apple-darwin.tar.gz",
            binary_name: "local-history-mcp",
            file_type: DownloadedFileType::GzipTar,
        }),
        (Os::Linux, Architecture::Aarch64) => Ok(ReleaseTarget {
            platform_label: "Linux aarch64",
            asset_stem: "local-history-mcp-aarch64-unknown-linux-gnu",
            archive_name: "local-history-mcp-aarch64-unknown-linux-gnu.tar.gz",
            binary_name: "local-history-mcp",
            file_type: DownloadedFileType::GzipTar,
        }),
        (Os::Linux, Architecture::X8664) => Ok(ReleaseTarget {
            platform_label: "Linux x86_64",
            asset_stem: "local-history-mcp-x86_64-unknown-linux-gnu",
            archive_name: "local-history-mcp-x86_64-unknown-linux-gnu.tar.gz",
            binary_name: "local-history-mcp",
            file_type: DownloadedFileType::GzipTar,
        }),
        (Os::Windows, Architecture::Aarch64) => Ok(ReleaseTarget {
            platform_label: "Windows aarch64",
            asset_stem: "local-history-mcp-aarch64-pc-windows-msvc",
            archive_name: "local-history-mcp-aarch64-pc-windows-msvc.zip",
            binary_name: "local-history-mcp.exe",
            file_type: DownloadedFileType::Zip,
        }),
        (Os::Windows, Architecture::X8664) => Ok(ReleaseTarget {
            platform_label: "Windows x86_64",
            asset_stem: "local-history-mcp-x86_64-pc-windows-msvc",
            archive_name: "local-history-mcp-x86_64-pc-windows-msvc.zip",
            binary_name: "local-history-mcp.exe",
            file_type: DownloadedFileType::Zip,
        }),
        _ => Err(format!(
            "local-history MCP bootstrap is not defined for {}",
            platform_label(os, architecture)
        )),
    }
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
        (Os::Linux, Architecture::Aarch64) => Ok(ReleaseTarget {
            platform_label: "Linux aarch64",
            asset_stem: "local-history-sidecar-aarch64-unknown-linux-gnu",
            archive_name: "local-history-sidecar-aarch64-unknown-linux-gnu.tar.gz",
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
        (Os::Windows, Architecture::Aarch64) => Ok(ReleaseTarget {
            platform_label: "Windows aarch64",
            asset_stem: "local-history-sidecar-aarch64-pc-windows-msvc",
            archive_name: "local-history-sidecar-aarch64-pc-windows-msvc.zip",
            binary_name: "local-history-sidecar.exe",
            file_type: DownloadedFileType::Zip,
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

fn is_compatible_sidecar_version(version: &str) -> Result<bool, String> {
    Ok(parse_semver_triplet(version)? >= parse_semver_triplet(MINIMUM_COMPATIBLE_SIDECAR_VERSION)?)
}

fn is_compatible_mcp_version(version: &str) -> Result<bool, String> {
    Ok(parse_semver_triplet(version)? >= parse_semver_triplet(MINIMUM_COMPATIBLE_MCP_VERSION)?)
}

fn incompatible_sidecar_error(binary: &str, version: &str) -> String {
    format!(
        "local-history extension {} requires sidecar >= {}, but `{binary}` reports {}",
        extension_version(),
        MINIMUM_COMPATIBLE_SIDECAR_VERSION,
        version
    )
}

fn incompatible_mcp_error(binary: &str, version: &str) -> String {
    format!(
        "local-history extension {} requires MCP binary >= {}, but `{binary}` reports {}",
        extension_version(),
        MINIMUM_COMPATIBLE_MCP_VERSION,
        version
    )
}

fn parse_semver_triplet(version: &str) -> Result<(u64, u64, u64), String> {
    let version = version
        .split_once('-')
        .map(|(core, _)| core)
        .unwrap_or(version);
    let version = version
        .split_once('+')
        .map(|(core, _)| core)
        .unwrap_or(version);
    let mut parts = version.split('.');

    let major = parts
        .next()
        .ok_or_else(|| format!("invalid version `{version}`"))?
        .parse::<u64>()
        .map_err(|error| format!("invalid major version in `{version}`: {error}"))?;
    let minor = parts
        .next()
        .ok_or_else(|| format!("invalid version `{version}`"))?
        .parse::<u64>()
        .map_err(|error| format!("invalid minor version in `{version}`: {error}"))?;
    let patch = parts
        .next()
        .ok_or_else(|| format!("invalid version `{version}`"))?
        .parse::<u64>()
        .map_err(|error| format!("invalid patch version in `{version}`: {error}"))?;

    if parts.next().is_some() {
        return Err(format!("invalid version `{version}`"));
    }

    Ok((major, minor, patch))
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
    use super::{
        cached_binary_command, cached_mcp_path, extension_version, is_compatible_mcp_version,
        is_compatible_sidecar_version, mcp_install_directory_name, mcp_release_target,
        parse_semver_triplet, path_lookup_command, release_tag, release_target,
        resolve_executable_path, sidecar_install_directory_name, BinaryCommand, ReleaseTarget,
    };
    use std::path::Path;
    use zed_extension_api::{Architecture, DownloadedFileType, Os};

    const EXTENSION_MANIFEST: &str = include_str!("../extension.toml");

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
    fn maps_linux_mcp_release_target() {
        assert_eq!(
            mcp_release_target(Os::Linux, Architecture::X8664)
                .expect("linux MCP target must exist"),
            ReleaseTarget {
                platform_label: "Linux x86_64",
                asset_stem: "local-history-mcp-x86_64-unknown-linux-gnu",
                archive_name: "local-history-mcp-x86_64-unknown-linux-gnu.tar.gz",
                binary_name: "local-history-mcp",
                file_type: DownloadedFileType::GzipTar,
            }
        );
    }

    #[test]
    fn maps_windows_arm_mcp_release_target() {
        assert_eq!(
            mcp_release_target(Os::Windows, Architecture::Aarch64)
                .expect("windows arm MCP target must exist"),
            ReleaseTarget {
                platform_label: "Windows aarch64",
                asset_stem: "local-history-mcp-aarch64-pc-windows-msvc",
                archive_name: "local-history-mcp-aarch64-pc-windows-msvc.zip",
                binary_name: "local-history-mcp.exe",
                file_type: DownloadedFileType::Zip,
            }
        );
    }

    #[test]
    fn maps_linux_arm_release_target() {
        assert_eq!(
            release_target(Os::Linux, Architecture::Aarch64).expect("linux arm target must exist"),
            ReleaseTarget {
                platform_label: "Linux aarch64",
                asset_stem: "local-history-sidecar-aarch64-unknown-linux-gnu",
                archive_name: "local-history-sidecar-aarch64-unknown-linux-gnu.tar.gz",
                binary_name: "local-history-sidecar",
                file_type: DownloadedFileType::GzipTar,
            }
        );
    }

    #[test]
    fn maps_windows_arm_release_target() {
        assert_eq!(
            release_target(Os::Windows, Architecture::Aarch64)
                .expect("windows arm target must exist"),
            ReleaseTarget {
                platform_label: "Windows aarch64",
                asset_stem: "local-history-sidecar-aarch64-pc-windows-msvc",
                archive_name: "local-history-sidecar-aarch64-pc-windows-msvc.zip",
                binary_name: "local-history-sidecar.exe",
                file_type: DownloadedFileType::Zip,
            }
        );
    }

    #[test]
    fn version_helpers_follow_package_version() {
        assert_eq!(release_tag(), "v0.1.0");
        assert_eq!(extension_version(), "0.1.0");
        assert_eq!(
            sidecar_install_directory_name(),
            "local-history-sidecar-0.1.0"
        );
        assert_eq!(mcp_install_directory_name(), "local-history-mcp-0.1.0");
    }

    #[test]
    fn parses_semver_triplets_with_optional_suffixes() {
        assert_eq!(
            parse_semver_triplet("1.2.3").expect("plain semver must parse"),
            (1, 2, 3)
        );
        assert_eq!(
            parse_semver_triplet("1.2.3-rc.1").expect("pre-release semver must parse"),
            (1, 2, 3)
        );
        assert_eq!(
            parse_semver_triplet("1.2.3+build.7").expect("build metadata semver must parse"),
            (1, 2, 3)
        );
    }

    #[test]
    fn compatibility_check_uses_minimum_sidecar_version() {
        assert!(
            is_compatible_sidecar_version("0.1.0").expect("matching version must be compatible")
        );
        assert!(
            is_compatible_sidecar_version("0.1.5").expect("newer patch version must be compatible")
        );
        assert!(!is_compatible_sidecar_version("0.0.9")
            .expect("older sidecar version must be incompatible"));
    }

    #[test]
    fn compatibility_check_uses_minimum_mcp_version() {
        assert!(is_compatible_mcp_version("0.1.0").expect("matching version must be compatible"));
        assert!(is_compatible_mcp_version("0.1.5").expect("newer patch version must be compatible"));
        assert!(
            !is_compatible_mcp_version("0.0.9").expect("older MCP version must be incompatible")
        );
    }

    #[test]
    fn extension_manifest_declares_context_server_and_mcp_capabilities() {
        assert!(
            EXTENSION_MANIFEST.contains("[context_servers.local-history]"),
            "extension must register the local-history MCP context server"
        );
        assert!(
            EXTENSION_MANIFEST.contains(r#"command = "local-history-mcp""#),
            "extension must declare process:exec for local-history-mcp"
        );
        assert!(
            EXTENSION_MANIFEST.contains(r#"command = "local-history-sidecar""#),
            "extension must declare process:exec for local-history-sidecar"
        );
        assert!(
            EXTENSION_MANIFEST.contains(r#"command = "sh""#)
                && EXTENSION_MANIFEST.contains("command -v local-history-mcp")
                && EXTENSION_MANIFEST.contains(r#"command = "where""#),
            "extension must declare narrow PATH lookup helpers for MCP dev binaries"
        );
        assert!(
            !EXTENSION_MANIFEST.contains(r#"command = "*""#),
            "extension must not grant wildcard process execution"
        );
        assert!(
            EXTENSION_MANIFEST.contains("download_file")
                && EXTENSION_MANIFEST.contains("github.com")
                && EXTENSION_MANIFEST.contains("zed-local-history"),
            "extension must allow GitHub release bootstrap downloads"
        );
    }

    #[test]
    fn binary_command_uses_stable_command_name_and_path_prefix() {
        let binary = BinaryCommand::from_path(
            Os::Linux,
            "/tmp/local-history-cache/local-history-mcp",
            "local-history-mcp",
        );

        assert_eq!(binary.command, "local-history-mcp");
        assert_eq!(
            binary.path_prefix.as_deref(),
            Some("/tmp/local-history-cache")
        );
        assert_eq!(
            binary.display_path,
            "/tmp/local-history-cache/local-history-mcp"
        );

        let env = binary.env(Os::Linux, vec![("PATH", "/usr/bin")]);
        assert_eq!(
            env,
            vec![(
                "PATH".to_string(),
                "/tmp/local-history-cache:/usr/bin".to_string()
            )]
        );
    }

    #[test]
    fn binary_command_uses_windows_path_separator() {
        let binary = BinaryCommand::from_path(
            Os::Windows,
            r"C:\tools\local-history-mcp.exe",
            "local-history-mcp.exe",
        );

        assert_eq!(binary.command, "local-history-mcp.exe");
        assert_eq!(binary.path_prefix.as_deref(), Some(r"C:\tools"));

        let env = binary.env(Os::Windows, vec![("Path", r"C:\Windows\System32")]);
        assert_eq!(
            env,
            vec![(
                "Path".to_string(),
                r"C:\tools;C:\Windows\System32".to_string()
            )]
        );
    }

    #[test]
    fn cached_mcp_path_resolves_to_spawn_ready_absolute_path() {
        let target = mcp_release_target(Os::Linux, Architecture::X8664)
            .expect("linux MCP target must exist");
        let base = std::env::temp_dir().join(format!(
            "local-history-zed-cache-test-{}",
            std::process::id()
        ));
        let nested = base.join(format!(
            "{}/{}",
            mcp_install_directory_name(),
            target.asset_stem
        ));
        std::fs::create_dir_all(&nested).expect("nested cache dir must be creatable");
        let binary = nested.join(target.binary_name);
        std::fs::write(&binary, b"").expect("placeholder binary must be writable");

        let previous_dir = std::env::current_dir().expect("current dir must be readable");
        std::env::set_current_dir(&base).expect("test must chdir into temp cache root");

        let relative = cached_mcp_path(target);
        let binary =
            cached_binary_command(Os::Linux, target, &relative).expect("cache path must resolve");
        let path_prefix = binary
            .path_prefix
            .as_deref()
            .expect("cache binary must provide a PATH prefix");

        assert_eq!(binary.command, "local-history-mcp");
        assert!(Path::new(path_prefix).is_absolute());
        assert!(Path::new(&binary.display_path).is_absolute());
        assert!(Path::new(&binary.display_path).exists());

        std::env::set_current_dir(previous_dir).expect("test must restore previous dir");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn resolve_executable_path_leaves_lookup_names_unchanged() {
        assert_eq!(
            resolve_executable_path("local-history-mcp").expect("lookup name must pass through"),
            "local-history-mcp"
        );
        assert_eq!(
            resolve_executable_path("local-history-sidecar.exe")
                .expect("windows lookup name must pass through"),
            "local-history-sidecar.exe"
        );
    }

    #[test]
    fn resolve_executable_path_leaves_absolute_paths_unchanged() {
        let path = if cfg!(windows) {
            r"C:\tools\local-history-mcp.exe".to_string()
        } else {
            "/usr/local/bin/local-history-mcp".to_string()
        };

        assert_eq!(
            resolve_executable_path(&path).expect("absolute path must pass through"),
            path
        );
    }

    #[test]
    fn path_lookup_command_uses_zed_host_os() {
        assert_eq!(
            path_lookup_command(Os::Windows, "local-history-mcp.exe"),
            ("where", vec!["local-history-mcp.exe".to_string()])
        );
        assert_eq!(
            path_lookup_command(Os::Linux, "local-history-mcp"),
            (
                "sh",
                vec!["-c".to_string(), "command -v local-history-mcp".to_string()]
            )
        );
    }

    #[test]
    fn resolve_executable_path_canonicalizes_existing_relative_paths() {
        let base =
            std::env::temp_dir().join(format!("local-history-zed-ext-test-{}", std::process::id()));
        let nested =
            base.join("local-history-mcp-0.1.0/local-history-mcp-x86_64-unknown-linux-gnu");
        std::fs::create_dir_all(&nested).expect("nested cache dir must be creatable");
        let binary = nested.join("local-history-mcp");
        std::fs::write(&binary, b"").expect("placeholder binary must be writable");

        let previous_dir = std::env::current_dir().expect("current dir must be readable");
        std::env::set_current_dir(&base).expect("test must chdir into temp cache root");

        let relative =
            "local-history-mcp-0.1.0/local-history-mcp-x86_64-unknown-linux-gnu/local-history-mcp";
        let resolved = resolve_executable_path(relative).expect("relative cache path must resolve");

        assert!(Path::new(&resolved).is_absolute());
        assert!(Path::new(&resolved).exists());

        std::env::set_current_dir(previous_dir).expect("test must restore previous dir");
        let _ = std::fs::remove_dir_all(base);
    }
}

zed::register_extension!(LocalHistoryExtension);
