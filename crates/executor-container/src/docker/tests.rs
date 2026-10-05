use super::*;
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
};

async fn server(
    response_to_post: bool,
) -> (
    tempfile::TempDir,
    Docker,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut byte = [0];
                if stream.read_exact(&mut byte).await.is_err() {
                    break;
                }
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8(bytes).unwrap();
            let line = header.lines().next().unwrap().to_owned();
            seen.lock().unwrap().push(line.clone());
            if line.starts_with("POST") {
                if response_to_post {
                    stream
                        .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                        .await
                        .unwrap();
                }
                continue;
            }
            let body=if line.contains("/version ") { serde_json::json!({"ApiVersion":"1.41","MinAPIVersion":"1.24"}) } else {
                serde_json::json!({"Id":"a".repeat(64),"Image":format!("sha256:{}","b".repeat(64)),"State":{"Running":true,"StartedAt":"2026-10-05T07:00:00Z"},"Config":{"Env":["SECRET=private"]}})
            }.to_string();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    (dir, Docker::new(socket), calls, task)
}
#[tokio::test]
async fn negotiated_full_id_inspection_is_selected_and_post_has_fixed_grace_without_retries() {
    let (_dir, engine, calls, task) = server(true).await;
    assert!(engine.inspect("../escape").await.is_err());
    assert!(calls.lock().unwrap().is_empty());
    let selected = engine.inspect(&"a".repeat(64)).await.unwrap();
    assert!(!serde_json::to_string(&selected).unwrap().contains("SECRET"));
    engine.restart(&"a".repeat(64)).await.unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls.iter().filter(|c| c.starts_with("POST")).count(), 1);
    assert_eq!(
        calls[3],
        format!(
            "POST /v1.41/containers/{}/restart?t=10 HTTP/1.1",
            "a".repeat(64)
        )
    );
    task.abort();
}
#[tokio::test]
async fn lost_post_response_is_an_error_with_exactly_one_post() {
    let (_dir, engine, calls, task) = server(false).await;
    assert!(engine.restart(&"a".repeat(64)).await.is_err());
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with("POST"))
            .count(),
        1
    );
    task.abort();
}
#[tokio::test]
async fn start_and_stop_use_only_the_fixed_selected_endpoints() {
    let (_dir, engine, calls, task) = server(true).await;
    engine.start(&"a".repeat(64)).await.unwrap();
    engine.stop(&"a".repeat(64)).await.unwrap();
    let posts: Vec<_> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.starts_with("POST"))
        .cloned()
        .collect();
    assert_eq!(
        posts,
        [
            format!("POST /v1.41/containers/{}/start HTTP/1.1", "a".repeat(64)),
            format!(
                "POST /v1.41/containers/{}/stop?t=10 HTTP/1.1",
                "a".repeat(64)
            )
        ]
    );
    task.abort();
}
