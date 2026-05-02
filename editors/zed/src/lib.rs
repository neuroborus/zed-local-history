use zed::process::Command as ProcessCommand;
use zed::serde_json::Value;
use zed::{Architecture, Os};
use zed_extension_api as zed;

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

    if let Some(path) = worktree.which("local-history-sidecar") {
        return Ok(path);
    }

    if matches!(os, Os::Windows) {
        if let Some(path) = worktree.which("local-history-sidecar.exe") {
            return Ok(path);
        }
    }

    Err(format!(
        "local-history-sidecar is not on PATH for {}. Expected binary `{}{}` from release artifact `{}` or a dev build available on PATH.",
        platform_label(os, architecture),
        "local-history-sidecar",
        binary_suffix(os),
        expected_release_artifact(os, architecture),
    ))
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

fn expected_release_artifact(os: Os, architecture: Architecture) -> &'static str {
    match (os, architecture) {
        (Os::Mac, Architecture::Aarch64) => "local-history-aarch64-apple-darwin",
        (Os::Mac, Architecture::X8664) => "local-history-x86_64-apple-darwin",
        (Os::Linux, Architecture::X8664) => "local-history-x86_64-unknown-linux-gnu",
        (Os::Windows, Architecture::X8664) => "local-history-x86_64-pc-windows-msvc",
        _ => "local-history release artifact for this platform is not defined yet",
    }
}

fn binary_suffix(os: Os) -> &'static str {
    match os {
        Os::Windows => ".exe",
        _ => "",
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

zed::register_extension!(LocalHistoryExtension);
