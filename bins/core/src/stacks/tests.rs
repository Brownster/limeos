use super::*;
use limeos_domain::{
    Role,
    deployment::discovery::{
        CatalogFacts, Declaration, DiscoveryCohort, EngineFacts, FileStatus, Membership,
        ProjectIdentity, RuntimeStatus,
    },
};

fn cohort() -> DiscoveryCohort {
    DiscoveryCohort::new(
        Some(CatalogFacts {
            engine_id: "engine".into(),
            sampled_at: now(),
            stacks: vec![Declaration {
                stack: "registered".into(),
                project: "media".into(),
                working_directory: "/opt/Media".into(),
                config_files: vec!["/opt/Media/compose.yaml".into()],
                services: vec!["web".into()],
                files: FileStatus::NotInspected,
            }],
        }),
        Some(EngineFacts {
            engine_id: "engine".into(),
            sampled_at: now(),
            members: vec![Membership::Compose {
                container: limeos_domain::ContainerSnapshot {
                    resource: format!("container:{}", "a".repeat(64)),
                    image: format!("sha256:{}", "b".repeat(64)),
                    started_at: "2026-10-20T00:00:00Z".into(),
                    running: false,
                },
                project: ProjectIdentity {
                    name: "media".into(),
                    working_directory: "/opt/Media".into(),
                    config_files: vec!["/opt/Media/compose.yaml".into()],
                },
                service: "web".into(),
                one_off: false,
            }],
        }),
    )
    .unwrap()
}
async fn setup(role: Role, resources: Vec<String>) -> (tempfile::TempDir, Core, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("core.sqlite")).unwrap();
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/werkzeug-hashes.json"
    ))
    .unwrap();
    let encoded = fixture[0]["hash"].as_str().unwrap().to_owned();
    let token = "c".repeat(64);
    let digest = limeos_identity::digest(&token);
    let id = db
        .call(move |store| {
            let principal = store.import_legacy_user("reader", &encoded, now())?;
            store.revise_grants(
                &principal.id,
                role,
                &resources
                    .into_iter()
                    .map(|resource| Scope {
                        operation: Operation::HealthRead,
                        resource,
                    })
                    .collect::<Vec<_>>(),
                now(),
            )?;
            let record = store.login_record("reader")?.unwrap();
            store.create_session(&record, None, &digest, "csrf", now())?;
            Ok(principal.id)
        })
        .await
        .unwrap();
    let core = Core {
        db,
        password_workers: Arc::new(Semaphore::new(1)),
        dummy_hash: "unused".into(),
        observations: Default::default(),
        telemetry: None,
        container_socket: Arc::new("/must-not-probe".into()),
        compose_catalog: None,
        storage_socket: Arc::new("/must-not-probe".into()),
        storage_target_socket: Arc::new("/unused".into()),
        storage_mount_socket: Arc::new("/unused".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        stacks: Default::default(),
    };
    core.stacks.update(&cohort()).unwrap();
    (dir, core, token, id)
}
fn scopes() -> Vec<String> {
    vec![
        "deployment:stacks".into(),
        "stack:registered".into(),
        "stack:media".into(),
        format!("container:{}", "a".repeat(64)),
    ]
}

#[tokio::test]
async fn whole_cached_view_requires_summary_stack_project_and_member_grants() {
    for removed in 0..4 {
        let mut grants = scopes();
        grants.remove(removed);
        let (_dir, core, token, _) = setup(Role::Viewer, grants).await;
        assert_eq!(
            core.stack_inventory(token).await.unwrap_err().0,
            ErrorCode::Forbidden
        );
    }
    let (_dir, core, token, _) = setup(Role::Viewer, scopes()).await;
    let original = core.stacks.cohort().unwrap().catalog().unwrap().sampled_at;
    for _ in 0..3 {
        let view = core.stack_inventory(token.clone()).await.unwrap();
        assert_eq!(view.stacks[0].runtime, RuntimeStatus::Stopped);
        assert_eq!(view.stacks[0].services[0].members.len(), 1);
        assert_eq!(view.catalog.sampled_at, Some(original));
    }
}
#[tokio::test]
async fn administrator_role_without_grants_never_discloses_the_cache() {
    let (_dir, core, token, _) = setup(Role::Administrator, vec![]).await;
    assert_eq!(
        core.stack_inventory(token).await.unwrap_err().0,
        ErrorCode::Forbidden
    );
}
#[tokio::test]
async fn revoked_session_and_changed_grants_clear_access_to_retained_stale_facts() {
    let (_dir, core, token, id) = setup(Role::Viewer, scopes()).await;
    core.stacks.failed();
    assert_eq!(
        core.stack_inventory(token.clone()).await.unwrap().stacks[0].counts,
        None
    );
    core.db
        .call(move |store| store.revise_grants(&id, Role::Viewer, &[], now()))
        .await
        .unwrap();
    assert_eq!(
        core.stack_inventory(token).await.unwrap_err().0,
        ErrorCode::Unauthenticated
    );
    assert_eq!(
        core.stack_inventory("d".repeat(64)).await.unwrap_err().0,
        ErrorCode::Unauthenticated
    );
}

#[tokio::test]
async fn actual_http_route_reads_the_core_cache_and_rechecks_revocation() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_dir, core, token, id) = setup(Role::Viewer, scopes()).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = limeos_api::router(core.clone(), "https://localhost".into());
    let server = tokio::spawn(async move { limeos_api::serve(listener, app).await });
    async fn get(address: std::net::SocketAddr, token: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream.write_all(format!("GET /api/v1/deployment/stacks HTTP/1.1\r\nHost: localhost\r\nCookie: __Host-limeos={token}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.take(70_000).read_to_end(&mut bytes),
        )
        .await
        .unwrap()
        .unwrap();
        String::from_utf8(bytes).unwrap()
    }
    let response = get(address, &token).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(
        response
            .to_ascii_lowercase()
            .contains("cache-control: no-store")
    );
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    let value: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(value["stacks"][0]["runtime"], "stopped");
    assert_eq!(
        value["stacks"][0]["services"][0]["members"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    core.db
        .call(move |store| store.revise_grants(&id, Role::Viewer, &[], now()))
        .await
        .unwrap();
    let denied = get(address, &token).await;
    assert!(denied.starts_with("HTTP/1.1 401"), "{denied}");
    assert!(!denied.contains("/opt/Media"));
    server.abort();
    let _ = server.await;
}
