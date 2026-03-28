use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("read local addr").port()
}

fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(30));
    }
    false
}

fn spawn_api_server(port: u16) -> Child {
    let exe = std::env::var("CARGO_BIN_EXE_viralclip-swarm").expect("binary path");
    Command::new(exe)
        .arg("--api")
        .arg("--api-bind")
        .arg(format!("127.0.0.1:{port}"))
        .env("VIRALCLIP_API_KEY", "test-key")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn api server")
}

fn send_raw_request(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to server");
    stream.write_all(request.as_bytes()).expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn ensure_input_fixture() -> PathBuf {
    let input_dir = Path::new("input");
    if !input_dir.exists() {
        fs::create_dir_all(input_dir).expect("create input dir");
    }
    let path = input_dir.join("api_test.mp4");
    fs::write(&path, b"dummy").expect("write dummy input file");
    path
}

fn cleanup_input_fixture(path: &Path) {
    let _ = fs::remove_file(path);
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn api_rejects_missing_content_length() {
    let port = find_free_port();
    let mut child = spawn_api_server(port);
    assert!(wait_for_port(port, Duration::from_secs(2)));

    let request = concat!(
        "POST /run HTTP/1.1\r\n",
        "Host: 127.0.0.1\r\n",
        "Content-Type: application/json\r\n",
        "x-api-key: test-key\r\n",
        "\r\n"
    );
    let response = send_raw_request(port, request);
    stop_child(&mut child);

    assert!(
        response.starts_with("HTTP/1.1 411"),
        "expected 411 response, got: {response}"
    );
}

#[test]
fn health_endpoint_is_available_without_auth() {
    let port = find_free_port();
    let mut child = spawn_api_server(port);
    assert!(wait_for_port(port, Duration::from_secs(2)));

    let request = concat!("GET /health HTTP/1.1\r\n", "Host: 127.0.0.1\r\n", "\r\n");
    let response = send_raw_request(port, request);
    stop_child(&mut child);

    assert!(
        response.starts_with("HTTP/1.1 200"),
        "expected 200 response, got: {response}"
    );
    assert!(
        response.contains("Cache-Control: no-store"),
        "expected no-store header, got: {response}"
    );
    assert!(
        response.contains("X-Content-Type-Options: nosniff"),
        "expected nosniff header, got: {response}"
    );
}

#[test]
fn api_rejects_invalid_json() {
    let port = find_free_port();
    let mut child = spawn_api_server(port);
    assert!(wait_for_port(port, Duration::from_secs(2)));

    let body = "{";
    let request = format!(
        "POST /run HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nx-api-key: test-key\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let response = send_raw_request(port, &request);
    stop_child(&mut child);

    assert!(
        response.starts_with("HTTP/1.1 400"),
        "expected 400 response, got: {response}"
    );
}

#[test]
fn api_rejects_output_path_traversal() {
    let input_path = ensure_input_fixture();
    let port = find_free_port();
    let mut child = spawn_api_server(port);
    assert!(wait_for_port(port, Duration::from_secs(2)));

    let body = format!(
        "{{\"input\":\"{}\",\"output_dir\":\"../escape\",\"num_clips\":1}}",
        input_path.to_string_lossy().replace('\\', "/")
    );
    let request = format!(
        "POST /run HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nx-api-key: test-key\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let response = send_raw_request(port, &request);
    stop_child(&mut child);
    cleanup_input_fixture(&input_path);

    assert!(
        response.starts_with("HTTP/1.1 400"),
        "expected 400 response, got: {response}"
    );
}
