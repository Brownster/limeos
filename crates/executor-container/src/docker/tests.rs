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

type Row = (usize, &'static str, bool, &'static str);
async fn project_server(
    rows: Vec<Row>,
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
            let line = String::from_utf8(bytes)
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned();
            seen.lock().unwrap().push(line.clone());
            let body = if line.contains("/version ") {
                serde_json::json!({"ApiVersion":"1.41","MinAPIVersion":"1.24"})
            } else if line.contains("/containers/json") {
                serde_json::Value::Array(
                    rows.iter()
                        .map(|(c, ..)| serde_json::json!({"Id": format!("{c:064x}")}))
                        .collect(),
                )
            } else {
                let (c, project, one_off, started) = rows
                    .iter()
                    .find(|(c, ..)| line.contains(&format!("{c:064x}")))
                    .unwrap();
                serde_json::json!({"Id":format!("{c:064x}"),"Image":format!("sha256:{}","b".repeat(64)),"State":{"Running":true,"StartedAt":started},"Config":{"Env":["SECRET=private"],"Labels":{"com.docker.compose.project":project,"com.docker.compose.oneoff":if *one_off {"True"} else {"False"}}}})
            }
            .to_string();
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
async fn project_members_use_only_the_project_label_exclude_one_offs_and_sort_by_id() {
    let started = "2026-10-05T07:00:00Z";
    let (_dir, engine, calls, task) = project_server(vec![
        (0xc, "media", false, started),
        (0xa, "media", false, started),
        (0xd, "media", true, started),
    ])
    .await;
    assert!(engine.project_members("../media").await.is_err());
    assert!(engine.project_members("Media").await.is_err());
    assert!(calls.lock().unwrap().is_empty());
    let members = engine.project_members("media").await.unwrap();
    let ids: Vec<_> = members
        .iter()
        .map(|m| m.resource[m.resource.len() - 1..].to_owned())
        .collect();
    assert_eq!(ids, ["a", "c"]);
    assert!(!serde_json::to_string(&members).unwrap().contains("SECRET"));
    let calls = calls.lock().unwrap();
    assert!(calls.iter().any(|line| line.starts_with(
        "GET /v1.41/containers/json?all=1&filters=%7B%22label%22%3A%5B%22com.docker.compose.project%3Dmedia%22%5D%7D "
    )));
    assert!(calls.iter().all(|line| line.starts_with("GET")));
    task.abort();
}
#[tokio::test]
async fn project_members_refuse_label_mismatch_and_oversized_projects() {
    let started = "2026-10-05T07:00:00Z";
    let (_dir, engine, _calls, task) = project_server(vec![(0xa, "other", false, started)]).await;
    assert!(engine.project_members("media").await.is_err());
    task.abort();
    let many = (0..=limeos_domain::MAX_STACK_MEMBERS)
        .map(|c| (c, "media", false, started))
        .collect::<Vec<_>>();
    let (_dir, engine, _calls, task) = project_server(many).await;
    assert!(engine.project_members("media").await.is_err());
    task.abort();
}
#[tokio::test]
async fn project_listing_bound_is_the_engine_bound_not_the_stack_bound() {
    use limeos_domain::{MAX_STACK_MEMBERS, deployment::discovery::MAX_ENGINE_CONTAINERS};
    let started = "2026-10-05T07:00:00Z";
    // Exactly the Engine bound of rows: a full 16-member stack plus 48 one-offs
    // is accepted, because one-offs are not members and rows <= Engine bound.
    let rows = (0..MAX_ENGINE_CONTAINERS)
        .map(|i| (i, "media", i >= MAX_STACK_MEMBERS, started))
        .collect::<Vec<_>>();
    let (_dir, engine, _calls, task) = project_server(rows).await;
    let members = engine.project_members("media").await.unwrap();
    assert_eq!(members.len(), MAX_STACK_MEMBERS);
    task.abort();
    // One row over the Engine bound is refused whole before any inspect.
    let rows = (0..=MAX_ENGINE_CONTAINERS)
        .map(|i| (i, "media", true, started))
        .collect::<Vec<_>>();
    let (_dir, engine, calls, task) = project_server(rows).await;
    assert!(engine.project_members("media").await.is_err());
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .all(|line| !line.contains(&format!("{:064x}", 0)))
    );
    task.abort();
}
#[tokio::test]
async fn project_members_accept_a_wybie_sized_engine_and_the_stack_bound_still_holds() {
    let started = "2026-10-05T07:00:00Z";
    // 18 rows: a 14-member media stack plus 4 one-off containers.
    let rows = (0..18)
        .map(|i| (i, "media", i >= 14, started))
        .collect::<Vec<_>>();
    let (_dir, engine, _calls, task) = project_server(rows).await;
    assert_eq!(engine.project_members("media").await.unwrap().len(), 14);
    task.abort();
    // 18 non-one-off rows exceed the per-stack bound of 16, whatever the Engine bound.
    let rows = (0..18)
        .map(|i| (i, "media", false, started))
        .collect::<Vec<_>>();
    let (_dir, engine, _calls, task) = project_server(rows).await;
    assert!(engine.project_members("media").await.is_err());
    task.abort();
}
