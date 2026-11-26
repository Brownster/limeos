//! The connection handler itself, over a socket pair: these fail if
//! authorization moves back to start-up or to the read made at accept.
//!
//! Each test skips itself when run as root, since a root test user cannot
//! be core (`core_uid` 0 is refused); CI runs them unprivileged.
use super::*;
use crate::test_root::{FILE, Root, container_ceiling_json, id};
use limeos_contracts::VERSION;
use limeos_domain::LogOptions;
use limeos_executor_protocol::{CeilingRole, Receipt};

fn me() -> u32 {
    rustix::process::geteuid().as_raw()
}

fn role(root: &Root, ceiling: CeilingRole) -> SharedRole {
    let source = match ceiling {
        CeilingRole::Container | CeilingRole::ContainerReadOnly => Source::Docker,
        _ => Source::Host,
    };
    SharedRole {
        ceilings: Arc::new(root.file(ceiling)),
        source,
        storage_reader: ceiling == CeilingRole::StorageReader,
        observations: limeos_observations::Cache::new(vec![source]),
        collecting: false,
        containers: Arc::new(ContainerRole {
            // `fake_engine` answers here when a test needs the Engine.
            engine: Docker::new(root.path("docker.sock")),
            receipts: None,
            epoch: None,
            docker: source == Source::Docker,
        }),
        storage_permit: Gate::default(),
        read_supervisor: Arc::new(Supervisor::default()),
        stacks: limeos_observations::stacks::Cache::new(),
        stack_scan: false,
        inventory_reads: Arc::new(BlockingReads::new(1)),
    }
}

/// A stand-in Engine on `root`'s Docker socket: it answers `/version`, and
/// answers every container inspection with another container's ID, which
/// the log path refuses as `Conflict`. A log request that gets `Conflict`
/// therefore passed the ceiling and reached the Engine.
fn fake_engine(root: &Root) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::UnixListener::bind(root.path("docker.sock")).unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&request);
                let body = if line.starts_with("GET /version ") {
                    r#"{"ApiVersion":"1.43","MinAPIVersion":"1.24"}"#.to_owned()
                } else {
                    format!(r#"{{"Id":"{}","Config":{{"Tty":false}}}}"#, "9".repeat(64))
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
}

/// Opens a connection to `role`'s handler and returns the client end once
/// the handler has made its accept-time read and is waiting for the frame.
async fn connect(role: Arc<SharedRole>) -> (UnixStream, tokio::task::JoinHandle<()>) {
    let (client, server) = UnixStream::pair().unwrap();
    let task = tokio::spawn(async move { role.serve(server).await });
    // On this single-threaded runtime the handler runs up to its first
    // await, the frame read, and so has made its first ceiling read.
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    (client, task)
}

async fn send(
    mut client: UnixStream,
    task: tokio::task::JoinHandle<()>,
    request: &Request,
) -> Receipt {
    // A peer refused at accept is answered and closed before its frame, so
    // the write may fail; the refusal is still there to read.
    let _ = limeos_contracts::write_frame(&mut client, request).await;
    let receipt: Receipt = limeos_contracts::read_frame(&mut client).await.unwrap();
    task.await.unwrap();
    receipt
}

async fn exchange(role: &Arc<SharedRole>, request: &Request) -> Receipt {
    let (client, task) = connect(role.clone()).await;
    send(client, task, request).await
}

fn logs(id: &str) -> Request {
    Request::ContainerLogs {
        version: VERSION,
        resource: format!("container:{id}"),
        options: LogOptions { tail: 10 },
    }
}
const HEALTH: Request = Request::Health { version: VERSION };

#[tokio::test(flavor = "current_thread")]
async fn a_revocation_between_two_requests_refuses_the_second() {
    if me() == 0 {
        return eprintln!("skipped: a root test user cannot be core");
    }
    let root = Root::new();
    let (a, b) = (id('a'), id('b'));
    root.write(&FILE[1..], &container_ceiling_json(me(), &[&a, &b]));
    let role = Arc::new(role(&root, CeilingRole::Container));
    fake_engine(&root);

    assert!(exchange(&role, &HEALTH).await.ready);
    // Authorized, dispatched, and refused by the stand-in Engine.
    assert_eq!(
        exchange(&role, &logs(&a)).await.error,
        Some(ErrorCode::Conflict)
    );

    root.write(&FILE[1..], &container_ceiling_json(me(), &[&b]));
    let revoked = exchange(&role, &logs(&a)).await;
    assert_eq!(revoked.error, Some(ErrorCode::Forbidden));
    assert!(!revoked.ready);
    assert_eq!(
        exchange(&role, &logs(&b)).await.error,
        Some(ErrorCode::Conflict)
    );

    // Health withdrawn as well: refused on the next request.
    root.write(
        &FILE[1..],
        r#"{"version":1,"core_uid":UID,"allow_health":false,"allow_container_read":true}"#
            .replace("UID", &me().to_string())
            .as_str(),
    );
    assert!(!exchange(&role, &HEALTH).await.ready);
}

#[tokio::test(flavor = "current_thread")]
async fn a_revocation_after_accept_and_before_the_frame_refuses_the_request() {
    if me() == 0 {
        return eprintln!("skipped: a root test user cannot be core");
    }
    let root = Root::new();
    let a = id('a');
    root.write(&FILE[1..], &container_ceiling_json(me(), &[&a]));
    let role = Arc::new(role(&root, CeilingRole::Container));
    fake_engine(&root);

    // Unchanged between accept and frame: dispatched to the Engine.
    let (client, task) = connect(role.clone()).await;
    assert_eq!(
        send(client, task, &logs(&a)).await.error,
        Some(ErrorCode::Conflict)
    );
    // Accepted, peer checked against a ceiling that lists `a`...
    let (client, task) = connect(role.clone()).await;
    assert!(!task.is_finished(), "the handler waits for the frame");
    // ...then `a` is revoked before the request is sent.
    root.write(&FILE[1..], &container_ceiling_json(me(), &[]));
    let receipt = send(client, task, &logs(&a)).await;
    assert_eq!(receipt.error, Some(ErrorCode::Forbidden));

    // And a ceiling broken in that window refuses it too, as unavailable.
    root.write(&FILE[1..], &container_ceiling_json(me(), &[&a]));
    let (client, task) = connect(role.clone()).await;
    root.write(&FILE[1..], "{");
    assert_eq!(
        send(client, task, &logs(&a)).await.error,
        Some(ErrorCode::Unavailable)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_root_core_or_another_peer_is_refused_by_the_handler() {
    if me() == 0 {
        return eprintln!("skipped: a root test user cannot be core");
    }
    let root = Root::new();
    root.write(&FILE[1..], &container_ceiling_json(0, &[]));
    let role = Arc::new(role(&root, CeilingRole::Container));
    assert_eq!(
        exchange(&role, &HEALTH).await.error,
        Some(ErrorCode::Unavailable)
    );
    root.write(&FILE[1..], &container_ceiling_json(me() + 1, &[]));
    assert_eq!(
        exchange(&role, &HEALTH).await.error,
        Some(ErrorCode::Forbidden)
    );
    // A ceiling for another role, as the role check runs on every read.
    root.write(
        &FILE[1..],
        r#"{"version":1,"core_uid":UID,"allow_health":true,"allow_stack_read":true}"#
            .replace("UID", &me().to_string())
            .as_str(),
    );
    assert_eq!(
        exchange(&role, &HEALTH).await.error,
        Some(ErrorCode::Unavailable)
    );
    root.write(&FILE[1..], &container_ceiling_json(me(), &[]));
    assert!(exchange(&role, &HEALTH).await.ready);
}

#[tokio::test(flavor = "current_thread")]
async fn a_capability_granted_after_start_without_its_resource_is_unavailable() {
    if me() == 0 {
        return eprintln!("skipped: a root test user cannot be core");
    }
    let root = Root::new();
    // Started without writes or a collector; the file now grants both.
    root.write(&FILE[1..], &container_ceiling_json(me(), &[&id('a')]));
    let role = Arc::new(role(&root, CeilingRole::Container));
    let observe = Request::Observe {
        version: VERSION,
        source: Source::Docker,
        active: false,
    };
    let answered = exchange(&role, &observe).await;
    assert_eq!(answered.error, Some(ErrorCode::Unavailable));
    assert!(answered.observations.is_none());
    let epoch = exchange(&role, &Request::ReceiptEpoch { version: VERSION }).await;
    assert_eq!(epoch.error, Some(ErrorCode::Unavailable));

    // A storage reader started without its stack scan.
    root.write(
        &FILE[1..],
        r#"{"version":1,"core_uid":UID,"allow_health":true,"allow_stack_read":true}"#
            .replace("UID", &me().to_string())
            .as_str(),
    );
    let reader = Arc::new(self::role(&root, CeilingRole::StorageReader));
    let stacks = exchange(&reader, &Request::StackInventory { version: VERSION }).await;
    assert_eq!(stacks.error, Some(ErrorCode::Unavailable));
    assert!(stacks.stack_cohort.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn a_container_health_read_the_ceiling_denies_is_forbidden_and_a_failed_one_says_why() {
    if me() == 0 {
        return eprintln!("skipped: a root test user cannot be core");
    }
    let root = Root::new();
    let ceiling = |granted: bool| {
        format!(
            r#"{{"version":1,"core_uid":{},"allow_health":true,"allow_storage_read":true,"allow_stack_read":true,"allow_container_health_read":{granted}}}"#,
            me()
        )
    };
    let read = Request::ContainerHealth {
        version: VERSION,
        nonce: "a".repeat(64),
    };
    // Denied: the ceiling's "no", before any collection or worker.
    root.write(&FILE[1..], &ceiling(false));
    let reader = Arc::new(self::role(&root, CeilingRole::StorageReader));
    let denied = exchange(&reader, &read).await;
    assert_eq!(denied.error, Some(ErrorCode::Forbidden));
    assert!(denied.health_failure.is_none());
    assert!(denied.container_health.is_none());
    // Granted, but this test process has no protected worker to run: the
    // refusal carries its reason.
    root.write(&FILE[1..], &ceiling(true));
    let failed = exchange(&reader, &read).await;
    assert_eq!(failed.error, Some(ErrorCode::Unavailable));
    assert!(failed.health_failure.is_some(), "no reason given");
    assert!(failed.container_health.is_none());
    assert!(!failed.ready);
}
