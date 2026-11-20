use super::*;
use axum::body::Body;
use limeos_contracts::{ResolutionView, ResolveJobResponse};
use limeos_domain::{ContainerAction, DeclaredOutcome, JobState, ResolveKind};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

#[derive(Clone)]
struct Stub {
    reads: Arc<AtomicUsize>,
    decisions: Arc<AtomicUsize>,
    /// What the backend answers to a decision.
    answer: Arc<Mutex<Result<ResolveStatus>>>,
    /// How far its password check got; `None` follows core: a 401 is a wrong
    /// password and every other answer comes after a verified one.
    check: Arc<Mutex<Option<PasswordCheck>>>,
}
impl Stub {
    fn new(answer: Result<ResolveStatus>) -> Self {
        Self {
            reads: Arc::default(),
            decisions: Arc::default(),
            answer: Arc::new(Mutex::new(answer)),
            check: Arc::default(),
        }
    }
}
fn view() -> ResolutionView {
    ResolutionView {
        job: "e".repeat(64),
        state: JobState::NeedsIntervention,
        kind: ResolveKind::Container,
        operation: ContainerAction::Restart,
        targets: vec![],
        evidence_digest: None,
        allowed: vec![],
        members: None,
        pending: None,
        resolution: None,
    }
}
impl TestBackend for Stub {
    async fn login(&self, _: Login, _: PasswordAttempt) -> PasswordChecked<IssuedSession> {
        PasswordChecked::rejected()
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
    async fn job_resolution(&self, _: String, id: String) -> Result<ResolutionView> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        assert_eq!(id, "e".repeat(64));
        (*self.answer.lock().unwrap()).map(|_| view())
    }
    async fn needs_attention_jobs(
        &self,
        _: String,
    ) -> Result<Vec<limeos_domain::NeedsAttentionJob>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        (*self.answer.lock().unwrap()).map(|_| {
            vec![limeos_domain::NeedsAttentionJob {
                job: "e".repeat(64),
                owner: "bob".into(),
                kind: ResolveKind::Container,
                operation: ContainerAction::Restart,
                state: JobState::NeedsIntervention,
                resource: format!("container:{}", "a".repeat(64)),
                targets: vec![format!("container:{}", "a".repeat(64))],
                created_at: 100,
            }]
        })
    }
    async fn resolve_job(
        &self,
        _: String,
        _: String,
        id: String,
        input: limeos_contracts::ResolveJobInput,
        _: PasswordAttempt,
    ) -> PasswordChecked<ResolveJobResponse> {
        self.decisions.fetch_add(1, Ordering::SeqCst);
        assert_eq!(id, "e".repeat(64));
        assert_eq!(input.declared, DeclaredOutcome::NotDone);
        let answer = *self.answer.lock().unwrap();
        let check = self.check.lock().unwrap().unwrap_or(match answer {
            Err(Error(ErrorCode::Unauthenticated)) => PasswordCheck::Rejected,
            _ => PasswordCheck::Accepted,
        });
        PasswordChecked::new(
            check,
            answer.map(|status| ResolveJobResponse {
                status,
                message: "m".into(),
                resolution: None,
                view: (status == ResolveStatus::StaleEvidence).then(view),
                blocking: if status == ResolveStatus::UnsupportedDeclaration {
                    vec!["a".repeat(64)]
                } else {
                    vec![]
                },
            }),
        )
    }
}
fn path() -> String {
    format!("/api/v1/jobs/{}/resolution", "e".repeat(64))
}
fn body() -> String {
    serde_json::json!({
        "declared": "not_done",
        "reason": "Checked by hand.",
        "evidence_digest": "d".repeat(64),
        "password": "correct horse battery"
    })
    .to_string()
}
fn request(method: &str, guarded: bool) -> axum::http::request::Builder {
    let mut r = Request::builder()
        .method(method)
        .uri(path())
        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)));
    if guarded {
        r = r
            .header("origin", "https://localhost")
            .header("x-csrf-token", "b".repeat(64));
    }
    r
}
fn post(guarded: bool) -> Request {
    request("POST", guarded)
        .header("content-type", "application/json")
        .body(Body::from(body()))
        .unwrap()
}
async fn send(stub: &Stub, request: Request) -> (StatusCode, serde_json::Value) {
    let response = router(stub.clone(), "https://localhost".into())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn decisions_need_origin_csrf_json_and_a_closed_body_before_the_backend() {
    let stub = Stub::new(Ok(ResolveStatus::Resolved));
    // No Origin or CSRF header.
    assert_eq!(send(&stub, post(false)).await.0, StatusCode::FORBIDDEN);
    // Wrong CSRF token.
    let wrong = request("POST", false)
        .header("origin", "https://localhost")
        .header("x-csrf-token", "c".repeat(64))
        .header("content-type", "application/json")
        .body(Body::from(body()))
        .unwrap();
    assert_eq!(send(&stub, wrong).await.0, StatusCode::FORBIDDEN);
    // Not JSON.
    let text = request("POST", true)
        .header("content-type", "text/plain")
        .body(Body::from(body()))
        .unwrap();
    assert_eq!(send(&stub, text).await.0, StatusCode::BAD_REQUEST);
    // Unknown and missing fields.
    for broken in [
        body().replacen('{', "{\"force\":true,", 1),
        r#"{"declared":"not_done","reason":"x","evidence_digest":"d"}"#.into(),
        body().replace("not_done", "gone"),
    ] {
        let req = request("POST", true)
            .header("content-type", "application/json")
            .body(Body::from(broken))
            .unwrap();
        assert_eq!(send(&stub, req).await.0, StatusCode::BAD_REQUEST);
    }
    // A job ID that cannot exist.
    let bad = Request::builder()
        .method("POST")
        .uri("/api/v1/jobs/xyz/resolution")
        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
        .header("origin", "https://localhost")
        .header("x-csrf-token", "b".repeat(64))
        .header("content-type", "application/json")
        .body(Body::from(body()))
        .unwrap();
    assert_eq!(send(&stub, bad).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(stub.decisions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn only_the_session_cookie_is_a_credential() {
    let stub = Stub::new(Ok(ResolveStatus::Resolved));
    // There are no API keys: a key header is refused before the backend.
    let mut get = request("GET", false).body(Body::empty()).unwrap();
    get.headers_mut()
        .insert("x-api-key", "key".parse().unwrap());
    assert_eq!(send(&stub, get).await.0, StatusCode::FORBIDDEN);
    let mut decide = post(true);
    decide
        .headers_mut()
        .insert("x-api-key", "key".parse().unwrap());
    assert_eq!(send(&stub, decide).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        stub.reads.load(Ordering::SeqCst) + stub.decisions.load(Ordering::SeqCst),
        0
    );
    // An Authorization header is never a credential: alone it authenticates
    // nothing, and next to the cookie (a proxy's basic authentication adds
    // one) it changes nothing.
    for value in ["Bearer tok", "Basic YTpi"] {
        let anonymous = Request::builder()
            .uri(path())
            .header("authorization", value)
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&stub, anonymous).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(stub.reads.load(Ordering::SeqCst), 0);
    }
    let mut get = request("GET", false).body(Body::empty()).unwrap();
    get.headers_mut()
        .insert("authorization", "Basic YTpi".parse().unwrap());
    assert_eq!(send(&stub, get).await.0, StatusCode::OK);
    let mut decide = post(true);
    decide
        .headers_mut()
        .insert("authorization", "Basic YTpi".parse().unwrap());
    assert_eq!(send(&stub, decide).await.0, StatusCode::OK);
    assert_eq!(stub.reads.load(Ordering::SeqCst), 1);
    assert_eq!(stub.decisions.load(Ordering::SeqCst), 1);
    let none = Request::builder().uri(path()).body(Body::empty()).unwrap();
    assert_eq!(send(&stub, none).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn backend_refusals_keep_their_status_and_the_four_outcomes_are_distinct() {
    for (answer, expected) in [
        (Err(Error(ErrorCode::Forbidden)), StatusCode::FORBIDDEN),
        (
            Err(Error(ErrorCode::Unauthenticated)),
            StatusCode::UNAUTHORIZED,
        ),
        (Err(Error(ErrorCode::Conflict)), StatusCode::CONFLICT),
        (Err(Error(ErrorCode::NotFound)), StatusCode::NOT_FOUND),
        (
            Err(Error(ErrorCode::Unavailable)),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (Ok(ResolveStatus::Resolved), StatusCode::OK),
        (Ok(ResolveStatus::Pending), StatusCode::ACCEPTED),
        (Ok(ResolveStatus::StaleEvidence), StatusCode::CONFLICT),
        (
            Ok(ResolveStatus::UnsupportedDeclaration),
            StatusCode::CONFLICT,
        ),
    ] {
        let stub = Stub::new(answer);
        let (status, json) = send(&stub, post(true)).await;
        assert_eq!(status, expected, "{answer:?}");
        if let Ok(status) = answer {
            assert_eq!(
                json["status"],
                serde_json::to_value(status).unwrap(),
                "{status:?}"
            );
            assert!(json["blocking"].is_array());
            assert_eq!(
                json["view"].is_object(),
                status == ResolveStatus::StaleEvidence
            );
        } else {
            assert!(json["code"].is_string() && json["message"].is_string());
        }
    }
}

#[tokio::test]
async fn only_failed_password_decisions_spend_the_principal_budget() {
    let stub = Stub::new(Ok(ResolveStatus::Resolved));
    let app = router(stub.clone(), "https://localhost".into());
    for _ in 0..12 {
        let junk = request("POST", true)
            .header("content-type", "application/json")
            .body(Body::from("{bad"))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(junk).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    for _ in 0..12 {
        assert_eq!(
            app.clone().oneshot(post(true)).await.unwrap().status(),
            StatusCode::OK
        );
    }
    *stub.answer.lock().unwrap() = Err(Error(ErrorCode::Unauthenticated));
    for _ in 0..10 {
        assert_eq!(
            app.clone().oneshot(post(true)).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        app.clone().oneshot(post(true)).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(stub.decisions.load(Ordering::SeqCst), 22);
}

#[tokio::test]
async fn malformed_passwords_are_refused_before_the_principal_budget() {
    let stub = Stub::new(Err(Error(ErrorCode::Unauthenticated)));
    let app = router(stub.clone(), "https://localhost".into());
    for password in ["".to_owned(), "x".repeat(1025)] {
        for _ in 0..12 {
            let body = serde_json::json!({
                "declared": "not_done",
                "reason": "Checked by hand.",
                "evidence_digest": "d".repeat(64),
                "password": password,
            });
            let request = request("POST", true)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            assert_eq!(
                app.clone().oneshot(request).await.unwrap().status(),
                StatusCode::BAD_REQUEST
            );
        }
    }
    assert_eq!(stub.decisions.load(Ordering::SeqCst), 0);
    for _ in 0..10 {
        assert_eq!(
            app.clone().oneshot(post(true)).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn a_verified_password_costs_nothing_when_the_decision_then_fails() {
    // Core verifies the password first; these failures come after it.
    for (code, expected) in [
        (ErrorCode::NotFound, StatusCode::NOT_FOUND),
        (ErrorCode::Conflict, StatusCode::CONFLICT),
        (ErrorCode::Unavailable, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let stub = Stub::new(Err(Error(code)));
        let app = router(stub.clone(), "https://localhost".into());
        for _ in 0..15 {
            assert_eq!(
                app.clone().oneshot(post(true)).await.unwrap().status(),
                expected,
                "{code:?}"
            );
        }
    }
}

#[tokio::test]
async fn an_unreached_check_is_free_and_an_accepted_check_answering_401_is_charged() {
    let stub = Stub::new(Err(Error(ErrorCode::Overloaded)));
    *stub.check.lock().unwrap() = Some(PasswordCheck::NotReached);
    let app = router(stub.clone(), "https://localhost".into());
    for _ in 0..15 {
        assert_eq!(
            app.clone().oneshot(post(true)).await.unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    assert_eq!(stub.decisions.load(Ordering::SeqCst), 15);
    // Correct, but the session closed during the check: indistinguishable
    // from a wrong password by its 401, so the budget must not differ either.
    *stub.answer.lock().unwrap() = Err(Error(ErrorCode::Unauthenticated));
    *stub.check.lock().unwrap() = Some(PasswordCheck::Accepted);
    for _ in 0..10 {
        assert_eq!(
            app.clone().oneshot(post(true)).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(stub.decisions.load(Ordering::SeqCst), 25);
    assert_eq!(
        app.clone().oneshot(post(true)).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(stub.decisions.load(Ordering::SeqCst), 25);
}

#[tokio::test]
async fn password_attempts_share_the_principal_rate_limit() {
    let stub = Stub::new(Err(Error(ErrorCode::Unauthenticated)));
    let app = router(stub.clone(), "https://localhost".into());
    let mut statuses = Vec::new();
    for _ in 0..12 {
        statuses.push(app.clone().oneshot(post(true)).await.unwrap().status());
    }
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::UNAUTHORIZED)
            .count(),
        10
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::TOO_MANY_REQUESTS)
            .count(),
        2
    );
    assert_eq!(stub.decisions.load(Ordering::SeqCst), 10);
}

#[tokio::test]
async fn reads_are_cookie_gets_and_the_route_has_no_other_method() {
    let stub = Stub::new(Ok(ResolveStatus::Resolved));
    let get = request("GET", false).body(Body::empty()).unwrap();
    let (status, json) = send(&stub, get).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["state"], "needs_intervention");
    assert_eq!(json["kind"], "container");
    assert_eq!(stub.reads.load(Ordering::SeqCst), 1);
    let bad = Request::builder()
        .uri("/api/v1/jobs/xyz/resolution")
        .header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&stub, bad).await.0, StatusCode::BAD_REQUEST);
    for method in ["PUT", "PATCH", "DELETE"] {
        let req = request(method, true).body(Body::empty()).unwrap();
        assert_eq!(
            send(&stub, req).await.0,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method}"
        );
    }
    assert_eq!(stub.decisions.load(Ordering::SeqCst), 0);
}

fn attention(cookie: bool) -> axum::http::request::Builder {
    let r = Request::builder().uri("/api/v1/jobs/needs-attention");
    if cookie {
        r.header("cookie", format!("__Host-limeos={}", "a".repeat(64)))
    } else {
        r
    }
}

#[tokio::test]
async fn the_attention_listing_is_a_cookie_only_read_with_the_backends_verdict() {
    let stub = Stub::new(Ok(ResolveStatus::Resolved));
    let (status, json) = send(&stub, attention(true).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json[0]["owner"], "bob");
    assert_eq!(json[0]["state"], "needs_intervention");
    assert_eq!(json[0]["kind"], "container");
    assert_eq!(stub.reads.load(Ordering::SeqCst), 1);
    // No session cookie: unauthenticated, and the backend is not asked.
    let none = attention(false).body(Body::empty()).unwrap();
    assert_eq!(send(&stub, none).await.0, StatusCode::UNAUTHORIZED);
    // Keys are refused whether or not a cookie comes with them; an
    // Authorization header is ignored either way.
    for cookie in [true, false] {
        let mut get = attention(cookie).body(Body::empty()).unwrap();
        get.headers_mut()
            .insert("x-api-key", "key".parse().unwrap());
        assert_eq!(send(&stub, get).await.0, StatusCode::FORBIDDEN);
    }
    for value in ["Bearer tok", "Basic YTpi"] {
        let mut get = attention(false).body(Body::empty()).unwrap();
        get.headers_mut()
            .insert("authorization", value.parse().unwrap());
        assert_eq!(send(&stub, get).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(stub.reads.load(Ordering::SeqCst), 1);
    let mut get = attention(true).body(Body::empty()).unwrap();
    get.headers_mut()
        .insert("authorization", "Basic YTpi".parse().unwrap());
    assert_eq!(send(&stub, get).await.0, StatusCode::OK);
    assert_eq!(stub.reads.load(Ordering::SeqCst), 2);
    // Reads only.
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        let req = attention(true).method(method).body(Body::empty()).unwrap();
        assert_eq!(
            send(&stub, req).await.0,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method}"
        );
    }
    assert_eq!(stub.reads.load(Ordering::SeqCst), 2);
    // A non-administrator session is refused by the backend; so is no session.
    for (answer, expected) in [
        (Err(Error(ErrorCode::Forbidden)), StatusCode::FORBIDDEN),
        (
            Err(Error(ErrorCode::Unauthenticated)),
            StatusCode::UNAUTHORIZED,
        ),
        (
            Err(Error(ErrorCode::Unavailable)),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    ] {
        let stub = Stub::new(answer);
        let (status, json) = send(&stub, attention(true).body(Body::empty()).unwrap()).await;
        assert_eq!(status, expected);
        assert!(json["code"].is_string());
    }
}
