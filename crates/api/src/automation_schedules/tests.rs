use super::*;
use axum::body::Body;
use limeos_domain::automation::{
    AutomationPage, MissedRunPolicy, Schedule, ScheduleDefinition, ScheduleStatus,
    ScheduledOperation,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

const PATH: &str = "/api/v1/automation/schedules";
const INPUT: &str =
    r#"{"interval_seconds":60,"grace_seconds":10,"missed":"coalesce_latest","window":null}"#;
#[derive(Clone, Default)]
struct RouteStub {
    reads: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
    error: Option<ErrorCode>,
    bad_page: bool,
}
fn row() -> ScheduleView {
    ScheduleView {
        schedule: Schedule {
            id: "d".repeat(64),
            revision: 1,
            paused: false,
            definition: ScheduleDefinition {
                operation: ScheduledOperation::ObserveHealth,
                anchor: 100,
                interval_seconds: 60,
                grace_seconds: 10,
                missed: MissedRunPolicy::CoalesceLatest,
                window: None,
            },
            settled_through: None,
        },
        status: ScheduleStatus::AwaitingProducer,
        last_occurrence: None,
    }
}
fn budget(token: &str, deadline: Instant) {
    assert_eq!(token, "a".repeat(64));
    assert!(deadline > Instant::now() && deadline <= Instant::now() + Duration::from_secs(10));
}
impl Backend for RouteStub {
    async fn login(&self, _: Login) -> Result<IssuedSession> {
        Err(Error(ErrorCode::Unauthenticated))
    }
    async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
        panic!("Core owns final snapshot authentication")
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        panic!("unrelated")
    }
    async fn health(&self) -> Result<Health> {
        Err(Error(ErrorCode::Unavailable))
    }
    async fn observe_health_schedules(
        &self,
        token: String,
        after: Option<String>,
        deadline: Instant,
    ) -> Result<AutomationPage<ScheduleView>> {
        // Transport only. Current stored authority is exercised by Core/Store tests.
        budget(&token, deadline);
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.error {
            return Err(Error(code));
        }
        Ok(AutomationPage {
            items: if self.bad_page {
                vec![row(); 33]
            } else if after.is_none() {
                vec![row()]
            } else {
                vec![]
            },
            next: None,
            partial: false,
        })
    }
    async fn create_observe_health_schedule(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ObserveHealthScheduleInput,
        deadline: Instant,
    ) -> Result<ScheduleView> {
        budget(&token, deadline);
        assert_eq!(csrf, "c".repeat(64));
        assert_eq!(
            input.definition().operation,
            ScheduledOperation::ObserveHealth
        );
        assert_eq!(input.definition().anchor, 0);
        self.writes.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.error {
            return Err(Error(code));
        }
        Ok(row())
    }
    async fn pause_observe_health_schedule(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: PauseSchedule,
        deadline: Instant,
    ) -> Result<ScheduleView> {
        budget(&token, deadline);
        assert_eq!(csrf, "c".repeat(64));
        assert_eq!(id, "d".repeat(64));
        self.writes.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.error {
            return Err(Error(code));
        }
        let mut value = row();
        value.schedule.revision = input.revision + 1;
        value.schedule.paused = input.paused;
        value.status = if input.paused {
            ScheduleStatus::Paused
        } else {
            ScheduleStatus::AwaitingProducer
        };
        Ok(value)
    }
}
fn request(method: &str, path: &str, body: &str) -> Request {
    Request::builder()
        .method(method)
        .uri(path)
        .header("origin", "https://localhost")
        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
        .header("x-csrf-token", "c".repeat(64))
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}
async fn error(response: Response, status: StatusCode, code: ErrorCode) {
    assert_eq!(response.status(), status);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: ErrorEnvelope = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.code, code);
}

#[tokio::test]
async fn fixed_inputs_methods_and_credentials_cannot_select_authority_or_effects() {
    let stub = RouteStub::default();
    let app = router(stub.clone(), "https://localhost".into());
    for field in [
        "owner",
        "operation",
        "anchor",
        "role",
        "grant",
        "status",
        "readiness",
        "deadline",
        "proof",
    ] {
        let mut body: serde_json::Value = serde_json::from_str(INPUT).unwrap();
        body[field] = serde_json::json!(true);
        error(
            app.clone()
                .oneshot(request("POST", PATH, &body.to_string()))
                .await
                .unwrap(),
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidInput,
        )
        .await;
    }
    for (header, status, code) in [
        ("origin", StatusCode::FORBIDDEN, ErrorCode::Forbidden),
        (
            "cookie",
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthenticated,
        ),
        ("x-csrf-token", StatusCode::FORBIDDEN, ErrorCode::Forbidden),
    ] {
        let mut value = request("POST", PATH, INPUT);
        value.headers_mut().remove(header);
        error(app.clone().oneshot(value).await.unwrap(), status, code).await;
    }
    for body in [
        "{}",
        "{bad",
        r#"{"interval_seconds":29,"grace_seconds":0,"missed":"skip"}"#,
    ] {
        error(
            app.clone()
                .oneshot(request("POST", PATH, body))
                .await
                .unwrap(),
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidInput,
        )
        .await;
    }
    let pause = format!("{PATH}/{}/pause", "d".repeat(64));
    assert_eq!(
        app.clone()
            .oneshot(request("GET", &pause, ""))
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    error(
        app.clone()
            .oneshot(request(
                "POST",
                &pause,
                r#"{"revision":1,"paused":true,"owner":"someone"}"#,
            ))
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        ErrorCode::InvalidInput,
    )
    .await;
    for query in [
        "?owner=someone",
        "?after=invalid",
        "?after=a&operation=backup",
    ] {
        error(
            app.clone()
                .oneshot(request("GET", &format!("{PATH}{query}"), ""))
                .await
                .unwrap(),
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidInput,
        )
        .await;
    }
    for path in [
        "/api/v1/automation/alerts",
        "/api/v1/automation/notifications",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request("GET", path, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(stub.reads.load(Ordering::SeqCst), 0);
    assert_eq!(stub.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn metadata_routes_preserve_budget_cookie_security_headers_and_read_only_get() {
    let stub = RouteStub::default();
    let app = router(stub.clone(), "https://localhost".into());
    let response = app
        .clone()
        .oneshot(request("POST", PATH, INPUT))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(!response.headers().contains_key("set-cookie"));
    assert_eq!(response.headers()["cache-control"], "no-store");
    let created: ScheduleView =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(created.status, ScheduleStatus::AwaitingProducer);
    let response = app.clone().oneshot(request("GET", PATH, "")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key("set-cookie"));
    let page: AutomationPage<ScheduleView> =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(page.items, vec![created]);
    assert_eq!(stub.writes.load(Ordering::SeqCst), 1);
    let response = app
        .oneshot(request(
            "POST",
            &format!("{PATH}/{}/pause", "d".repeat(64)),
            r#"{"revision":1,"paused":true}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key("set-cookie"));
    let paused: ScheduleView =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(paused.status, ScheduleStatus::Paused);
}

#[tokio::test]
async fn exact_backend_errors_and_bad_pages_never_become_success() {
    for (code, status) in [
        (ErrorCode::Forbidden, StatusCode::FORBIDDEN),
        (ErrorCode::Unauthenticated, StatusCode::UNAUTHORIZED),
        (ErrorCode::Expired, StatusCode::FORBIDDEN),
        (ErrorCode::Conflict, StatusCode::CONFLICT),
        (ErrorCode::Overloaded, StatusCode::TOO_MANY_REQUESTS),
        (ErrorCode::StateNotDurable, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let app = router(
            RouteStub {
                error: Some(code),
                ..Default::default()
            },
            "https://localhost".into(),
        );
        error(
            app.oneshot(request("POST", PATH, INPUT)).await.unwrap(),
            status,
            code,
        )
        .await;
    }
    let app = router(
        RouteStub {
            bad_page: true,
            ..Default::default()
        },
        "https://localhost".into(),
    );
    error(
        app.oneshot(request("GET", PATH, "")).await.unwrap(),
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::StateNotDurable,
    )
    .await;
}
