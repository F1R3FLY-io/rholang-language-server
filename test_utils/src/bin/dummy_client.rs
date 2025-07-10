use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::anyhow;
use serde_json::{json, Value};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Get server path dynamically with fallback
    let server_path = std::env::var("CARGO_BIN_EXE_rholang-language-server")
        .or_else(|_| {
            let fallback = "./target/debug/rholang-language-server";
            if Path::new(fallback).exists() {
                Ok(fallback.to_string())
            } else {
                Err(anyhow!("CARGO_BIN_EXE_rholang-language-server not set and fallback path {} does not exist", fallback))
            }
        })?;

    // Verify server binary exists
    if !Path::new(&server_path).exists() {
        return Err(anyhow!("Server binary not found at: {}", server_path));
    }

    // Spawn the language server
    let client_pid = std::process::id();
    let mut server = Command::new(&server_path)
        .args([
            "--stdio",
            "--client-process-id", &client_pid.to_string(),
            "--log-level", "debug",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped()) // Re-enable stderr
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn server: {}", e))?;

    // Get server PID
    let server_pid = server.id() as i32;
    println!("{}", server_pid);

    // Capture server stderr for debugging
    let server_stderr = server.stderr.take().expect("Failed to capture server stderr");
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(server_stderr);
        let mut buffer = Vec::new();
        while reader.read_to_end(&mut buffer).unwrap_or(0) > 0 {
            eprint!("Server stderr: {}", String::from_utf8_lossy(&buffer));
            buffer.clear();
        }
    });

    // Initialize LSP communication
    let mut server_stdin = server.stdin.take().expect("Failed to take server stdin");
    let mut server_stdout = std::io::BufReader::new(server.stdout.take().expect("Failed to take server stdout"));

    // Send initialize request with enhanced parameters
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": client_pid,
            "rootUri": null,
            "clientInfo": {
                "name": "dummy_client",
                "version": "0.1.0"
            },
            "capabilities": {
                "textDocument": {
                    "synchronization": {
                        "dynamicRegistration": false,
                        "willSave": false,
                        "didSave": false,
                        "willSaveWaitUntil": false
                    },
                    "publishDiagnostics": {
                        "relatedInformation": true
                    }
                },
                "workspace": {
                    "workspaceFolders": false
                }
            },
            "workspaceFolders": null
        }
    });
    send_lsp_message(&mut server_stdin, &initialize)?;

    // Read initialize response with timeout
    let response = match read_lsp_message(&mut server_stdout, Duration::from_secs(5)) {
        Ok(resp) => resp,
        Err(e) => {
            eprintln!("Failed to read initialize response: {}", e);
            return Err(e);
        }
    };
    if response.get("id") != Some(&Value::Number(1.into())) {
        return Err(anyhow!("Unexpected initialize response: {:?}", response));
    }
    eprintln!("Initialize response received successfully");

    // Send initialized notification
    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    send_lsp_message(&mut server_stdin, &initialized)?;

    // Simulate brief runtime (100ms) then exit
    sleep(Duration::from_millis(100)).await;
    eprintln!("Dummy client exiting without shutting down server");

    Ok(())
}

fn send_lsp_message(stdin: &mut std::process::ChildStdin, message: &Value) -> anyhow::Result<()> {
    let message_str = serde_json::to_string(message)?;
    let header = format!("Content-Length: {}\r\n\r\n", message_str.len());
    eprintln!("SENDING:\n{}", message_str);
    stdin.write_all(header.as_bytes())?;
    stdin.write_all(message_str.as_bytes())?;
    stdin.flush()?;
    Ok(())
}

fn read_lsp_message(stdout: &mut std::io::BufReader<std::process::ChildStdout>, timeout: Duration) -> anyhow::Result<Value> {
    let start = std::time::Instant::now();
    let mut headers = String::new();
    let mut buffer = [0; 1024];

    // Read headers
    loop {
        if start.elapsed() >= timeout {
            return Err(anyhow!("Timeout reading LSP headers"));
        }
        let bytes_read = stdout.get_mut().read(&mut buffer)?;
        if bytes_read == 0 {
            return Err(anyhow!("EOF while reading headers"));
        }
        let chunk = String::from_utf8_lossy(&buffer[..bytes_read]);
        headers.push_str(&chunk);
        eprintln!("RAW HEADER CHUNK:\n{}", chunk);
        if headers.contains("\r\n\r\n") {
            break;
        }
    }

    // Parse Content-Length
    let content_length = headers
        .lines()
        .find(|line| line.starts_with("Content-Length: "))
        .and_then(|line| line["Content-Length: ".len()..].trim().parse::<usize>().ok())
        .ok_or_else(|| anyhow!("Missing or invalid Content-Length header: {}", headers))?;
    eprintln!("Parsed Content-Length: {}", content_length);

    // Read content
    let mut content = vec![0; content_length];
    let mut total_read = 0;
    while total_read < content_length {
        if start.elapsed() >= timeout {
            return Err(anyhow!("Timeout reading LSP content"));
        }
        let bytes_read = stdout.get_mut().read(&mut content[total_read..])?;
        if bytes_read == 0 {
            return Err(anyhow!("EOF while reading content"));
        }
        total_read += bytes_read;
        eprintln!("Read {} bytes of content (total: {}/{})", bytes_read, total_read, content_length);
    }

    let message: Value = serde_json::from_slice(&content)?;
    eprintln!("RECEIVING:\n{}", message);
    Ok(message)
}
