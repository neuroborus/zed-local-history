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
}
