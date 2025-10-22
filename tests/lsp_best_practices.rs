use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use std::thread::sleep;

use nix::sys::signal;
use nix::unistd::Pid;

use tower_lsp::lsp_types::DiagnosticSeverity;

use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};

#[tokio::test]
async fn test_server_terminates_on_client_death() -> io::Result<()> {
    // Ensure the server and dummy client binaries exist
    let server_path = env!("CARGO_BIN_EXE_rholang-language-server");
    let client_path = env!("CARGO_BIN_EXE_dummy_client");
    assert!(
        Path::new(server_path).exists(),
        "Server binary not found. Run `cargo build` first."
    );
    assert!(
        Path::new(client_path).exists(),
        "Dummy client binary not found. Run `cargo build` first."
    );

    // Spawn the dummy client process
    let mut client_process = Command::new(client_path)
        .env("CARGO_BIN_EXE_rholang-language-server", server_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn dummy client");

    // Capture client stdout to get the server PID
    let client_stdout = client_process.stdout.take().expect("Failed to capture client stdout");
    let mut stdout_reader = BufReader::new(client_stdout);
    let mut stdout_buffer = String::new();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let mut line = String::new();
        if stdout_reader.read_line(&mut line)? == 0 {
            break;
        }
        stdout_buffer.push_str(&line);
        if line.trim().parse::<i32>().is_ok() {
            break;
        }
    }
    if stdout_buffer.is_empty() {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "Failed to read server PID within 5 seconds"));
    }

    // Parse the server PID from stdout
    let server_pid: i32 = stdout_buffer
        .lines()
        .find_map(|line| line.trim().parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "No valid server PID found in output"))?;
    eprintln!("Language server PID: {}", server_pid);

    // Capture client stderr for debugging
    let client_stderr = client_process.stderr.take().expect("Failed to capture client stderr");
    std::thread::spawn(move || {
        let mut reader = BufReader::new(client_stderr);
        let mut buffer = String::new();
        if reader.read_to_string(&mut buffer).is_ok() {
            eprint!("Client stderr: {}", buffer);
        }
    });

    // Terminate the client process
    eprintln!("Terminating dummy client");
    client_process.wait()?;

    // Monitor the server PID to ensure it terminates
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        match signal::kill(Pid::from_raw(server_pid), None) {
            Ok(()) => sleep(Duration::from_millis(100)), // Server still running
            Err(nix::errno::Errno::ESRCH) => break,     // Server process no longer exists
            Err(e) => return Err(io::Error::new(io::ErrorKind::Other, format!("Failed to check server PID: {}", e))),
        }
    }
    if signal::kill(Pid::from_raw(server_pid), None).is_ok() {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "Server did not terminate within 15 seconds"));
    }

    // Print success message
    println!("test_server_terminates_on_client_death passed successfully");
    Ok(())
}

fn run_diagnostic_test(client: &LspClient) {
    let doc = client.open_document("/path/to/invalid.rho", "new x in { x!(\"Hello\") ").expect("Failed to open document");
    let diagnostic_params = client.await_diagnostics(&doc).unwrap();
    assert_eq!(diagnostic_params.uri.to_string(), doc.uri());
    assert_eq!(diagnostic_params.diagnostics.len(), 1);
    let diagnostic = &diagnostic_params.diagnostics[0];
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
    assert!(diagnostic.message.contains("Missing token: }"), "Expected message to contain 'Missing token: }}', but got: {}", diagnostic.message);
    assert_eq!(diagnostic.range.start.line, 0);
    assert_eq!(diagnostic.range.start.character, 22);  // Position of missing brace
}

with_lsp_client!(test_diagnostic_stdio, CommType::Stdio, |client: &LspClient| {
    run_diagnostic_test(client);
});

with_lsp_client!(test_diagnostic_tcp, CommType::Tcp { port: None }, |client: &LspClient| {
    run_diagnostic_test(client);
});

with_lsp_client!(test_diagnostic_pipe, CommType::Pipe { path: None }, |client: &LspClient| {
    run_diagnostic_test(client);
});

with_lsp_client!(test_diagnostic_websocket, CommType::WebSocket { port: None }, |client: &LspClient| {
    run_diagnostic_test(client);
});
