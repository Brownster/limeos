use super::*;
use axum::body::Body;
use limeos_domain::{ContainerAction, ContainerSnapshot, StackPlan, StackSnapshot};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

#[derive(Clone, Default)]
struct Stub {
    mutations: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
}
fn plan() -> limeos_domain::PlannedStackAction {
    let member = |c: char| ContainerSnapshot {
        resource: format!("container:{}", c.to_string().repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running: true,
    };
    limeos_domain::PlannedStackAction {
        plan: StackPlan {
            id: "c".repeat(64),
            version: 1,
            operation: ContainerAction::Restart,
            principal: "admin".into(),
            grant_revision: 1,
            catalog_stack: "registered".into(),
            expected: StackSnapshot::new("media", vec![member('a'), member('b')]).unwrap(),
            created_at: 100,
            expires_at: 400,
        },
        digest: "d".repeat(64),
    }
}
fn job() -> limeos_domain::StackJob {
    limeos_domain::StackJob {
        id: "e".repeat(64),
        state: limeos_domain::JobState::Queued,
        plan: plan().plan,
    }
}
impl Backend for Stub {
    async fn login(&self, _: Login) -> Result<IssuedSession> {
        Err(Error(ErrorCode::Unauthenticated))
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        panic!("unexpected mutation")
    }
    async fn health(&self) -> Result<Health> {
        Err(Error(ErrorCode::Unavailable))
    }
    async fn session(&self, _: String, csrf: Option<String>) -> Result<SessionView> {
        if csrf.is_some_and(|c| c != "b".repeat(64)) {
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
    async fn plan_stack(
        &self,
        _: String,
        _: String,
        input: limeos_contracts::StackInput,
    ) -> Result<limeos_domain::PlannedStackAction> {
        assert_eq!(input.project, "media");
        self.mutations.fetch_add(1, Ordering::SeqCst);
        Ok(plan())
    }
    async fn approve_stack(
        &self,
        _: String,
        _: String,
        id: String,
        digest: String,
    ) -> Result<limeos_domain::PlanApproval> {
        assert_eq!((id, digest), ("c".repeat(64), "d".repeat(64)));
        self.mutations.fetch_add(1, Ordering::SeqCst);
        Ok(limeos_domain::PlanApproval {
            token: "f".repeat(64),
            expires_at: 400,
        })
    }
    async fn queue_stack(
        &self,
        _: String,
        _: String,
        input: limeos_contracts::QueueStackInput,
    ) -> Result<limeos_domain::StackJob> {
        assert_eq!(input.idempotency_key, "key");
        self.mutations.fetch_add(1, Ordering::SeqCst);
        Ok(job())
    }
    async fn cancel_stack_action(&self, _: String, _: String, _: String, _: bool) -> Result<()> {
        self.mutations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn stack_jobs(&self, _: String) -> Result<Vec<limeos_domain::StackJob>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![job()])
    }
    async fn stack_job_progress(
        &self,
        _: String,
        _: String,
        after: i64,
    ) -> Result<limeos_contracts::StackJobProgress> {
        assert_eq!(after, 3);
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(limeos_contracts::StackJobProgress {
            job: job(),
            events: vec![],
            receipt: None,
        })
    }
}
fn request(method: &str, path: &str, body: Option<&str>, guarded: bool) -> Request {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)));
    if guarded {
        r = r
            .header("origin", "https://localhost")
            .header("x-csrf-token", "b".repeat(64));
    }
    match body {
        Some(body) => r
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned())),
        None => r.body(Body::empty()),
    }
    .unwrap()
}
async fn status(stub: &Stub, request: Request) -> StatusCode {
    router(stub.clone(), "https://localhost".into())
        .oneshot(request)
        .await
        .unwrap()
        .status()
}
const PLAN: &str = r#"{"project":"media","operation":"restart"}"#;
fn queue_body() -> String {
    serde_json::json!({"proposal": plan(), "approval": "f".repeat(64), "idempotency_key": "key"})
        .to_string()
}

#[tokio::test]
async fn stack_mutations_need_origin_csrf_json_and_a_closed_body_before_the_backend() {
    let stub = Stub::default();
    let queue = queue_body();
    let bodies = [
        ("/api/v1/stack/plans", PLAN),
        ("/api/v1/stack/plans/cccc/approval", r#"{"digest":"x"}"#),
        ("/api/v1/stack/jobs", queue.as_str()),
    ];
    for (path, body) in bodies {
        // Missing origin/CSRF.
        assert_eq!(
            status(&stub, request("POST", path, Some(body), false)).await,
            StatusCode::FORBIDDEN
        );
        // Wrong CSRF token.
        let mut wrong = request("POST", path, Some(body), true);
        wrong
            .headers_mut()
            .insert("x-csrf-token", "c".repeat(64).parse().unwrap());
        assert_eq!(status(&stub, wrong).await, StatusCode::FORBIDDEN);
        // Not JSON.
        let mut text = request("POST", path, Some(body), true);
        text.headers_mut()
            .insert("content-type", "text/plain".parse().unwrap());
        assert_eq!(status(&stub, text).await, StatusCode::BAD_REQUEST);
    }
    // Files, services, containers and commands are not selectable.
    for extra in ["compose_file", "service", "container", "command", "path"] {
        let body = PLAN.replacen('{', &format!("{{\"{extra}\":\"x\","), 1);
        assert_eq!(
            status(
                &stub,
                request("POST", "/api/v1/stack/plans", Some(&body), true)
            )
            .await,
            StatusCode::BAD_REQUEST,
            "{extra}"
        );
    }
    // Only start, stop and restart exist.
    for operation in ["up", "down", "recreate", "remove"] {
        let body = format!(r#"{{"project":"media","operation":"{operation}"}}"#);
        assert_eq!(
            status(
                &stub,
                request("POST", "/api/v1/stack/plans", Some(&body), true)
            )
            .await,
            StatusCode::BAD_REQUEST,
            "{operation}"
        );
    }
    assert_eq!(stub.mutations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn plan_approve_queue_and_cancel_reach_the_backend_once_with_closed_statuses() {
    let stub = Stub::default();
    let approval = format!("/api/v1/stack/plans/{}/approval", "c".repeat(64));
    let cancel_plan = format!("/api/v1/stack/plans/{}/cancel", "c".repeat(64));
    let cancel_job = format!("/api/v1/stack/jobs/{}/cancel", "e".repeat(64));
    let digest = format!(r#"{{"digest":"{}"}}"#, "d".repeat(64));
    let queue = queue_body();
    let cases = [
        ("/api/v1/stack/plans", Some(PLAN), StatusCode::CREATED),
        (approval.as_str(), Some(digest.as_str()), StatusCode::OK),
        (
            "/api/v1/stack/jobs",
            Some(queue.as_str()),
            StatusCode::ACCEPTED,
        ),
        (cancel_plan.as_str(), None, StatusCode::NO_CONTENT),
        (cancel_job.as_str(), None, StatusCode::NO_CONTENT),
    ];
    for (path, body, expected) in cases {
        assert_eq!(
            status(&stub, request("POST", path, body, true)).await,
            expected,
            "{path}"
        );
    }
    assert_eq!(stub.mutations.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn stack_reads_need_only_the_session_and_unsupported_methods_are_closed() {
    let stub = Stub::default();
    let id = "e".repeat(64);
    assert_eq!(
        status(&stub, request("GET", "/api/v1/stack/jobs", None, false)).await,
        StatusCode::OK
    );
    assert_eq!(
        status(
            &stub,
            request(
                "GET",
                &format!("/api/v1/stack/jobs/{id}?after=3"),
                None,
                false
            )
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        status(
            &stub,
            request(
                "GET",
                &format!("/api/v1/stack/jobs/{id}?after=3&x=1"),
                None,
                false
            )
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    let anonymous = Request::builder()
        .uri("/api/v1/stack/jobs")
        .body(Body::empty())
        .unwrap();
    assert_eq!(status(&stub, anonymous).await, StatusCode::UNAUTHORIZED);
    for (method, path) in [
        ("PUT", "/api/v1/stack/jobs"),
        ("DELETE", "/api/v1/stack/jobs"),
        ("GET", "/api/v1/stack/plans"),
        ("PATCH", "/api/v1/stack/plans"),
    ] {
        assert_eq!(
            status(&stub, request(method, path, None, true)).await,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path}"
        );
    }
    assert_eq!(stub.reads.load(Ordering::SeqCst), 2);
    assert_eq!(stub.mutations.load(Ordering::SeqCst), 0);
}
