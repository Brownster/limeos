use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU16, Ordering},
};
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
    let post = Arc::new(AtomicU16::new(if response_to_post { 204 } else { 0 }));
    scripted(post, Arc::new(AtomicU16::new(200))).await
}
/// The Engine's reply status to each POST (0: no reply) and to each container
/// inspection, read per request so a test can change them between calls.
async fn scripted(
    post: Arc<AtomicU16>,
    inspect: Arc<AtomicU16>,
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
            let status = if line.starts_with("POST") {
                post.load(Ordering::SeqCst)
            } else if line.contains("/version ") {
                200
            } else {
                inspect.load(Ordering::SeqCst)
            };
            let response = match status {
                0 => continue,
                204 => "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".to_owned(),
                304 => "HTTP/1.1 304 Not Modified\r\n\r\n".to_owned(),
                200 => {
                    let body=if line.contains("/version ") { serde_json::json!({"ApiVersion":"1.41","MinAPIVersion":"1.24"}) } else {
                        serde_json::json!({"Id":"a".repeat(64),"Image":format!("sha256:{}","b".repeat(64)),"State":{"Running":true,"StartedAt":"2026-10-05T07:00:00Z"},"Config":{"Env":["SECRET=private"]}})
                    }.to_string();
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    )
                }
                other => {
                    let body = r#"{"message":"fixture"}"#;
                    format!(
                        "HTTP/1.1 {other} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                }
            };
            stream.write_all(response.as_bytes()).await.unwrap();
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
#[tokio::test]
async fn effect_statuses_from_the_engine_map_to_defined_results_with_one_post_each() {
    use ContainerAction::{Restart, Start, Stop};
    use ErrorCode::{Conflict, NotFound, Unavailable};
    let id = "a".repeat(64);
    let post = Arc::new(AtomicU16::new(0));
    let (_dir, engine, calls, task) = scripted(post.clone(), Arc::new(AtomicU16::new(200))).await;
    let cases = [
        (204, Start, Ok(())),
        (204, Stop, Ok(())),
        (204, Restart, Ok(())),
        // Already running (start) or already stopped (stop): the intended
        // state holds; the separate fresh verification still decides.
        (304, Start, Ok(())),
        (304, Stop, Ok(())),
        (304, Restart, Err(Unavailable)),
        (404, Start, Err(NotFound)),
        (404, Stop, Err(NotFound)),
        (404, Restart, Err(NotFound)),
        (409, Start, Err(Conflict)),
        (409, Stop, Err(Conflict)),
        (409, Restart, Err(Conflict)),
        (500, Start, Err(Unavailable)),
        (500, Stop, Err(Unavailable)),
        (500, Restart, Err(Unavailable)),
    ];
    for (status, operation, expected) in cases {
        post.store(status, Ordering::SeqCst);
        let result = match operation {
            Restart => engine.restart(&id).await,
            Start => engine.start(&id).await,
            Stop => engine.stop(&id).await,
        };
        assert_eq!(result.map_err(|e| e.0), expected, "{status} {operation:?}");
    }
    let posts = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.starts_with("POST"))
        .count();
    assert_eq!(posts, cases.len());
    task.abort();
}
#[tokio::test]
async fn container_inspection_statuses_map_to_defined_errors() {
    let id = "a".repeat(64);
    let inspect = Arc::new(AtomicU16::new(200));
    let (_dir, engine, _calls, task) = scripted(Arc::new(AtomicU16::new(0)), inspect.clone()).await;
    for (status, expected) in [
        (404, ErrorCode::NotFound),
        (409, ErrorCode::Conflict),
        (500, ErrorCode::Unavailable),
        (204, ErrorCode::Unavailable),
    ] {
        inspect.store(status, Ordering::SeqCst);
        assert_eq!(
            engine.inspect(&id).await.unwrap_err().0,
            expected,
            "{status}"
        );
    }
    task.abort();
}
#[tokio::test]
async fn a_stuck_job_whose_container_was_removed_can_be_resolved() {
    use limeos_domain::{DeclaredOutcome, ResolutionEvidence, ResolveKind};
    use std::os::unix::fs::PermissionsExt;
    let id = "a".repeat(64);
    // The restart's response is an Engine error: the outcome is unknown.
    let inspect = Arc::new(AtomicU16::new(200));
    let (_dir, engine, calls, task) =
        scripted(Arc::new(AtomicU16::new(500)), inspect.clone()).await;
    let receipts = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let mut store = crate::ReceiptStore::open(receipts.path()).unwrap();
    let ceiling = crate::ContainerCeiling {
        version: 1,
        core_uid: 1001,
        allow_restart: true,
        allow_start: false,
        allow_stop: false,
        allow_stack: false,
        managed_containers: vec![id.clone()],
        managed_stacks: vec![],
    };
    let expected = engine.inspect(&id).await.unwrap();
    let plan = limeos_domain::RestartPlan {
        operation: ContainerAction::Restart,
        id: "c".repeat(64),
        version: 1,
        principal: "alice".into(),
        grant_revision: 1,
        expected: expected.clone(),
        created_at: 200,
        expires_at: 500,
    };
    let request = crate::RestartRequest {
        action: "d".repeat(64),
        plan_digest: limeos_identity::digest(&crate::encode(&plan).unwrap()),
        plan,
    };
    let receipt = store
        .execute(&engine, &ceiling, 1001, &request, || 201)
        .await
        .unwrap();
    assert_eq!(receipt.state, crate::ExecutionState::OutcomeUnknown);
    // The operator removes the container (`docker rm`); the Engine says 404.
    inspect.store(404, Ordering::SeqCst);
    let evidence =
        ResolutionEvidence::observe(ContainerAction::Restart, &[&expected], &[None]).unwrap();
    let confirmed = store
        .resolve_action(
            &engine,
            &ceiling,
            1001,
            ResolveKind::Container,
            &request.action,
            &request.plan_digest,
            &evidence.digest().unwrap(),
            DeclaredOutcome::AcceptedDivergence,
            &"9".repeat(64),
            &limeos_domain::ActionPlan::Container(request.plan.clone()),
            false,
            201,
        )
        .await
        .unwrap();
    let crate::ResolveOutcome::Resolved(confirmed) = confirmed else {
        panic!("not resolved: {confirmed:?}");
    };
    assert_eq!(confirmed.action, request.action);
    let posts = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.starts_with("POST"))
        .count();
    assert_eq!(posts, 1);
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

/// Serves one container's inspection and a fixed logs body, written in small
/// pieces so frames and lines cross the reads the client sees.
async fn logs_server(tty: bool, body: Vec<u8>) -> (tempfile::TempDir, Docker) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("engine.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
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
            let payload = if line.contains("/version ") {
                serde_json::json!({"ApiVersion":"1.41","MinAPIVersion":"1.24"})
                    .to_string()
                    .into_bytes()
            } else if line.contains("/logs?") {
                body.clone()
            } else {
                serde_json::json!({"Id":"a".repeat(64),"Config":{"Tty":tty}})
                    .to_string()
                    .into_bytes()
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                payload.len()
            );
            if stream.write_all(head.as_bytes()).await.is_err() {
                continue;
            }
            for piece in payload.chunks(1021) {
                if stream.write_all(piece).await.is_err() {
                    break;
                }
                let _ = stream.flush().await;
                tokio::task::yield_now().await;
            }
        }
    });
    (dir, Docker::new(socket))
}
fn frame(stream: u8, text: &[u8]) -> Vec<u8> {
    let mut raw = vec![stream, 0, 0, 0];
    raw.extend_from_slice(&(text.len() as u32).to_be_bytes());
    raw.extend_from_slice(text);
    raw
}
fn log_line(i: usize) -> String {
    format!(
        "2026-11-19T00:00:00.{i:09}Z line {i:03} {}\n",
        "word ".repeat(112)
    )
}
async fn read_logs(tty: bool, body: Vec<u8>) -> limeos_domain::ContainerLogs {
    let (_dir, engine) = logs_server(tty, body).await;
    engine
        .logs(
            &format!("container:{}", "a".repeat(64)),
            limeos_domain::LogOptions { tail: 200 },
        )
        .await
        .unwrap()
}
fn assert_newest_lines(value: &limeos_domain::ContainerLogs) {
    assert!(value.truncated);
    assert!(value.text.len() <= limeos_domain::CONTAINER_LOG_BYTES);
    assert!(
        value.text.ends_with(&log_line(199)),
        "{}",
        &value.text[..80]
    );
    // Whole, consecutive lines up to the newest one: a line-boundary start.
    let lines: Vec<_> = value.text.split_inclusive('\n').collect();
    assert!(lines.len() > 30);
    let first = 200 - lines.len();
    for (offset, line) in lines.iter().enumerate() {
        assert_eq!(*line, log_line(first + offset));
    }
}
#[tokio::test]
async fn logs_over_the_bounds_return_the_newest_whole_lines_from_multiplexed_streams() {
    // About 120 KiB of 200 lines, one frame per line on alternating streams.
    let body = (0..200)
        .flat_map(|i| frame(1 + (i % 2) as u8, log_line(i).as_bytes()))
        .collect();
    assert_newest_lines(&read_logs(false, body).await);
    // Frames larger than one line, with lines crossing frame boundaries.
    let text: Vec<u8> = (0..200).flat_map(|i| log_line(i).into_bytes()).collect();
    let body = text.chunks(4093).flat_map(|c| frame(1, c)).collect();
    assert_newest_lines(&read_logs(false, body).await);
}
#[tokio::test]
async fn logs_over_the_bounds_return_the_newest_whole_lines_from_a_terminal() {
    let body = (0..200).flat_map(|i| log_line(i).into_bytes()).collect();
    assert_newest_lines(&read_logs(true, body).await);
}
#[tokio::test]
async fn a_single_line_private_key_is_redacted_and_later_lines_are_kept() {
    let body = frame(
        1,
        b"before\nkey=-----BEGIN PRIVATE KEY-----MIIEvQ-----END PRIVATE KEY-----\nafter\n",
    );
    let value = read_logs(false, body).await;
    assert_eq!(value.text, "before\n[private key redacted]\nafter\n");
    assert!(!value.truncated);
}
#[tokio::test]
async fn a_line_whose_start_was_dropped_is_never_returned_in_part() {
    // The credential marker is in the evicted start of one unterminated line.
    let line = format!("password={}", "a".repeat(70_000));
    for (tty, body) in [
        (true, line.clone().into_bytes()),
        (false, frame(1, line.as_bytes())),
        (
            false,
            line.as_bytes()
                .chunks(4093)
                .flat_map(|c| frame(2, c))
                .collect(),
        ),
    ] {
        let value = read_logs(tty, body).await;
        assert_eq!(value.text, "", "{tty}");
        assert!(value.truncated);
    }
    // A whole line longer than the text bound is named, never cut.
    let mut body = log_line(0).into_bytes();
    body.extend(format!("{}\n", "b".repeat(30_000)).into_bytes());
    let value = read_logs(true, body).await;
    assert_eq!(
        value.text,
        format!("{}[line over the size limit omitted]\n", log_line(0))
    );
    assert!(value.truncated);
}
#[tokio::test]
async fn logs_for_a_removed_container_are_not_found() {
    let (_dir, engine, calls, task) =
        scripted(Arc::new(AtomicU16::new(0)), Arc::new(AtomicU16::new(404))).await;
    let error = engine
        .logs(
            &format!("container:{}", "a".repeat(64)),
            limeos_domain::LogOptions { tail: 200 },
        )
        .await
        .unwrap_err();
    assert_eq!(error.0, ErrorCode::NotFound);
    assert!(calls.lock().unwrap().iter().all(|c| !c.contains("/logs")));
    task.abort();
}
