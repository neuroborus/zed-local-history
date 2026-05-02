use std::env;
use std::path::Path;
use std::process::ExitCode;

use local_history_core::placeholder_project_id;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    match args.as_slice() {
        [_] => {
            print_help();
            ExitCode::SUCCESS
        }
        [_, command] if command == "-h" || command == "--help" => {
            print_help();
            ExitCode::SUCCESS
        }
        [_, command] if command == "health" => {
            println!("{{\"status\":\"ok\",\"mode\":\"bootstrap\"}}");
            ExitCode::SUCCESS
        }
        [_, command, project_root] if command == "watch" => {
            let project_id = placeholder_project_id(Path::new(project_root));
            println!(
                "watch bootstrap: project_root={project_root} project_id={project_id} mode=placeholder"
            );
            ExitCode::SUCCESS
        }
        [_, command, project_root] if command == "ensure-daemon" => {
            let project_id = placeholder_project_id(Path::new(project_root));
            println!(
                "ensure-daemon bootstrap: project_root={project_root} project_id={project_id}"
            );
            ExitCode::SUCCESS
        }
        [_, command, project_root] if command == "status" => {
            let project_id = placeholder_project_id(Path::new(project_root));
            println!("status bootstrap: project_root={project_root} project_id={project_id}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("unknown or incomplete command");
            print_help();
            ExitCode::from(1)
        }
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
