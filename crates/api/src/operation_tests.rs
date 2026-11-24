use super::*;
use axum::body::Body;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;
#[derive(Clone)]
struct Mutations(Arc<AtomicUsize>);
impl Mutations {
    /// As core does, in the call itself: a wrong CSRF value is refused before
    /// anything counts as a mutation.
    fn mutate<T>(&self, csrf: &str) -> Result<T> {
        if csrf != "b".repeat(64) {
            return Err(Error(ErrorCode::Forbidden));
        }
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Error(ErrorCode::Unavailable))
    }
}
impl TestBackend for Mutations {
    /// Routes this fake does not count still check the CSRF binding, as core does.
    fn fallback(&self, _: &'static str, _: Option<&str>, csrf: Option<&str>) -> Error {
        if csrf.is_some_and(|csrf| csrf != "b".repeat(64)) {
            return Error(ErrorCode::Forbidden);
        }
        Error(ErrorCode::Unavailable)
    }
    async fn plan_storage_mount(
        &self,
        _: String,
        csrf: String,
        _: limeos_domain::StorageMountSelection,
    ) -> Result<limeos_domain::PlannedStorageMount> {
        self.mutate(&csrf)
    }
    async fn approve_storage_mount(
        &self,
        _: String,
        csrf: String,
        _: String,
        _: String,
    ) -> Result<limeos_domain::PlanApproval> {
        self.mutate(&csrf)
    }
    async fn queue_storage_mount(
        &self,
        _: String,
        csrf: String,
        _: limeos_contracts::QueueStorageMountInput,
    ) -> Result<limeos_domain::StorageMountJob> {
        self.mutate(&csrf)
    }
    async fn cancel_storage_mount(&self, _: String, csrf: String, _: String) -> Result<()> {
        self.mutate(&csrf)
    }
    async fn cancel_storage_mount_job(&self, _: String, csrf: String, _: String) -> Result<()> {
        self.mutate(&csrf)
    }
    async fn plan_storage(
        &self,
        _: String,
        csrf: String,
        _: limeos_domain::StorageSetupInput,
    ) -> Result<limeos_domain::PlannedStorageSetup> {
        self.mutate(&csrf)
    }
    async fn approve_storage(
        &self,
        _: String,
        csrf: String,
        _: String,
        _: String,
    ) -> Result<limeos_domain::PlanApproval> {
        self.mutate(&csrf)
    }
    async fn cancel_storage(&self, _: String, csrf: String, _: String) -> Result<()> {
        self.mutate(&csrf)
    }
    async fn plan_compose(
        &self,
        _: String,
        csrf: String,
        _: limeos_domain::ComposeSelection,
    ) -> Result<limeos_domain::PlannedCompose> {
        self.mutate(&csrf)
    }
    async fn approve_compose(
        &self,
        _: String,
        csrf: String,
        _: String,
        _: String,
    ) -> Result<limeos_domain::PlanApproval> {
        self.mutate(&csrf)
    }
    async fn cancel_compose(&self, _: String, csrf: String, _: String) -> Result<()> {
        self.mutate(&csrf)
    }
    async fn login(&self, _: Login, _: PasswordAttempt) -> PasswordChecked<IssuedSession> {
        PasswordChecked::rejected()
    }
    async fn session(&self, _: String, csrf: Option<String>) -> Result<SessionView> {
        if csrf.as_deref() != Some("b".repeat(64).as_str()) {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(SessionView {
            principal: limeos_domain::Principal {
                id: "admin".into(),
                role: limeos_domain::Role::Administrator,
                grant_revision: 1,
            },
            csrf_token: "b".repeat(64),
            expires_at: 999,
        })
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        panic!("unexpected mutation")
    }
    async fn health(&self) -> Result<Health> {
        Err(Error(ErrorCode::Unavailable))
    }
    async fn plan_restart(
        &self,
        _: String,
        csrf: String,
        _: limeos_contracts::RestartInput,
    ) -> Result<limeos_domain::PlannedRestart> {
        self.mutate(&csrf)
    }
    async fn plan_container(
        &self,
        _: String,
        csrf: String,
        _: limeos_contracts::ContainerInput,
    ) -> Result<limeos_domain::PlannedContainerAction> {
        self.mutate(&csrf)
    }
    async fn approve_container(
        &self,
        _: String,
        csrf: String,
        _: String,
        _: String,
    ) -> Result<limeos_domain::PlanApproval> {
        self.mutate(&csrf)
    }
    async fn cancel_restart(&self, _: String, csrf: String, _: String, _: bool) -> Result<()> {
        self.mutate(&csrf)
    }
}
#[tokio::test]
async fn every_mutation_rejects_get_origin_csrf_and_untyped_extra_fields() {
    let mutations = Arc::new(AtomicUsize::new(0));
    let router = router(Mutations(mutations.clone()), "https://localhost".into());
    for path in [
        "/api/v1/storage/mounts/plans".into(),
        format!("/api/v1/storage/mounts/plans/{}/approval", "a".repeat(64)),
        format!("/api/v1/storage/mounts/plans/{}/cancel", "a".repeat(64)),
        format!("/api/v1/storage/mounts/jobs/{}/cancel", "a".repeat(64)),
        "/api/v1/storage/targets/plans".into(),
        format!("/api/v1/storage/targets/plans/{}/approval", "a".repeat(64)),
        format!("/api/v1/storage/targets/plans/{}/cancel", "a".repeat(64)),
        "/api/v1/storage/targets/jobs".into(),
        format!("/api/v1/storage/targets/jobs/{}/cancel", "a".repeat(64)),
        "/api/v1/storage/plans".into(),
        format!("/api/v1/storage/plans/{}/approval", "a".repeat(64)),
        format!("/api/v1/storage/plans/{}/cancel", "a".repeat(64)),
        "/api/v1/compose/plans".into(),
        format!("/api/v1/compose/plans/{}/approval", "a".repeat(64)),
        format!("/api/v1/compose/plans/{}/cancel", "a".repeat(64)),
        "/api/v1/container/plans".into(),
        format!("/api/v1/container/plans/{}/approval", "a".repeat(64)),
        format!("/api/v1/container/plans/{}/cancel", "a".repeat(64)),
        format!("/api/v1/container/jobs/{}/cancel", "a".repeat(64)),
        "/api/v1/container/restart/plans".into(),
        format!(
            "/api/v1/container/restart/plans/{}/approval",
            "a".repeat(64)
        ),
        format!("/api/v1/container/restart/plans/{}/cancel", "a".repeat(64)),
        format!("/api/v1/container/restart/jobs/{}/cancel", "a".repeat(64)),
    ] {
        for (method, origin, csrf, status) in [
            ("GET", "https://localhost", "b".repeat(64), 405),
            ("POST", "https://foreign.invalid", "b".repeat(64), 403),
            // Core checks the CSRF binding in the call itself, after the body
            // is parsed: an empty body is refused first, a valid one with 403
            // (the route-table test sends valid bodies). Neither mutates.
            ("POST", "https://localhost", "a".repeat(64), 0),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(&path)
                        .header("origin", origin)
                        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                        .header("x-csrf-token", csrf)
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            if status == 0 {
                assert!(matches!(response.status().as_u16(), 400 | 403), "{path}");
            } else {
                assert_eq!(response.status().as_u16(), status, "{path}");
            }
        }
    }
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/container/restart/plans")
                .header("origin", "https://localhost")
                .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                .header("x-csrf-token", "b".repeat(64))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"resource":"container:fixture","command":"sh -c forbidden"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn compose_selections_cannot_carry_manifests_or_claimed_authority() {
    let mutations = Arc::new(AtomicUsize::new(0));
    let router = router(Mutations(mutations.clone()), "https://localhost".into());
    for extra in ["project", "yaml", "privileged", "principal", "command"] {
        let mut body = serde_json::json!({"stack":"media","template":"standard"});
        body[extra] = serde_json::json!("forbidden");
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/compose/plans")
                    .header("origin", "https://localhost")
                    .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                    .header("x-csrf-token", "b".repeat(64))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn guided_storage_accepts_no_fstab_options_commands_units_or_claimed_principal() {
    let mutations = Arc::new(AtomicUsize::new(0));
    let router = router(Mutations(mutations.clone()), "https://localhost".into());
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    for extra in ["fstab", "options", "command", "unit", "principal"] {
        let mut body = serde_json::json!({"contract": fixture["cases"][0]["contract"], "inventory_digest": "a".repeat(64), "timeout_seconds": 10});
        body[extra] = serde_json::json!("forbidden");
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/storage/plans")
                    .header("origin", "https://localhost")
                    .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                    .header("x-csrf-token", "b".repeat(64))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn mount_catalog_and_job_reads_never_dispatch_and_selection_accepts_no_paths_or_authority() {
    let mutations = Arc::new(AtomicUsize::new(0));
    let router = router(Mutations(mutations.clone()), "https://localhost".into());
    for path in [
        "/api/v1/storage/mounts/targets",
        "/api/v1/storage/mounts/jobs",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    for extra in [
        "path",
        "contract",
        "filesystem_uuid",
        "options",
        "principal",
        "quiescence",
    ] {
        let mut body = serde_json::json!({"target_id":"data"});
        body[extra] = serde_json::json!("forbidden");
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/storage/mounts/plans")
                    .header("origin", "https://localhost")
                    .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                    .header("x-csrf-token", "b".repeat(64))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
}

/// A valid reviewed mount proposal of 16 to 32 KiB, as one queue body.
pub(super) fn mount_queue_input() -> limeos_contracts::QueueStorageMountInput {
    let identity = serde_json::json!({"major":8,"minor":1,"mount_id":10,"inode":2,"mode":16877,"uid":0,"gid":0});
    let mut devices = vec![
        serde_json::json!({"device":{"major":8,"minor":16},"filesystem_uuid":"uuid-data","filesystem":"ext4","serial":null,"boot_backing":false,"mounts":[]}),
    ];
    for minor in 0..100 {
        devices.push(serde_json::json!({"device":{"major":9,"minor":minor},"filesystem_uuid":format!("uuid-other-{minor}"),"filesystem":"ext4","serial":"s".repeat(64),"boot_backing":false,"mounts":[]}));
    }
    let value = serde_json::json!({
        "proposal":{"plan":{"id":"f".repeat(64),"version":1,"principal":"admin","grant_revision":1,
            "effect":{"version":1,"operation":"storage.mount","action":"e".repeat(64),
                "device":{"id":"data","role":"data","filesystem_uuid":"uuid-data","filesystem":"ext4","mountpoint":"/mnt/data","serial":null},
                "expected":{"host_root_mount_id":10,"topology_digest":"a".repeat(64),"mounts_digest":"b".repeat(64),"fstab_digest":"c".repeat(64),"fstab_entries":[],"devices":devices},
                "target":{"mountpoint":"/mnt/data","parent":identity,"existing":identity},
                "physical_devices":[{"major":8,"minor":16}],"consumer_digest":"d".repeat(64),"created_at":100,"expires_at":400}},"digest":"b".repeat(64)},
        "approval":"c".repeat(64),"idempotency_key":"mount-http-fixture"});
    serde_json::from_value(value).unwrap()
}
#[tokio::test]
async fn mount_queue_accepts_a_valid_large_plan_but_retains_a_bounded_http_body() {
    use limeos_domain::*;
    let input = mount_queue_input();
    input.proposal.plan.validate(100).unwrap();
    let body = serde_json::to_string(&input).unwrap();
    assert!(body.len() > 16 * 1024 && body.len() < 32 * 1024);
    assert!(serde_json::to_vec(&input.proposal.plan).unwrap().len() <= STORAGE_MOUNT_PLAN_BYTES);
    let mutations = Arc::new(AtomicUsize::new(0));
    let router = router(Mutations(mutations.clone()), "https://localhost".into());
    for (path, payload, origin, csrf, status) in [
        (
            "/api/v1/storage/mounts/jobs",
            body.clone(),
            "https://foreign.invalid",
            "b".repeat(64),
            StatusCode::FORBIDDEN,
        ),
        (
            "/api/v1/storage/mounts/jobs",
            body.clone(),
            "https://localhost",
            "a".repeat(64),
            StatusCode::FORBIDDEN,
        ),
        (
            "/api/v1/storage/mounts/jobs",
            body.clone(),
            "https://localhost",
            "b".repeat(64),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            "/api/v1/storage/mounts/jobs",
            format!("{body}{}", " ".repeat(32 * 1024)),
            "https://localhost",
            "b".repeat(64),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/v1/storage/mounts/plans",
            format!("{{\"target_id\":\"data\"}}{}", " ".repeat(16 * 1024)),
            "https://localhost",
            "b".repeat(64),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("origin", origin)
                    .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                    .header("x-csrf-token", csrf)
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{path}");
    }
    // Only the bounded, authenticated queue request reaches the typed backend.
    assert_eq!(mutations.load(Ordering::SeqCst), 1);
}

/// A resource that is not `container:<64 lowercase hex>` is refused while
/// decoding the body (`ContainerResource`), before any backend call.
#[tokio::test]
async fn a_malformed_container_resource_never_reaches_the_backend() {
    let mutations = Arc::new(AtomicUsize::new(0));
    let router = router(Mutations(mutations.clone()), "https://localhost".into());
    let id = "a".repeat(64);
    let resources = [
        String::new(),
        id.clone(),
        format!("container:{}", id.to_uppercase()),
        format!("container:container:{id}"),
        format!("stack:{id}"),
        format!("container:{}", &id[..63]),
    ];
    for (path, extra) in [
        ("/api/v1/container/restart/plans", None),
        ("/api/v1/container/plans", Some("stop")),
    ] {
        for resource in &resources {
            let mut body = serde_json::json!({ "resource": resource });
            if let Some(operation) = extra {
                body["operation"] = serde_json::json!(operation);
            }
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header("origin", "https://localhost")
                        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                        .header("x-csrf-token", "b".repeat(64))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{path} {resource:?}"
            );
        }
    }
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
    // Control: the same request with a valid resource reaches the backend.
    router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/container/restart/plans")
                .header("origin", "https://localhost")
                .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
                .header("x-csrf-token", "b".repeat(64))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "resource": format!("container:{id}") }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mutations.load(Ordering::SeqCst), 1);
}
