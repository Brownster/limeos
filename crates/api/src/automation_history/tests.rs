//! Route-shape tests; real current authority is exercised by Core/Store tests.
use super::*;
use axum::body::Body;
use limeos_domain::automation::{AlertEvent, AlertEventKind, AlertKind, IncidentState, Severity};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

#[derive(Clone)]
struct Stub {
    alerts: Vec<AlertState>,
    notes: Vec<Notification>,
    next: Option<String>,
    calls: Arc<AtomicUsize>,
}
fn alert() -> AlertState {
    AlertState {
        key: "mount:/Case/Path".into(),
        kind: AlertKind::Mount,
        last_sequence: 2,
        incident: IncidentState::Open {
            generation: 2,
            severity: Severity::Warning,
            summary: "Fixture incident".into(),
            announced: true,
        },
    }
}
fn note() -> Notification {
    Notification {
        id: "d".repeat(64),
        revision: 2,
        event: AlertEvent {
            key: "mount:/Case/Path".into(),
            generation: 2,
            event: AlertEventKind::Incident,
            kind: AlertKind::Mount,
            severity: Severity::Warning,
            summary: "Fixture incident".into(),
            at: 100,
        },
        channel: DeliveryChannel::InApp,
        delivery: DeliveryState::Delivered { at: 101 },
        attempts: 1,
        acknowledged_at: Some(102),
    }
}
fn stub() -> Stub {
    Stub {
        alerts: vec![alert()],
        notes: vec![note()],
        next: None,
        calls: Arc::new(AtomicUsize::new(0)),
    }
}
fn budget(token: &str, deadline: Instant) {
    assert_eq!(token, "a".repeat(64));
    assert!(deadline > Instant::now() && deadline <= Instant::now() + Duration::from_secs(10));
}
impl Backend for Stub {
    async fn login(&self, _: Login) -> Result<IssuedSession> {
        Err(Error(ErrorCode::Unauthenticated))
    }
    async fn session(&self, _: String, _: Option<String>) -> Result<SessionView> {
        panic!("Store authenticates")
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        panic!("unrelated")
    }
    async fn health(&self) -> Result<Health> {
        Err(Error(ErrorCode::Unavailable))
    }
    async fn observe_health_alerts(
        &self,
        token: String,
        _: Option<String>,
        deadline: Instant,
    ) -> Result<AutomationPage<AlertState>> {
        budget(&token, deadline);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AutomationPage {
            items: self.alerts.clone(),
            next: self.next.clone(),
            partial: true,
        })
    }
    async fn observe_health_notifications(
        &self,
        token: String,
        _: Option<String>,
        deadline: Instant,
    ) -> Result<AutomationPage<Notification>> {
        budget(&token, deadline);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AutomationPage {
            items: self.notes.clone(),
            next: self.next.clone(),
            partial: true,
        })
    }
    async fn acknowledge_observe_health_notification(
        &self,
        token: String,
        csrf: String,
        id: String,
        input: AcknowledgeNotification,
        deadline: Instant,
    ) -> Result<Notification> {
        budget(&token, deadline);
        assert_eq!(csrf, "c".repeat(64));
        assert_eq!(id, "d".repeat(64));
        assert_eq!(input.revision, 1);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.notes[0].clone())
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
async fn value(response: Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 70_000).await.unwrap()).unwrap()
}

#[tokio::test]
async fn registered_reads_preserve_partial_and_literal_keys_and_post_only_acknowledgment() {
    let backend = stub();
    let app = router(backend.clone(), "https://localhost".into());
    let response = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/automation/alerts?after=mount%3A%2FCase%2FOther",
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let page = value(response).await;
    assert_eq!(page["partial"], true);
    assert_eq!(page["items"][0]["key"], "mount:/Case/Path");
    let path = format!(
        "/api/v1/automation/notifications/{}/acknowledgment",
        "d".repeat(64)
    );
    assert_eq!(
        app.clone()
            .oneshot(request("GET", &path, ""))
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    let response = app
        .oneshot(request("POST", &path, r#"{"revision":1}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(value(response).await["acknowledged_at"], 102);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn malformed_cursors_credentials_and_ack_fields_never_reach_backend() {
    let backend = stub();
    let app = router(backend.clone(), "https://localhost".into());
    for path in [
        "/api/v1/automation/alerts?after=",
        "/api/v1/automation/alerts?after=%00",
        "/api/v1/automation/alerts?owner=other",
        "/api/v1/automation/notifications?after=mount:path",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request("GET", path, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let mut missing = request("GET", "/api/v1/automation/notifications", "");
    missing.headers_mut().remove("cookie");
    assert_eq!(
        app.clone().oneshot(missing).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let path = format!(
        "/api/v1/automation/notifications/{}/acknowledgment",
        "d".repeat(64)
    );
    let mut csrf = request("POST", &path, r#"{"revision":1}"#);
    csrf.headers_mut().remove("x-csrf-token");
    assert_eq!(
        app.clone().oneshot(csrf).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    for body in [
        r#"{"revision":0}"#,
        r#"{"revision":1,"owner":"other"}"#,
        r#"{"revision":1,"delivered":true}"#,
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request("POST", &path, body))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    for path in [
        "/api/v1/automation/alerts",
        "/api/v1/automation/notifications",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request("POST", path, "{}"))
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    for path in [
        "/api/v1/automation/tick",
        "/api/v1/automation/notifications/delivery",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request("POST", path, "{}"))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn invalid_backend_rows_order_cursor_and_size_are_never_exposed() {
    let mut cases = vec![];
    let mut bad = stub();
    bad.alerts = vec![alert(); 33];
    cases.push((bad, "/api/v1/automation/alerts"));
    let mut bad = stub();
    bad.alerts.push(alert());
    cases.push((bad, "/api/v1/automation/alerts"));
    let mut bad = stub();
    bad.next = Some("another:key".into());
    cases.push((bad, "/api/v1/automation/alerts"));
    let mut bad = stub();
    bad.alerts[0].last_sequence = -1;
    cases.push((bad, "/api/v1/automation/alerts"));
    let mut bad = stub();
    bad.notes[0].id = "short".into();
    cases.push((bad, "/api/v1/automation/notifications"));
    let mut bad = stub();
    bad.notes[0].channel = DeliveryChannel::External;
    cases.push((bad, "/api/v1/automation/notifications"));
    let mut bad = stub();
    bad.notes[0].delivery = DeliveryState::Pending;
    bad.notes[0].attempts = 0;
    cases.push((bad, "/api/v1/automation/notifications"));
    for (backend, path) in cases {
        let response = router(backend, "https://localhost".into())
            .oneshot(request("GET", path, ""))
            .await
            .unwrap();
        assert_eq!(value(response).await["code"], "state_not_durable");
    }
    assert!(bounded("x".repeat(64 * 1024)).is_err());
}
