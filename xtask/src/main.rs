use std::error::Error;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

type TaskResult<T> = Result<T, Box<dyn Error>>;

fn main() -> TaskResult<()> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "help".to_string());

    match command.as_str() {
        "fmt" => run_checked("cargo", &["fmt", "--all", "--check"])?,
        "clippy" => run_checked(
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        )?,
        "test" => run_checked("cargo", &["test", "--workspace"])?,
        "build" => run_checked("cargo", &["build", "--workspace"])?,
        "ci" => {
            run_checked("cargo", &["fmt", "--all", "--check"])?;
            run_checked(
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
            )?;
            run_checked("cargo", &["test", "--workspace"])?;
            run_checked("cargo", &["build", "--workspace"])?;
        }
        "zed-fmt" => run_zed_checked(&["fmt", "--all", "--check"])?,
        "zed-clippy" => run_zed_checked(&[
            "clippy",
            "--target",
            "wasm32-wasip2",
            "--",
            "-D",
            "warnings",
        ])?,
        "zed-check" => run_zed_checked(&["check", "--target", "wasm32-wasip2"])?,
        "zed-ci" => {
            run_zed_checked(&["fmt", "--all", "--check"])?;
            run_zed_checked(&[
                "clippy",
                "--target",
                "wasm32-wasip2",
                "--",
                "-D",
                "warnings",
            ])?;
            run_zed_checked(&["check", "--target", "wasm32-wasip2"])?;
            run_zed_checked(&["test"])?;
        }
        "full-ci" => {
            run_checked("cargo", &["fmt", "--all", "--check"])?;
            run_checked(
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
            )?;
            run_checked("cargo", &["test", "--workspace"])?;
            run_checked("cargo", &["build", "--workspace"])?;
            run_zed_checked(&["fmt", "--all", "--check"])?;
            run_zed_checked(&[
                "clippy",
                "--target",
                "wasm32-wasip2",
                "--",
                "-D",
                "warnings",
            ])?;
            run_zed_checked(&["check", "--target", "wasm32-wasip2"])?;
            run_zed_checked(&["test"])?;
        }
        _ => print_help(),
    }

    Ok(())
}

fn run_checked(program: &str, args: &[&str]) -> TaskResult<()> {
    let status = Command::new(program).args(args).status()?;

    check_status(program, args, status)
}

fn run_zed_checked(args: &[&str]) -> TaskResult<()> {
    let cargo = zed_cargo_path()?;
    let status = Command::new(&cargo)
        .current_dir(workspace_root().join("editors").join("zed"))
        // `cargo run -p xtask -- ...` inherits the root workspace rustup override.
        // Clear it so the nested cargo process can honor `editors/zed/rust-toolchain.toml`.
        .env_remove("RUSTUP_TOOLCHAIN")
        .args(args)
        .status()?;

    check_status(cargo.to_string_lossy().as_ref(), args, status)
}

fn check_status(program: &str, args: &[&str], status: ExitStatus) -> TaskResult<()> {
    if status.success() {
        return Ok(());
    }

    Err(format!("command failed: {} {}", program, args.join(" ")).into())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must live under the workspace root")
        .to_path_buf()
}

fn zed_cargo_path() -> TaskResult<PathBuf> {
    let path = home_dir().join(".cargo").join("bin").join("cargo");

    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "expected rustup-managed cargo at {} for editors/zed checks",
            path.display()
        )
        .into())
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn print_help() {
    println!(
        "\
xtask

Usage:
  cargo run -p xtask -- fmt
  cargo run -p xtask -- clippy
  cargo run -p xtask -- test
  cargo run -p xtask -- build
  cargo run -p xtask -- ci
  cargo run -p xtask -- zed-fmt
  cargo run -p xtask -- zed-clippy
  cargo run -p xtask -- zed-check
  cargo run -p xtask -- zed-ci
  cargo run -p xtask -- full-ci
"
    );
}
