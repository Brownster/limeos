use super::*;
use axum::body::Body;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

const PATH: &str = "/api/v1/auth/capabilities/observe-health/enable";
#[derive(Clone, Default)]
struct RouteStub {
    calls: Arc<AtomicUsize>,
}
impl Backend for RouteStub {
    async fn login(&self, _: Login, _: PasswordAttempt) -> PasswordChecked<IssuedSession> {
        PasswordChecked::rejected()
    }
    async fn session(&self, token: String, csrf: Option<String>) -> Result<SessionView> {
        if token != "a".repeat(64) || csrf != Some("c".repeat(64)) {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        Ok(SessionView {
            principal: limeos_domain::Principal {
                id: "p".repeat(64),
                role: limeos_domain::Role::Administrator,
                grant_revision: 1,
            },
            csrf_token: "c".repeat(64),
            expires_at: 999,
        })
    }
    async fn logout(&self, _: String, _: String) -> Result<()> {
        panic!("unrelated route")
    }
    async fn health(&self) -> Result<Health> {
        Err(Error(ErrorCode::Unavailable))
    }
    async fn enable_observe_health(
        &self,
        token: String,
        csrf: String,
        input: limeos_contracts::ObserveHealthEnable,
        deadline: Instant,
        _: PasswordAttempt,
    ) -> PasswordChecked<limeos_contracts::ObserveHealthEnabled> {
        // Transport stub only. Real password and transaction coverage is in Core.
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(token, "a".repeat(64));
        assert_eq!(csrf, "c".repeat(64));
        assert!(deadline > Instant::now() && deadline <= Instant::now() + Duration::from_secs(10));
        match input.password.as_str() {
            "wrong" => PasswordChecked::rejected(),
            // Verified, then the grant changed underneath: a refusal after the match.
            "raced" => {
                PasswordChecked::new(PasswordCheck::Accepted, Err(Error(ErrorCode::Conflict)))
            }
            password => PasswordChecked::new(
                PasswordCheck::Accepted,
                Ok(limeos_contracts::ObserveHealthEnabled {
                    enabled: true,
                    reauthenticate: password != "noop",
                }),
            ),
        }
    }
}
fn request(body: &str) -> Request {
    Request::builder()
        .method("POST")
        .uri(PATH)
        .header("origin", "https://localhost")
        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
        .header("x-csrf-token", "c".repeat(64))
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn fixed_body_and_request_method_cannot_select_authority_or_bypass_credentials() {
    let stub = RouteStub::default();
    let app = router(stub.clone(), "https://localhost".into());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(PATH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    for field in [
        "principal",
        "username",
        "role",
        "scopes",
        "revision",
        "verified",
        "deadline",
        "enabled",
    ] {
        let body = format!("{{\"password\":\"correct\",\"{field}\":true}}");
        assert_eq!(
            app.clone().oneshot(request(&body)).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
    let stub = RouteStub::default();
    let app = router(stub.clone(), "https://localhost".into());
    for (header, status) in [
        ("origin", StatusCode::FORBIDDEN),
        ("cookie", StatusCode::UNAUTHORIZED),
        ("x-csrf-token", StatusCode::FORBIDDEN),
    ] {
        let mut value = request(r#"{"password":"correct"}"#);
        value.headers_mut().remove(header);
        assert_eq!(app.clone().oneshot(value).await.unwrap().status(), status);
    }
    for body in [
        "{}".to_owned(),
        "{bad".to_owned(),
        r#"{"password":""}"#.to_owned(),
        serde_json::json!({"password":"x".repeat(1025)}).to_string(),
        "x".repeat(16385),
    ] {
        assert_eq!(
            app.clone().oneshot(request(&body)).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn changed_outcome_clears_cookie_but_noop_retains_it_and_wrong_password_stays_401() {
    let stub = RouteStub::default();
    let app = router(stub, "https://localhost".into());
    for (password, status, clear) in [
        ("correct", StatusCode::OK, true),
        ("noop", StatusCode::OK, false),
        ("wrong", StatusCode::UNAUTHORIZED, false),
    ] {
        let response = app
            .clone()
            .oneshot(request(
                &serde_json::json!({"password":password}).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers().contains_key("set-cookie"), clear);
        if clear {
            assert!(
                response.headers()["set-cookie"]
                    .to_str()
                    .unwrap()
                    .contains("Max-Age=0")
            );
        }
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(password));
    }
}

#[tokio::test]
async fn only_failed_enablement_passwords_spend_the_principal_budget() {
    let stub = RouteStub::default();
    let app = router(stub.clone(), "https://localhost".into());
    for _ in 0..12 {
        assert_eq!(
            app.clone()
                .oneshot(request(r#"{"password":"raced"}"#))
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        for body in ["{bad", r#"{"password":""}"#] {
            assert_eq!(
                app.clone().oneshot(request(body)).await.unwrap().status(),
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            app.clone()
                .oneshot(request(r#"{"password":"noop"}"#))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    for _ in 0..10 {
        assert_eq!(
            app.clone()
                .oneshot(request(r#"{"password":"wrong"}"#))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(stub.calls.load(Ordering::SeqCst), 34);
    assert_eq!(
        app.clone()
            .oneshot(request(r#"{"password":"correct"}"#))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // The principal's budget is not the login budget of any username.
    let login = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("origin", "https://localhost")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"username":"alice","password":"wrong"}"#))
        .unwrap();
    assert_eq!(
        app.oneshot(login).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(stub.calls.load(Ordering::SeqCst), 34);
}
