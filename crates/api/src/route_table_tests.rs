//! Walks every route in the table the router is built from. A route added
//! later without the gate, or one that skips the backend's own session,
//! CSRF and scope check, fails here. The backend here is a fake that does
//! the refusing itself; core's real check is swept route by route in
//! `bins/core/src/tests/route_sweep.rs`.
use super::gate::{Access, table};
use super::*;
use axum::{body::Body, http::Method};
use std::sync::Mutex as StdMutex;
use tower::ServiceExt;

const ORIGIN: &str = "https://localhost";
const CSRF: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
/// A signed-in principal whose grants cover none of these routes.
const VIEWER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ID: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

/// One backend call: method, token and CSRF value handed over.
type Call = (String, Option<String>, Option<String>);
/// Records every backend call with the credentials it was handed and, as
/// core does in the same store call, refuses the viewer: insufficient scope.
#[derive(Clone, Default)]
struct Gatekeeper {
    calls: Arc<StdMutex<Vec<Call>>>,
}
impl Gatekeeper {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }
    fn clear(&self) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.clear();
        }
    }
}
impl TestBackend for Gatekeeper {
    fn fallback(&self, method: &'static str, token: Option<&str>, csrf: Option<&str>) -> Error {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push((method.into(), token.map(Into::into), csrf.map(Into::into)));
        }
        if token == Some(VIEWER) {
            Error(ErrorCode::Forbidden)
        } else {
            Error(ErrorCode::Unauthenticated)
        }
    }
    async fn login(&self, _: Login, _: PasswordAttempt) -> PasswordChecked<IssuedSession> {
        PasswordChecked::rejected()
    }
    /// A live viewer session: password routes read it for the budget key.
    async fn session(&self, token: String, csrf: Option<String>) -> Result<SessionView> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(("session".into(), Some(token.clone()), csrf));
        }
        if token != VIEWER {
            return Err(Error(ErrorCode::Unauthenticated));
        }
        Ok(SessionView {
            principal: limeos_domain::Principal {
                id: "viewer".into(),
                role: limeos_domain::Role::Viewer,
                grant_revision: 1,
            },
            csrf_token: CSRF.into(),
            expires_at: 999,
        })
    }
    async fn logout(&self, token: String, csrf: String) -> Result<()> {
        Err(self.fallback("logout", Some(&token), Some(&csrf)))
    }
    async fn health(&self) -> Result<Health> {
        Err(Error(ErrorCode::Unavailable))
    }
}

fn storage_contract() -> serde_json::Value {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    fixture["cases"][0]["contract"].clone()
}
/// A whole reviewed storage-target proposal, as the queue route takes it.
fn storage_target_queue() -> String {
    let contract: limeos_domain::StorageContract =
        serde_json::from_value(storage_contract()).unwrap();
    let device = &contract.devices[0];
    let preparation = limeos_domain::TargetPreparationPlan {
        version: 1,
        operation: limeos_domain::STORAGE_TARGET_OPERATION.into(),
        action: "c".repeat(64),
        expected: limeos_domain::StorageInventory {
            host_root_mount_id: 1,
            topology_digest: "a".repeat(64),
            mounts_digest: "b".repeat(64),
            fstab_digest: "c".repeat(64),
            fstab_entries: vec![],
            devices: vec![limeos_domain::StorageObservedDevice {
                device: limeos_domain::StorageBlockDevice { major: 8, minor: 1 },
                filesystem_uuid: device.filesystem_uuid.clone(),
                filesystem: "ext4".into(),
                serial: None,
                boot_backing: false,
                in_use_as_swap: false,
                mounts: vec![],
            }],
        },
        targets: vec![limeos_domain::TargetEvidence {
            mountpoint: device.mountpoint.clone(),
            parent: limeos_domain::DirectoryIdentity {
                major: 8,
                minor: 0,
                mount_id: 1,
                inode: 5,
                mode: 0o40755,
                uid: 0,
                gid: 0,
            },
            existing: None,
        }],
        contract: contract.clone(),
        created_at: 200,
        expires_at: 500,
    };
    serde_json::to_string(&limeos_contracts::QueueStorageTargetsInput {
        proposal: limeos_domain::PlannedStorageTargets {
            plan: limeos_domain::StorageTargetPlan {
                id: "f".repeat(64),
                version: limeos_domain::STORAGE_TARGET_VERSION,
                principal: "admin".into(),
                grant_revision: 1,
                preparation,
            },
            digest: "e".repeat(64),
        },
        approval: "c".repeat(64),
        idempotency_key: "key".into(),
    })
    .unwrap()
}
/// A body each mutating route accepts, so the request reaches the backend.
/// A new mutating route needs an entry here; the test names it otherwise.
fn sample_body(path: &str) -> Option<String> {
    let approval = format!(r#"{{"digest":"{}"}}"#, "e".repeat(64));
    let body = match path {
        "/api/v1/storage/mounts/plans" => r#"{"target_id":"data"}"#.into(),
        "/api/v1/storage/mounts/plans/{id}/approval"
        | "/api/v1/storage/targets/plans/{id}/approval"
        | "/api/v1/storage/plans/{id}/approval"
        | "/api/v1/compose/plans/{id}/approval"
        | "/api/v1/container/plans/{id}/approval"
        | "/api/v1/container/restart/plans/{id}/approval"
        | "/api/v1/stack/plans/{id}/approval" => approval,
        "/api/v1/storage/plans" => {
            serde_json::json!({"contract": storage_contract(), "inventory_digest": "a".repeat(64), "timeout_seconds": 10}).to_string()
        }
        "/api/v1/auth/capabilities/observe-health/enable" => r#"{"password":"correct"}"#.into(),
        "/api/v1/compose/plans" => r#"{"stack":"media","template":"standard"}"#.into(),
        "/api/v1/container/restart/plans" => {
            format!(r#"{{"resource":"container:{}"}}"#, "b".repeat(64))
        }
        "/api/v1/container/plans" => format!(
            r#"{{"operation":"restart","resource":"container:{}"}}"#,
            "b".repeat(64)
        ),
        "/api/v1/stack/plans" => r#"{"project":"media","operation":"restart"}"#.into(),
        "/api/v1/jobs/{id}/resolution" => serde_json::json!({
            "declared": "verified_done",
            "reason": "checked by hand",
            "evidence_digest": "f".repeat(64),
            "password": "correct",
        })
        .to_string(),
        "/api/v1/automation/schedules" => {
            r#"{"interval_seconds":60,"grace_seconds":10,"missed":"coalesce_latest","window":null}"#
                .into()
        }
        "/api/v1/automation/schedules/{id}/pause" => r#"{"revision":1,"paused":true}"#.into(),
        "/api/v1/automation/notifications/{id}/acknowledgment" => r#"{"revision":1}"#.into(),
        "/api/v1/automation/health-policy" => {
            serde_json::to_string(&limeos_domain::automation::ReplaceHealthPolicy {
                revision: 0,
                definition: Default::default(),
            })
            .unwrap()
        }
        "/api/v1/storage/targets/plans" => storage_contract().to_string(),
        "/api/v1/storage/targets/jobs" => storage_target_queue(),
        "/api/v1/storage/mounts/jobs" => {
            serde_json::to_string(&super::operation_tests::mount_queue_input()).unwrap()
        }
        "/api/v1/stack/jobs" => super::stack_operation_tests::queue_body(),
        "/api/v1/container/jobs" | "/api/v1/container/restart/jobs" => serde_json::json!({
            "proposal": {"plan": {
                "id": "f".repeat(64), "version": 1, "principal": "admin", "grant_revision": 1,
                "expected": {"resource": format!("container:{}", "b".repeat(64)),
                    "image": format!("sha256:{}", "c".repeat(64)),
                    "started_at": "2026-10-05T07:00:00Z", "running": true},
                "created_at": 100, "expires_at": 400}, "digest": "e".repeat(64)},
            "approval": "c".repeat(64),
            "idempotency_key": "key",
        })
        .to_string(),
        _ => return None,
    };
    Some(body)
}
/// Routes whose request reaches the backend with no body at all.
fn bodiless(path: &str) -> bool {
    path.ends_with("/cancel") || path == "/api/v1/auth/logout"
}
/// Query strings a read needs before it reaches the backend.
fn sample_query(path: &str) -> &'static str {
    match path {
        "/api/v1/system/history" => "?range=24h",
        _ => "",
    }
}

fn request(
    method: &Method,
    path: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    csrf: Option<&str>,
    body: Option<String>,
) -> Request {
    let mut builder = Request::builder()
        .method(method.clone())
        .uri(path.replace("{id}", ID));
    if let Some(token) = cookie {
        builder = builder.header("cookie", format!("__Host-limeos={token}"));
    }
    if let Some(origin) = origin {
        builder = builder.header("origin", origin);
    }
    if let Some(csrf) = csrf {
        builder = builder.header("x-csrf-token", csrf);
    }
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(body.map(Body::from).unwrap_or_else(Body::empty))
        .unwrap()
}
/// Status and error envelope code; panics on anything but a JSON envelope.
async fn send(backend: &Gatekeeper, request: Request) -> (StatusCode, ErrorCode) {
    let response = router(backend.clone(), ORIGIN.into())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let kind = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    assert!(
        kind.starts_with("application/json"),
        "{status} {kind}: {}",
        String::from_utf8_lossy(&bytes)
    );
    let envelope: ErrorEnvelope = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("not an error envelope: {}", String::from_utf8_lossy(&bytes)));
    (status, envelope.code)
}

#[test]
fn only_health_and_login_are_public() {
    let public: Vec<_> = table::<Gatekeeper>()
        .into_iter()
        .filter(|e| e.access == Access::Public)
        .map(|e| (e.method, e.path))
        .collect();
    assert_eq!(
        public,
        [
            (Method::GET, "/api/v1/health"),
            (Method::POST, "/api/v1/auth/login"),
        ]
    );
    // A mutation is never mounted as a read, and a read never as a mutation.
    for entry in table::<Gatekeeper>() {
        match entry.access {
            Access::Read => assert_eq!(entry.method, Method::GET, "{}", entry.path),
            Access::Mutate => assert_eq!(entry.method, Method::POST, "{}", entry.path),
            Access::Public => {}
        }
    }
}

#[tokio::test]
async fn every_mutating_route_refuses_no_session_no_csrf_a_foreign_origin_and_no_scope() {
    let backend = Gatekeeper::default();
    let mut checked = 0;
    for entry in table::<Gatekeeper>() {
        if entry.access != Access::Mutate {
            continue;
        }
        let (method, path) = (&entry.method, entry.path);
        let body = if bodiless(path) {
            None
        } else {
            Some(sample_body(path).unwrap_or_else(|| panic!("add a sample body for {path}")))
        };
        let gate_cases = [
            (
                None,
                Some(ORIGIN),
                Some(CSRF),
                ErrorCode::Unauthenticated,
                "no session",
            ),
            (
                Some(VIEWER),
                Some(ORIGIN),
                None,
                ErrorCode::Forbidden,
                "no CSRF",
            ),
            (
                Some(VIEWER),
                Some("https://foreign.invalid"),
                Some(CSRF),
                ErrorCode::Forbidden,
                "foreign Origin",
            ),
            (
                Some(VIEWER),
                None,
                Some(CSRF),
                ErrorCode::Forbidden,
                "no Origin",
            ),
        ];
        for (cookie, origin, csrf, code, case) in gate_cases {
            backend.clear();
            let sent = request(method, path, cookie, origin, csrf, body.clone());
            let (status, got) = send(&backend, sent).await;
            assert_eq!(got, code, "{path}: {case} ({status})");
            assert!(
                backend.calls().is_empty(),
                "{path}: {case} reached the backend"
            );
        }
        // Past the gate, the backend decides session, CSRF and scope in the
        // call itself, handed exactly the caller's credentials.
        backend.clear();
        let sent = request(
            method,
            path,
            Some(VIEWER),
            Some(ORIGIN),
            Some(CSRF),
            body.clone(),
        );
        let (status, got) = send(&backend, sent).await;
        assert_eq!(
            (status, got),
            (StatusCode::FORBIDDEN, ErrorCode::Forbidden),
            "{path}: no scope"
        );
        let calls = backend.calls();
        let decided = calls
            .iter()
            .filter(|(m, ..)| m != "session")
            .collect::<Vec<_>>();
        assert_eq!(decided.len(), 1, "{path}: {calls:?}");
        assert_eq!(decided[0].1.as_deref(), Some(VIEWER), "{path}");
        assert_eq!(decided[0].2.as_deref(), Some(CSRF), "{path}");
        checked += 1;
    }
    // 38 today: catches an empty walk.
    assert!(checked >= 30, "only {checked} mutating routes");
}

#[tokio::test]
async fn every_read_needs_the_session_cookie_and_hands_its_token_to_the_backend() {
    let backend = Gatekeeper::default();
    let mut checked = 0;
    for entry in table::<Gatekeeper>() {
        if entry.access != Access::Read {
            continue;
        }
        let path = format!("{}{}", entry.path, sample_query(entry.path));
        backend.clear();
        let (_, got) = send(
            &backend,
            request(&entry.method, &path, None, None, None, None),
        )
        .await;
        assert_eq!(got, ErrorCode::Unauthenticated, "{path}");
        assert!(backend.calls().is_empty(), "{path}");
        // A bearer or key header is never a credential, cookie or not.
        let mut bearer = request(&entry.method, &path, Some(VIEWER), None, None, None);
        bearer
            .headers_mut()
            .insert("authorization", HeaderValue::from_static("Bearer x"));
        assert_eq!(
            send(&backend, bearer).await.1,
            ErrorCode::Forbidden,
            "{path}"
        );
        assert!(backend.calls().is_empty(), "{path}");
        let response = router(backend.clone(), ORIGIN.into())
            .oneshot(request(
                &entry.method,
                &path,
                Some(VIEWER),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        let calls = backend.calls();
        assert!(!calls.is_empty(), "{path} never asked the backend");
        assert!(
            calls
                .iter()
                .all(|(_, token, _)| token.as_deref() == Some(VIEWER)),
            "{path}"
        );
        if entry.path != "/api/v1/auth/session" {
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        }
        checked += 1;
    }
    assert!(checked >= 25, "only {checked} reads");
}

#[tokio::test]
async fn no_route_answers_a_bad_path_or_query_with_plain_text() {
    let backend = Gatekeeper::default();
    for entry in table::<Gatekeeper>() {
        let malformed = [
            entry.path.replace("{id}", "%FF%FE"),
            format!(
                "{}?after=x&tail=x&offset=-1&range=x&unknown=%FF",
                entry.path
            ),
        ];
        for path in malformed {
            let mut sent = request(
                &entry.method,
                "/",
                Some(VIEWER),
                Some(ORIGIN),
                Some(CSRF),
                Some("{}".into()),
            );
            *sent.uri_mut() = path.replace("{id}", ID).parse().unwrap();
            let response = router(backend.clone(), ORIGIN.into())
                .oneshot(sent)
                .await
                .unwrap();
            let status = response.status();
            let kind = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
            // JSON always; on any refusal, the error envelope.
            assert!(
                kind.starts_with("application/json"),
                "{path}: {status} {kind}"
            );
            if !status.is_success() {
                let envelope: ErrorEnvelope = serde_json::from_slice(&bytes).unwrap();
                if path.contains("%FF%FE") {
                    assert_eq!(envelope.code, ErrorCode::InvalidInput, "{path}");
                }
            }
        }
    }
}
