use super::*;
use axum::body::Body;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tower::ServiceExt;

#[derive(Clone, Default)]
struct Stub {
    calls: Arc<AtomicUsize>,
    revoked: Arc<AtomicBool>,
    oversize: bool,
}
impl Backend for Stub {
    async fn login(&self, _: Login) -> Result<IssuedSession> {
        Err(Error(ErrorCode::Unauthenticated))
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        Err(Error(ErrorCode::Unavailable))
    }
    async fn health(&self) -> Result<Health> {
        Err(Error(ErrorCode::Unavailable))
    }
    async fn session(&self, _: String, csrf: Option<String>) -> Result<SessionView> {
        if self.revoked.load(Ordering::SeqCst) {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        if csrf.as_ref().is_some_and(|csrf| *csrf != "b".repeat(64)) {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(SessionView {
            principal: limeos_domain::Principal {
                id: "operator".into(),
                role: limeos_domain::Role::Operator,
                grant_revision: 1,
            },
            csrf_token: "b".repeat(64),
            expires_at: 9999,
        })
    }
}
fn view() -> ScheduleView {
    ScheduleView {
        schedule: Schedule {
            id: "health".into(),
            revision: 1,
            paused: false,
            settled_through: None,
            definition: ScheduleDefinition {
                operation: ScheduledOperation::ObserveHealth,
                anchor: 1000,
                interval_seconds: 60,
                grace_seconds: 30,
                missed: MissedRunPolicy::CoalesceLatest,
                window: None,
            },
        },
        status: ScheduleStatus::AwaitingProducer,
        last_occurrence: None,
    }
}
impl AutomationBackend for Stub {
    async fn automation_schedules(
        &self,
        _: String,
        _: Option<String>,
    ) -> Result<AutomationPage<ScheduleView>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AutomationPage {
            items: vec![view(); if self.oversize { 33 } else { 1 }],
            next: None,
            partial: true,
        })
    }
    async fn automation_create(
        &self,
        _: String,
        _: String,
        _: CreateSchedule,
    ) -> Result<ScheduleView> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(view())
    }
    async fn automation_pause(
        &self,
        _: String,
        _: String,
        _: String,
        _: PauseSchedule,
    ) -> Result<ScheduleView> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Error(ErrorCode::Conflict))
    }
    async fn automation_acknowledge(
        &self,
        _: String,
        _: String,
        _: String,
        _: AcknowledgeNotification,
    ) -> Result<Notification> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Error(ErrorCode::Conflict))
    }
}
fn router(backend: Stub) -> Router {
    routes().with_state(App {
        backend,
        origin: Arc::from("https://localhost"),
        limiter: Arc::new(Mutex::new(Limiter::default())),
        readers: Arc::new(Semaphore::new(2)),
    })
}
fn request(method: &str, path: &str, origin: &str, csrf: &str, body: &str) -> Request {
    Request::builder()
        .method(method)
        .uri(path)
        .header("origin", origin)
        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
        .header("x-csrf-token", csrf)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}
#[tokio::test]
async fn mutation_checks_origin_session_csrf_before_body_or_backend() {
    let backend = Stub::default();
    let app = router(backend.clone());
    for path in [
        "/api/v1/automation/schedules",
        "/api/v1/automation/schedules/health/pause",
        "/api/v1/automation/notifications/note/acknowledgment",
    ] {
        for (origin, csrf) in [
            ("https://foreign.invalid", "b".repeat(64)),
            ("https://localhost", "a".repeat(64)),
        ] {
            let response = app
                .clone()
                .oneshot(request("POST", path, origin, &csrf, "invalid body"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }
    backend.revoked.store(true, Ordering::SeqCst);
    assert_eq!(
        app.oneshot(request(
            "POST",
            "/api/v1/automation/schedules",
            "https://localhost",
            &"b".repeat(64),
            "invalid body"
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn reads_require_session_and_mutations_reject_get() {
    let backend = Stub::default();
    let app = router(backend.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/automation/schedules")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    for path in [
        "/api/v1/automation/schedules/health/pause",
        "/api/v1/automation/notifications/note/acknowledgment",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request(
                    "GET",
                    path,
                    "https://localhost",
                    &"b".repeat(64),
                    "{}"
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    let response = app
        .oneshot(request(
            "GET",
            "/api/v1/automation/schedules",
            "https://localhost",
            "",
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: AutomationPage<ScheduleView> =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    assert!(value.partial);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn bounded_closed_inputs_and_stale_revisions() {
    let backend = Stub::default();
    let app = router(backend.clone());
    let csrf = "b".repeat(64);
    for body in [
        r#"{"revision":1,"paused":true,"principal":"admin"}"#.to_owned(),
        "x".repeat(16385),
        r#"{"revision":0,"paused":true}"#.to_owned(),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request(
                    "POST",
                    "/api/v1/automation/schedules/health/pause",
                    "https://localhost",
                    &csrf,
                    &body
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        app.clone()
            .oneshot(request(
                "POST",
                "/api/v1/automation/schedules/health/pause",
                "https://localhost",
                &csrf,
                r#"{"revision":1,"paused":true}"#
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.oneshot(request(
            "POST",
            "/api/v1/automation/notifications/note/acknowledgment",
            "https://localhost",
            &csrf,
            r#"{"revision":1}"#
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn oversized_store_response_and_unknown_cursor_refused() {
    let backend = Stub {
        oversize: true,
        ..Default::default()
    };
    let app = router(backend);
    assert_eq!(
        app.clone()
            .oneshot(request(
                "GET",
                "/api/v1/automation/schedules",
                "https://localhost",
                "",
                ""
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        app.oneshot(request(
            "GET",
            "/api/v1/automation/schedules?permission=admin",
            "https://localhost",
            "",
            ""
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::BAD_REQUEST
    );
}
