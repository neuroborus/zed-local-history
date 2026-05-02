use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use local_history_core::{default_data_dir, project_id_for_root, StorageLayout};

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
        [_, command, project_root] if command == "status" => {
            print_status(Path::new(project_root));
            ExitCode::SUCCESS
        }
        [_, command, project_root] if command == "view-root" => {
            let layout = layout_for(Path::new(project_root));
            println!("{}", layout.view_dir.display());
            ExitCode::SUCCESS
        }
        [_, command, project_root] if command == "recent" => {
            let project_id = project_id_for_root(Path::new(project_root));
            println!(
                "recent bootstrap: project_root={project_root} project_id={project_id} limit=10"
            );
            ExitCode::SUCCESS
        }
        [_, command, project_root] if command == "list" => {
            let project_id = project_id_for_root(Path::new(project_root));
            println!(
                "list bootstrap: project_root={project_root} project_id={project_id} page=1 page_size=20"
            );
            ExitCode::SUCCESS
        }
        [_, command, snapshot_id] if command == "restore" => {
            println!("restore bootstrap: snapshot_id={snapshot_id}");
            ExitCode::SUCCESS
        }
        [_, command, scope, project_root] if command == "render-markdown" => {
            let layout = layout_for(Path::new(project_root));
            let target = layout
                .view_dir
                .join(PathBuf::from(scope))
                .with_extension("md");
            println!("{}", target.display());
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("unknown or incomplete command");
            print_help();
            ExitCode::from(1)
        }
    }
}

fn layout_for(project_root: &Path) -> StorageLayout {
    let project_id = project_id_for_root(project_root);
    StorageLayout::for_project(default_data_dir(), project_id)
}

fn print_status(project_root: &Path) {
    let project_id = project_id_for_root(project_root);
    let layout = StorageLayout::for_project(default_data_dir(), project_id.clone());

    println!("project_root={}", project_root.display());
    println!("project_id={project_id}");
    println!("data_dir={}", layout.project_dir.display());
    println!("database={}", layout.database_path.display());
    println!("view_root={}", layout.view_dir.display());
}

fn print_help() {
    println!(
        "\
local-history

Usage:
  local-history status <project-root>
  local-history view-root <project-root>
  local-history recent <project-root>
  local-history list <project-root>
  local-history restore <snapshot-id>
  local-history render-markdown <scope> <project-root>
"
    );
}
