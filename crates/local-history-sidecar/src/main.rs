mod runtime;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SidecarCommand {
    Health,
    Watch {
        project_root: PathBuf,
        daemon_mode: bool,
    },
    EnsureDaemon {
        project_root: PathBuf,
    },
    Status {
        project_root: PathBuf,
    },
    ViewRoot {
        project_root: PathBuf,
    },
    RenderCurrentHour {
        project_root: PathBuf,
    },
    RenderPreviousHour {
        project_root: PathBuf,
    },
    RenderHour {
        project_root: PathBuf,
        hour: String,
    },
    RenderSegment {
        project_root: PathBuf,
        from: String,
        to: String,
    },
    Restore {
        snapshot_id: String,
    },
    Help,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    match parse_command(&args) {
        Ok(command) => run(command),
        Err(error) => {
            eprintln!("{error}");
            print_help();
            ExitCode::from(1)
        }
    }
}

fn parse_command(args: &[String]) -> Result<SidecarCommand, String> {
    match args {
        [_] => Ok(SidecarCommand::Help),
        [_, command] if command == "-h" || command == "--help" => Ok(SidecarCommand::Help),
        [_, command] if command == "health" => Ok(SidecarCommand::Health),
        [_, command, project_root] if command == "watch" => Ok(SidecarCommand::Watch {
            project_root: PathBuf::from(project_root),
            daemon_mode: false,
        }),
        [_, command, project_root, daemon_flag]
            if command == "watch" && daemon_flag == "--daemon" =>
        {
            Ok(SidecarCommand::Watch {
                project_root: PathBuf::from(project_root),
                daemon_mode: true,
            })
        }
        [_, command, project_root] if command == "ensure-daemon" => {
            Ok(SidecarCommand::EnsureDaemon {
                project_root: PathBuf::from(project_root),
            })
        }
        [_, command, project_root] if command == "status" => Ok(SidecarCommand::Status {
            project_root: PathBuf::from(project_root),
        }),
        [_, command, project_root] if command == "view-root" => Ok(SidecarCommand::ViewRoot {
            project_root: PathBuf::from(project_root),
        }),
        [_, command, scope, project_root]
            if command == "render-markdown" && scope == "current-hour" =>
        {
            Ok(SidecarCommand::RenderCurrentHour {
                project_root: PathBuf::from(project_root),
            })
        }
        [_, command, scope, project_root]
            if command == "render-markdown" && scope == "previous-hour" =>
        {
            Ok(SidecarCommand::RenderPreviousHour {
                project_root: PathBuf::from(project_root),
            })
        }
        [_, command, scope, project_root, hour_flag, hour]
            if command == "render-markdown" && scope == "hour" && hour_flag == "--hour" =>
        {
            Ok(SidecarCommand::RenderHour {
                project_root: PathBuf::from(project_root),
                hour: hour.to_string(),
            })
        }
        [_, command, scope, project_root, from_flag, from, to_flag, to]
            if command == "render-markdown"
                && scope == "segment"
                && from_flag == "--from"
                && to_flag == "--to" =>
        {
            Ok(SidecarCommand::RenderSegment {
                project_root: PathBuf::from(project_root),
                from: from.to_string(),
                to: to.to_string(),
            })
        }
        [_, command, snapshot_id] if command == "restore" => Ok(SidecarCommand::Restore {
            snapshot_id: snapshot_id.to_string(),
        }),
        _ => Err("unknown or incomplete command".to_string()),
    }
}

fn run(command: SidecarCommand) -> ExitCode {
    match command {
        SidecarCommand::Health => {
            print_json(&runtime::health_value());
            ExitCode::SUCCESS
        }
        SidecarCommand::Watch {
            project_root,
            daemon_mode: _,
        } => match runtime::watch(&project_root) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(1)
            }
        },
        SidecarCommand::EnsureDaemon { project_root } => {
            match runtime::ensure_daemon(&project_root) {
                Ok(value) => {
                    print_json(&value);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::from(1)
                }
            }
        }
        SidecarCommand::Status { project_root } => match runtime::status(&project_root) {
            Ok(value) => {
                print_json(&value);
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(1)
            }
        },
        SidecarCommand::ViewRoot { project_root } => match runtime::view_root(&project_root) {
            Ok(value) => {
                print_json(&value);
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(1)
            }
        },
        SidecarCommand::RenderCurrentHour { project_root } => {
            match runtime::render_current_hour_markdown(&project_root) {
                Ok(value) => {
                    print_json(&value);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::from(1)
                }
            }
        }
        SidecarCommand::RenderPreviousHour { project_root } => {
            match runtime::render_previous_hour_markdown(&project_root) {
                Ok(value) => {
                    print_json(&value);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::from(1)
                }
            }
        }
        SidecarCommand::RenderHour { project_root, hour } => {
            match runtime::render_hour_markdown(&project_root, &hour) {
                Ok(value) => {
                    print_json(&value);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::from(1)
                }
            }
        }
        SidecarCommand::RenderSegment {
            project_root,
            from,
            to,
        } => match runtime::render_segment_markdown(&project_root, &from, &to) {
            Ok(value) => {
                print_json(&value);
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(1)
            }
        },
        SidecarCommand::Restore { snapshot_id } => match runtime::restore_snapshot(&snapshot_id) {
            Ok(value) => {
                print_json(&value);
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(1)
            }
        },
        SidecarCommand::Help => {
            print_help();
            ExitCode::SUCCESS
        }
    }
}

fn print_json(value: &serde_json::Value) {
    match serde_json::to_string_pretty(value) {
        Ok(rendered) => println!("{rendered}"),
        Err(error) => eprintln!("failed to render JSON output: {error}"),
    }
}

fn print_help() {
    println!(
        "\
local-history-sidecar

Usage:
  local-history-sidecar health
  local-history-sidecar watch <project-root>
  local-history-sidecar ensure-daemon <project-root>
  local-history-sidecar status <project-root>
  local-history-sidecar view-root <project-root>
  local-history-sidecar render-markdown current-hour <project-root>
  local-history-sidecar render-markdown previous-hour <project-root>
  local-history-sidecar render-markdown hour <project-root> --hour <ISO-hour>
  local-history-sidecar render-markdown segment <project-root> --from <ISO-datetime> --to <ISO-datetime>
  local-history-sidecar restore <snapshot-id>
"
    );
}

#[cfg(test)]
mod tests {
    use super::{parse_command, SidecarCommand};
    use std::path::PathBuf;

    #[test]
    fn parses_watch_command() {
        let args = vec![
            "local-history-sidecar".to_string(),
            "watch".to_string(),
            ".".to_string(),
        ];

        assert_eq!(
            parse_command(&args).expect("watch command must parse"),
            SidecarCommand::Watch {
                project_root: PathBuf::from("."),
                daemon_mode: false,
            }
        );
    }

    #[test]
    fn parses_watch_daemon_command() {
        let args = vec![
            "local-history-sidecar".to_string(),
            "watch".to_string(),
            ".".to_string(),
            "--daemon".to_string(),
        ];

        assert_eq!(
            parse_command(&args).expect("watch daemon command must parse"),
            SidecarCommand::Watch {
                project_root: PathBuf::from("."),
                daemon_mode: true,
            }
        );
    }

    #[test]
    fn parses_status_command() {
        let args = vec![
            "local-history-sidecar".to_string(),
            "status".to_string(),
            ".".to_string(),
        ];

        assert_eq!(
            parse_command(&args).expect("status command must parse"),
            SidecarCommand::Status {
                project_root: PathBuf::from("."),
            }
        );
    }

    #[test]
    fn parses_render_current_hour_command() {
        let args = vec![
            "local-history-sidecar".to_string(),
            "render-markdown".to_string(),
            "current-hour".to_string(),
            ".".to_string(),
        ];

        assert_eq!(
            parse_command(&args).expect("render current hour command must parse"),
            SidecarCommand::RenderCurrentHour {
                project_root: PathBuf::from("."),
            }
        );
    }

    #[test]
    fn parses_render_hour_command() {
        let args = vec![
            "local-history-sidecar".to_string(),
            "render-markdown".to_string(),
            "hour".to_string(),
            ".".to_string(),
            "--hour".to_string(),
            "2026-05-02T14".to_string(),
        ];

        assert_eq!(
            parse_command(&args).expect("render hour command must parse"),
            SidecarCommand::RenderHour {
                project_root: PathBuf::from("."),
                hour: "2026-05-02T14".to_string(),
            }
        );
    }

    #[test]
    fn parses_restore_command() {
        let args = vec![
            "local-history-sidecar".to_string(),
            "restore".to_string(),
            "snap_123".to_string(),
        ];

        assert_eq!(
            parse_command(&args).expect("restore command must parse"),
            SidecarCommand::Restore {
                snapshot_id: "snap_123".to_string(),
            }
        );
    }
}
