use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Regression guard for Zed MCP startup: newline-delimited JSON initialize must respond promptly.
#[test]
fn stdio_initialize_handshake_matches_zed_client() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_local-history-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("local-history-mcp binary must spawn");

    {
        let stdin = child.stdin.as_mut().expect("stdin must be available");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"zed","version":"1.4.4"}}}}}}"#
        )
        .expect("initialize request must be writable");
    }

    let stdout = child.stdout.take().expect("stdout must be available");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();

    let deadline = Instant::now() + Duration::from_secs(5);
    while line.is_empty() && Instant::now() < deadline {
        if reader
            .read_line(&mut line)
            .expect("stdout must be readable")
            == 0
        {
            break;
        }
        if line.trim().is_empty() {
            line.clear();
        }
    }

    assert!(
        !line.is_empty(),
        "MCP must answer initialize on stdout within 5 seconds (Zed times out after 60s)"
    );

    let response: serde_json::Value =
        serde_json::from_str(line.trim()).expect("initialize response must be valid JSON");

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert!(
        response["result"]["protocolVersion"].is_string(),
        "initialize must advertise protocolVersion"
    );
    assert!(
        response["result"]["capabilities"]["tools"].is_object(),
        "initialize must advertise tools capability"
    );
    assert_eq!(
        response["result"]["serverInfo"]["name"],
        "local-history-mcp"
    );

    child.kill().ok();
    child.wait().ok();
}

#[test]
fn version_flag_prints_package_version_to_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_local-history-mcp"))
        .arg("--version")
        .output()
        .expect("local-history-mcp --version must run");

    assert!(
        output.status.success(),
        "local-history-mcp --version must exit successfully"
    );

    let version = String::from_utf8(output.stdout).expect("version output must be UTF-8");
    assert_eq!(
        version.trim(),
        env!("CARGO_PKG_VERSION"),
        "extension compatibility probes rely on --version stdout"
    );
}
