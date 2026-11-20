//! Core's own authentication, swept over every route the API mounts.
//!
//! The API's gate only checks that a credential is present and well formed;
//! the session, its CSRF binding and the principal's scope are decided by
//! core in the call that does the work. So this sweep drives the real router
//! (`limeos_api::respond`) with the real `Core` and a real store, over the
//! route list read from the table the router is built from
//! (`limeos_api::routes`). A route added later is swept automatically; a new
//! mutation fails here until it has a body fixture.
use crate::*;
use limeos_api::{Access, Route};
use limeos_domain::{JobState, Role};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

const ORIGIN: &str = "https://localhost";
const ID: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
/// Well formed, and no session has it.
const UNKNOWN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
/// Well formed, and no session's CSRF value.
const WRONG_CSRF: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const PASSWORD: &str = "legacy-password";
/// What Caddy's `basic_auth` in front of LimeOS forwards on every request.
const PROXY_BASIC: &str = "Basic b3BlcmF0b3I6aHVudGVyMg==";

#[derive(Clone)]
struct Session {
    token: String,
    csrf: String,
}
impl Session {
    fn new(fill: char) -> Self {
        let token = fill.to_string().repeat(64);
        let csrf = limeos_identity::digest(&format!("csrf:{token}"));
        Self { token, csrf }
    }
}
struct Sweep {
    _dir: tempfile::TempDir,
    db: std::path::PathBuf,
    core: Core,
    /// The enrolled administrator.
    admin: Session,
    /// The administrator's second session, spent on the logout positive case.
    spare: Session,
    /// A signed-in viewer with no grant at all.
    viewer: Session,
    /// The viewer's second session, spent on the logout case.
    viewer_spare: Session,
    /// The viewer's principal id.
    viewer_id: String,
    /// Another principal's plans, jobs and session.
    other: Others,
}
/// What another principal (an operator) holds: a container plan and a
/// container job left `needs_intervention`, a stack plan and its queued
/// job, a compose plan and a session. No identifier of theirs, nor the
/// administrator's principal id, may appear in any answer to the viewer.
struct Others {
    container_plan: String,
    container_job: String,
    stack_plan: String,
    stack_job: String,
    compose_plan: String,
    /// Every identifier above, the operator's principal id, username,
    /// session token and its digests, and the administrator's id.
    identifiers: Vec<String>,
}
impl Others {
    /// The other principal's identifiers that `body` contains.
    fn leaked(&self, body: &str) -> Vec<String> {
        self.identifiers
            .iter()
            .filter(|id| body.contains(id.as_str()))
            .cloned()
            .collect()
    }
}

/// A core with a real store, an administrator, a viewer without scope, and
/// with or without an installed compose catalog (state an unauthenticated
/// caller must not be able to observe).
async fn fixture(catalog: bool) -> Sweep {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let db = Database::open(&path).unwrap();
    let (admin, spare, viewer) = (Session::new('a'), Session::new('f'), Session::new('b'));
    let viewer_spare = Session::new('9');
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/werkzeug-hashes.json"
    ))
    .unwrap();
    // Both accounts' password is PASSWORD, so password-gated routes reach
    // their own decision for the administrator.
    let encoded = fixtures[0]["hash"].as_str().unwrap().to_owned();
    assert_eq!(fixtures[0]["password"], PASSWORD);
    let sessions = [&admin, &spare, &viewer, &viewer_spare].map(|s| {
        (
            limeos_identity::digest(&s.token),
            limeos_identity::digest(&s.csrf),
        )
    });
    let other_encoded = encoded.clone();
    let (admin_id, viewer_id) = db
        .call(move |s| {
            let admin = s.import_legacy_user("admin", &encoded, now())?;
            s.revise_grants(&admin.id, Role::Administrator, &every_grant(), now())?;
            let record = s.login_record("admin")?.unwrap();
            for (digest, csrf) in &sessions[..2] {
                s.create_session(&record, None, digest, csrf, now())?;
            }
            let viewer = s.import_legacy_user("viewer", &encoded, now())?;
            s.revise_grants(&viewer.id, Role::Viewer, &[], now())?;
            let record = s.login_record("viewer")?.unwrap();
            for (digest, csrf) in &sessions[2..] {
                s.create_session(&record, None, digest, csrf, now())?;
            }
            Ok((admin.id, viewer.id))
        })
        .await
        .unwrap();
    let mut parsed: limeos_domain::ComposeCatalog = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/compose-catalog.json"
    ))
    .unwrap();
    parsed.normalize().unwrap();
    let other = others(&db, other_encoded, admin_id, parsed.clone()).await;
    let compose_catalog = catalog.then(|| Arc::new(parsed));
    let core = Core {
        automation_status: Default::default(),
        reader_health: Default::default(),
        db,
        password_workers: Arc::new(PasswordWorkers::new(2)),
        dummy_hash: "unused".into(),
        observations: limeos_observations::Cache::default(),
        telemetry: None,
        container_socket: Arc::new("/nonexistent/container.sock".into()),
        compose_catalog,
        storage_socket: Arc::new("/nonexistent/storage.sock".into()),
        storage_target_socket: Arc::new("/nonexistent/targets.sock".into()),
        storage_mount_socket: Arc::new("/nonexistent/mounts.sock".into()),
        storage_readers: Arc::new(Semaphore::new(1)),
        claims_in_doubt: Default::default(),
        stacks: Default::default(),
    };
    // A fresh stack inventory, so stack planning reaches its scope check.
    let stacks = crate::stack_operation_tests::cohort(
        &crate::stack_operation_tests::ordinary(),
        &["web", "db"],
    );
    core.stacks.update(&stacks).unwrap();
    Sweep {
        _dir: dir,
        db: path,
        core,
        admin,
        spare,
        viewer,
        viewer_spare,
        viewer_id,
        other,
    }
}

/// Seeds another principal's objects (see `Others`).
async fn others(
    db: &Database,
    encoded: String,
    admin_id: String,
    catalog: limeos_domain::ComposeCatalog,
) -> Others {
    use limeos_domain::{ContainerAction, ContainerSnapshot, StackSnapshot};
    let session = Session::new('7');
    let (digest, csrf) = (
        limeos_identity::digest(&session.token),
        limeos_identity::digest(&session.csrf),
    );
    let snapshot = |c: char| ContainerSnapshot {
        resource: format!("container:{}", c.to_string().repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running: true,
    };
    let (session_digest, session_csrf) = (digest.clone(), csrf.clone());
    let seeded = db
        .call(move |s| {
            let other = s.import_legacy_user("otheroperator", &encoded, now())?;
            s.revise_grants(&other.id, Role::Operator, &every_grant(), now())?;
            let record = s.login_record("otheroperator")?.unwrap();
            s.create_session(&record, None, &session_digest, &session_csrf, now())?;
            let p = record.principal;
            // A container job claimed and left needing an administrator.
            let current = snapshot('1');
            let plan = s.plan_container(&p, ContainerAction::Restart, &current, now())?;
            let approved = s.approve_container(&p, &plan.plan.id, &plan.digest, now())?;
            let job =
                s.queue_container(&p, "other-job", &plan, &approved.token, &current, now())?;
            s.claim_container(&p, &job.id, &current, &"1".repeat(64), now())?;
            s.transition(
                &job.id,
                JobState::Running,
                JobState::NeedsIntervention,
                now(),
            )?;
            // An open container plan.
            let open = s.plan_container(&p, ContainerAction::Restart, &snapshot('2'), now())?;
            // A queued stack job, and an open stack plan.
            let members = StackSnapshot::new("otherstack", vec![snapshot('3'), snapshot('4')])?;
            let stack =
                s.plan_stack(&p, ContainerAction::Restart, "otherstack", &members, now())?;
            let approval = s.approve_stack(&p, &stack.plan.id, &stack.digest, now())?;
            let stack_job =
                s.queue_stack(&p, "other-stack", &stack, &approval.token, &members, now())?;
            let members = StackSnapshot::new("otherstack", vec![snapshot('5'), snapshot('6')])?;
            let open_stack =
                s.plan_stack(&p, ContainerAction::Restart, "otherstack", &members, now())?;
            let selection = limeos_domain::ComposeSelection {
                stack: "media".into(),
                template: "standard".into(),
            };
            let compose = s.plan_compose(&p, &catalog, &selection, now())?;
            Ok((
                p.id,
                [plan.plan.id, plan.digest, approved.token, approval.token],
                job.id,
                open.plan.id,
                stack_job.id,
                open_stack.plan.id,
                compose.plan.id,
            ))
        })
        .await
        .unwrap();
    let (other_id, extra, container_job, container_plan, stack_job, stack_plan, compose_plan) =
        seeded;
    let mut identifiers = vec![
        other_id,
        "otheroperator".to_owned(),
        admin_id,
        session.token,
        session.csrf,
        digest,
        csrf,
        container_job.clone(),
        container_plan.clone(),
        stack_job.clone(),
        stack_plan.clone(),
        compose_plan.clone(),
    ];
    identifiers.extend(extra);
    Others {
        container_plan,
        container_job,
        stack_plan,
        stack_job,
        compose_plan,
        identifiers,
    }
}

/// Every grant an administrator can hold, so that the administrator's own
/// answer on a route is never the scope refusal a CSRF refusal looks like.
fn every_grant() -> Vec<Scope> {
    use limeos_domain::Operation::*;
    [
        (HealthRead, "*"),
        (MediaRequest, "*"),
        (ContainerManage, "*"),
        (DeploymentManage, "*"),
        (StorageManage, "*"),
        (IdentityManage, "*"),
        (AutomationManage, "automation:observe-health"),
    ]
    .map(|(operation, resource)| Scope {
        operation,
        resource: resource.into(),
    })
    .to_vec()
}
/// A digest of every table's rows, by table: equal before and after means
/// the request changed nothing in the store, audit events included.
fn state(path: &Path) -> std::collections::BTreeMap<String, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let mut state = std::collections::BTreeMap::new();
    for table in tables {
        let mut statement = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let columns = statement.column_count();
        let mut rows: Vec<String> = statement
            .query_map([], |r| {
                (0..columns)
                    .map(|i| r.get_ref(i).map(|v| format!("{v:?}")))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map(|v| v.join("|"))
            })
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        rows.sort();
        state.insert(table, limeos_identity::digest(&rows.join("\n")));
    }
    state
}
/// The tables whose rows differ between two states.
fn changed(
    before: &std::collections::BTreeMap<String, String>,
    after: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    after
        .iter()
        .filter(|(table, digest)| before.get(*table) != Some(*digest))
        .map(|(table, _)| table.clone())
        .collect()
}

fn storage_contract() -> serde_json::Value {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    fixture["cases"][0]["contract"].clone()
}
/// A whole reviewed storage-target proposal, which the API parses before it
/// calls core.
fn storage_target_queue() -> String {
    let contract: limeos_domain::StorageContract =
        serde_json::from_value(storage_contract()).unwrap();
    let device = &contract.devices[0];
    let identity = limeos_domain::DirectoryIdentity {
        major: 8,
        minor: 0,
        mount_id: 1,
        inode: 5,
        mode: 0o40755,
        uid: 0,
        gid: 0,
    };
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
            parent: identity,
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
fn storage_mount_queue() -> String {
    let identity = serde_json::json!({"major":8,"minor":1,"mount_id":10,"inode":2,"mode":16877,"uid":0,"gid":0});
    let devices = [
        serde_json::json!({"device":{"major":8,"minor":16},"filesystem_uuid":"uuid-data",
        "filesystem":"ext4","serial":null,"boot_backing":false,"mounts":[]}),
    ];
    serde_json::json!({
        "proposal":{"plan":{"id":"f".repeat(64),"version":1,"principal":"admin","grant_revision":1,
            "effect":{"version":1,"operation":"storage.mount","action":"e".repeat(64),
                "device":{"id":"data","role":"data","filesystem_uuid":"uuid-data","filesystem":"ext4",
                    "mountpoint":"/mnt/data","serial":null},
                "expected":{"host_root_mount_id":10,"topology_digest":"a".repeat(64),
                    "mounts_digest":"b".repeat(64),"fstab_digest":"c".repeat(64),"fstab_entries":[],
                    "devices":devices},
                "target":{"mountpoint":"/mnt/data","parent":identity,"existing":identity},
                "physical_devices":[{"major":8,"minor":16}],"consumer_digest":"d".repeat(64),
                "created_at":100,"expires_at":400}},"digest":"b".repeat(64)},
        "approval":"c".repeat(64),"idempotency_key":"mount-fixture"})
    .to_string()
}
fn container_queue() -> String {
    serde_json::json!({
        "proposal": {"plan": {
            "id": "f".repeat(64), "version": 1, "principal": "admin", "grant_revision": 1,
            "expected": {"resource": format!("container:{}", "b".repeat(64)),
                "image": format!("sha256:{}", "c".repeat(64)),
                "started_at": "2026-10-05T07:00:00Z", "running": true},
            "created_at": 100, "expires_at": 400}, "digest": "e".repeat(64)},
        "approval": "c".repeat(64),
        "idempotency_key": "key",
    })
    .to_string()
}
fn stack_queue() -> String {
    use limeos_domain::{ContainerAction, ContainerSnapshot, StackPlan, StackSnapshot};
    let member = |c: char| ContainerSnapshot {
        resource: format!("container:{}", c.to_string().repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running: true,
    };
    let proposal = limeos_domain::PlannedStackAction {
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
    };
    serde_json::json!({"proposal": proposal, "approval": "f".repeat(64), "idempotency_key": "key"})
        .to_string()
}
/// A body the API accepts for each mutation, so the request reaches core.
/// `None` for the mutations that take no body.
fn body(path: &str) -> Option<String> {
    if path.ends_with("/cancel") || path == "/api/v1/auth/logout" {
        return None;
    }
    let approval = format!(r#"{{"digest":"{}"}}"#, "e".repeat(64));
    Some(match path {
        "/api/v1/storage/mounts/plans" => r#"{"target_id":"data"}"#.into(),
        p if p.ends_with("/approval") => approval,
        "/api/v1/storage/plans" => serde_json::json!({"contract": storage_contract(),
            "inventory_digest": "a".repeat(64), "timeout_seconds": 10})
        .to_string(),
        "/api/v1/storage/targets/plans" => storage_contract().to_string(),
        "/api/v1/storage/targets/jobs" => storage_target_queue(),
        "/api/v1/storage/mounts/jobs" => storage_mount_queue(),
        "/api/v1/auth/capabilities/observe-health/enable" => {
            serde_json::json!({ "password": PASSWORD }).to_string()
        }
        "/api/v1/compose/plans" => r#"{"stack":"media","template":"standard"}"#.into(),
        "/api/v1/container/restart/plans" => {
            format!(r#"{{"resource":"container:{}"}}"#, "b".repeat(64))
        }
        "/api/v1/container/plans" => format!(
            r#"{{"operation":"restart","resource":"container:{}"}}"#,
            "b".repeat(64)
        ),
        "/api/v1/container/jobs" | "/api/v1/container/restart/jobs" => container_queue(),
        "/api/v1/stack/plans" => r#"{"project":"media","operation":"restart"}"#.into(),
        "/api/v1/stack/jobs" => stack_queue(),
        "/api/v1/jobs/{id}/resolution" => serde_json::json!({
            "declared": "verified_done",
            "reason": "checked by hand",
            "evidence_digest": "f".repeat(64),
            "password": PASSWORD,
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
        _ => panic!("add a body fixture for POST {path} to the route sweep"),
    })
}
/// The URI a route is requested at: every `{param}` filled, and the query a
/// read needs to reach core.
fn uri(path: &str) -> String {
    let filled: Vec<String> = path
        .split('/')
        .map(|part| {
            if part.starts_with('{') {
                ID.to_owned()
            } else {
                part.to_owned()
            }
        })
        .collect();
    let query = match path {
        "/api/v1/system/history" => "?range=24h",
        _ => "",
    };
    format!("{}{query}", filled.join("/"))
}

/// How a request presents itself.
#[derive(Clone, Copy, Default)]
struct Ask<'a> {
    cookie: Option<&'a str>,
    origin: Option<&'a str>,
    csrf: Option<&'a str>,
    authorization: Option<&'a str>,
}
impl<'a> Ask<'a> {
    /// A mutation's full presentation for `session`; a read ignores the
    /// Origin and CSRF headers.
    fn signed(session: &'a Session) -> Self {
        Self {
            cookie: Some(&session.token),
            origin: Some(ORIGIN),
            csrf: Some(&session.csrf),
            authorization: None,
        }
    }
    fn anonymous() -> Self {
        Self {
            origin: Some(ORIGIN),
            csrf: Some(WRONG_CSRF),
            ..Self::default()
        }
    }
}
/// The answer: status, and for a refusal the envelope without its audit id.
#[derive(Debug, PartialEq, Eq)]
struct Answer {
    status: u16,
    envelope: Option<(String, String, bool)>,
}
async fn send(core: &Core, route: &Route, uri: &str, ask: Ask<'_>, body: Option<&str>) -> Answer {
    send_text(core, route, uri, ask, body).await.0
}
/// As `send`, with the whole response body.
async fn send_text(
    core: &Core,
    route: &Route,
    uri: &str,
    ask: Ask<'_>,
    body: Option<&str>,
) -> (Answer, String) {
    let cookie = ask.cookie.map(|t| format!("__Host-limeos={t}"));
    let mut headers = vec![];
    if let Some(cookie) = &cookie {
        headers.push(("cookie", cookie.as_str()));
    }
    for (name, value) in [
        ("origin", ask.origin),
        ("x-csrf-token", ask.csrf),
        ("authorization", ask.authorization),
    ] {
        if let Some(value) = value {
            headers.push((name, value));
        }
    }
    if body.is_some() {
        headers.push(("content-type", "application/json"));
    }
    let (status, text) =
        limeos_api::respond(core.clone(), ORIGIN, route.method, uri, &headers, body).await;
    let envelope = (status >= 400).then(|| {
        let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|_| {
            panic!(
                "{} {uri}: {status} is not an envelope: {text}",
                route.method
            )
        });
        (
            value["code"].as_str().unwrap_or_default().to_owned(),
            value["message"].as_str().unwrap_or_default().to_owned(),
            value["retry"].as_bool().unwrap_or_default(),
        )
    });
    (Answer { status, envelope }, text)
}

/// Signed-in routes a principal may use without any grant, and what the
/// viewer (no grant, no objects of its own) is answered: its own session,
/// its own logout, and lists of its own jobs (none). Every answer to the
/// viewer, here or not, is also checked for another principal's identifiers
/// and for any `principal` but the viewer's. Not here, so 403: `GET
/// /api/v1/jobs/needs-attention`, which lists every principal's stuck jobs
/// and is administrator-only; and the observation reads (overview,
/// resources, stream), filtered to the principal's health grants and
/// refused when that leaves nothing.
const NO_SCOPE_NEEDED: &[(&str, u16)] = &[
    ("GET /api/v1/auth/session", 200),
    ("POST /api/v1/auth/logout", 204),
    ("GET /api/v1/container/jobs", 200),
    ("GET /api/v1/container/restart/jobs", 200),
    ("GET /api/v1/stack/jobs", 200),
];
/// Routes on a plan or job by id that core looks up among the principal's
/// own: another principal's, or a missing one, is "not found", so a
/// principal without scope (who can create none) is refused with 404, and
/// whether the id exists is not disclosed. The viewer asks for another
/// principal's real plan or job here (`viewer_request`).
const OWN_OBJECTS_ONLY: &[&str] = &[
    "GET /api/v1/compose/plans/{id}",
    "POST /api/v1/compose/plans/{id}/approval",
    "POST /api/v1/compose/plans/{id}/cancel",
    "POST /api/v1/container/plans/{id}/approval",
    "POST /api/v1/container/plans/{id}/cancel",
    "POST /api/v1/container/jobs",
    "GET /api/v1/container/jobs/{id}",
    "POST /api/v1/container/jobs/{id}/cancel",
    "POST /api/v1/container/restart/plans/{id}/approval",
    "POST /api/v1/container/restart/plans/{id}/cancel",
    "POST /api/v1/container/restart/jobs",
    "GET /api/v1/container/restart/jobs/{id}",
    "POST /api/v1/container/restart/jobs/{id}/cancel",
    "POST /api/v1/stack/plans/{id}/approval",
    "POST /api/v1/stack/plans/{id}/cancel",
    "POST /api/v1/stack/jobs",
    "GET /api/v1/stack/jobs/{id}",
    "POST /api/v1/stack/jobs/{id}/cancel",
];

/// The URI and body the viewer sends: on a route by id, another principal's
/// real plan or job of that kind; a queue request names another principal's
/// plan. Elsewhere the sweep's own.
fn viewer_request(other: &Others, path: &str, body: Option<&str>) -> (String, Option<String>) {
    let id = if path.starts_with("/api/v1/compose/plans/") {
        Some(&other.compose_plan)
    } else if path.starts_with("/api/v1/container/plans/")
        || path.starts_with("/api/v1/container/restart/plans/")
    {
        Some(&other.container_plan)
    } else if path.starts_with("/api/v1/container/jobs/")
        || path.starts_with("/api/v1/container/restart/jobs/")
    {
        Some(&other.container_job)
    } else if path.starts_with("/api/v1/stack/plans/") {
        Some(&other.stack_plan)
    } else if path.starts_with("/api/v1/stack/jobs/") {
        Some(&other.stack_job)
    } else {
        None
    };
    let uri = match id {
        Some(id) => uri(path).replace(ID, id),
        None => uri(path),
    };
    let body = body.map(|body| match path {
        "/api/v1/container/jobs" | "/api/v1/container/restart/jobs" => {
            body.replace(&"f".repeat(64), &other.container_plan)
        }
        "/api/v1/stack/jobs" => body.replace(&"c".repeat(64), &other.stack_plan),
        _ => body.to_owned(),
    });
    (uri, body)
}
/// Every `principal` value in a JSON answer that is not `own`.
fn foreign_principals(text: &str, own: &str) -> Vec<String> {
    fn walk(value: &serde_json::Value, own: &str, found: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(fields) => {
                for (key, field) in fields {
                    match field.as_str() {
                        Some(principal) if key == "principal" && principal != own => {
                            found.push(principal.to_owned())
                        }
                        _ => walk(field, own, found),
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, own, found)),
            _ => {}
        }
    }
    let mut found = vec![];
    if let Ok(value) = serde_json::from_str(text) {
        walk(&value, own, &mut found);
    }
    found
}

#[tokio::test]
async fn every_route_is_authenticated_by_core_itself() {
    let sweep = fixture(true).await;
    let routes = limeos_api::routes::<Core>();
    let mut swept = 0;
    let mut misses = vec![];
    for route in &routes {
        let name = format!("{} {}", route.method, route.path);
        assert!(matches!(route.method, "GET" | "POST"), "{name}");
        if route.access == Access::Public {
            assert!(
                ["GET /api/v1/health", "POST /api/v1/auth/login"].contains(&name.as_str()),
                "{name} is public"
            );
            continue;
        }
        let mutate = route.access == Access::Mutate;
        assert_eq!(mutate, route.method == "POST", "{name}");
        let uri = uri(route.path);
        let body = if mutate { body(route.path) } else { None };
        let body = body.as_deref();
        let mut refused = |answer: &Answer, status: u16, case: &str| {
            if answer.status != status {
                misses.push(format!("{name}: {case}: {answer:?}"));
            }
        };
        let bearer = format!("Bearer {}", sweep.admin.token);
        let mut cases = vec![
            (Ask::anonymous(), 401, "no cookie"),
            // An Authorization header alone authenticates nothing, even one
            // carrying a live session's token.
            (
                Ask {
                    authorization: Some(&bearer),
                    ..Ask::anonymous()
                },
                401,
                "Authorization header, no cookie",
            ),
            (
                Ask {
                    cookie: Some(UNKNOWN),
                    ..Ask::anonymous()
                },
                401,
                "unknown session",
            ),
        ];
        if mutate {
            let admin = Ask::signed(&sweep.admin);
            cases.extend([
                (
                    Ask {
                        csrf: Some(WRONG_CSRF),
                        ..admin
                    },
                    403,
                    "wrong CSRF",
                ),
                (
                    Ask {
                        csrf: None,
                        ..admin
                    },
                    403,
                    "no CSRF",
                ),
                (
                    Ask {
                        origin: Some("https://foreign.invalid"),
                        ..admin
                    },
                    403,
                    "foreign Origin",
                ),
                (
                    Ask {
                        origin: None,
                        ..admin
                    },
                    403,
                    "no Origin",
                ),
                // Origin before anything else: before the cookie is read.
                (
                    Ask {
                        origin: Some("https://foreign.invalid"),
                        ..Ask::anonymous()
                    },
                    403,
                    "foreign Origin, no cookie",
                ),
            ]);
        }
        let mut cases: Vec<_> = cases.into_iter().map(|(a, s, c)| (a, s, c, body)).collect();
        if mutate && body.is_some() {
            // The CSRF value is verified before an input refusal is released,
            // and that refusal reaches only an authenticated caller.
            let admin = Ask::signed(&sweep.admin);
            cases.extend([
                (
                    Ask {
                        csrf: Some(WRONG_CSRF),
                        ..admin
                    },
                    403,
                    "wrong CSRF, body not JSON",
                    Some("not json"),
                ),
                (admin, 400, "body not JSON", Some("not json")),
            ]);
        }
        for (ask, status, case, body) in cases {
            let before = state(&sweep.db);
            let answer = send(&sweep.core, route, &uri, ask, body).await;
            refused(&answer, status, case);
            let changed = changed(&before, &state(&sweep.db));
            assert!(changed.is_empty(), "{name}: {case} changed {changed:?}");
        }
        // The viewer without scope: the expected answer, nothing of another
        // principal's in it, and no store change but its own logout's.
        let expected = NO_SCOPE_NEEDED
            .iter()
            .find(|(route, _)| *route == name)
            .map(|(_, status)| *status)
            .unwrap_or(if OWN_OBJECTS_ONLY.contains(&name.as_str()) {
                404
            } else {
                403
            });
        let logout = name == "POST /api/v1/auth/logout";
        let viewer = if logout {
            &sweep.viewer_spare
        } else {
            &sweep.viewer
        };
        let (viewer_uri, viewer_body) = viewer_request(&sweep.other, route.path, body);
        let before = state(&sweep.db);
        let (answer, text) = send_text(
            &sweep.core,
            route,
            &viewer_uri,
            Ask::signed(viewer),
            viewer_body.as_deref(),
        )
        .await;
        if answer.status != expected {
            misses.push(format!("{name}: signed in without scope: {answer:?}"));
        }
        let leaked = sweep.other.leaked(&text);
        if !leaked.is_empty() {
            misses.push(format!("{name}: the viewer was shown {leaked:?}: {text}"));
        }
        let foreign = foreign_principals(&text, &sweep.viewer_id);
        if !foreign.is_empty() {
            misses.push(format!(
                "{name}: the viewer was shown principals {foreign:?}"
            ));
        }
        if name == "GET /api/v1/auth/session" && !text.contains(&sweep.viewer_id) {
            misses.push(format!("{name}: not the viewer's own session: {text}"));
        }
        let changed = changed(&before, &state(&sweep.db));
        let allowed: &[&str] = if logout {
            &["events", "meta", "sessions", "sqlite_sequence"]
        } else {
            &[]
        };
        assert!(
            changed
                .iter()
                .all(|table| allowed.contains(&table.as_str())),
            "{name}: the viewer changed {changed:?}"
        );
        // Authenticated and scoped: whatever core then answers, it is neither
        // "who are you" nor the 403 the CSRF and Origin cases expect, so
        // those cases above are refusals and not core's own answer.
        // The administrator's request carries the Authorization header a
        // proxy's basic authentication adds: it changes nothing.
        if name != "POST /api/v1/auth/logout" {
            let ask = Ask {
                authorization: Some(PROXY_BASIC),
                ..Ask::signed(&sweep.admin)
            };
            let answer = send(&sweep.core, route, &uri, ask, body).await;
            if matches!(answer.status, 401 | 403) {
                misses.push(format!("{name}: administrator: {answer:?}"));
            }
        }
        swept += 1;
    }
    assert!(misses.is_empty(), "{}", misses.join("\n"));
    // 67 gated routes today: catches an empty or truncated walk.
    assert!(swept >= 60, "only {swept} routes swept");
}

#[tokio::test]
async fn a_signed_in_scoped_request_passes_the_gate_in_each_access_class() {
    let sweep = fixture(true).await;
    let route = |method: &'static str, path: &'static str| {
        limeos_api::routes::<Core>()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("{method} {path} is not mounted"))
    };
    let health = route("GET", "/api/v1/health");
    assert_eq!(health.access, Access::Public);
    let answer = send(&sweep.core, &health, "/api/v1/health", Ask::default(), None).await;
    assert_eq!(answer.status, 200, "{answer:?}");
    let session = route("GET", "/api/v1/auth/session");
    assert_eq!(session.access, Access::Read);
    let answer = send(
        &sweep.core,
        &session,
        "/api/v1/auth/session",
        Ask {
            authorization: Some(PROXY_BASIC),
            ..Ask::signed(&sweep.admin)
        },
        None,
    )
    .await;
    assert_eq!(answer.status, 200, "{answer:?}");
    let logout = route("POST", "/api/v1/auth/logout");
    assert_eq!(logout.access, Access::Mutate);
    let answer = send(
        &sweep.core,
        &logout,
        "/api/v1/auth/logout",
        Ask {
            authorization: Some(PROXY_BASIC),
            ..Ask::signed(&sweep.spare)
        },
        None,
    )
    .await;
    assert_eq!(answer.status, 204, "{answer:?}");
    // That session is gone; the administrator's other one is not.
    let answer = send(
        &sweep.core,
        &session,
        "/api/v1/auth/session",
        Ask::signed(&sweep.spare),
        None,
    )
    .await;
    assert_eq!(answer.status, 401, "{answer:?}");
    let answer = send(
        &sweep.core,
        &session,
        "/api/v1/auth/session",
        Ask::signed(&sweep.admin),
        None,
    )
    .await;
    assert_eq!(answer.status, 200, "{answer:?}");
}

/// What an unauthenticated caller may learn from any route: that it is not
/// signed in, and nothing that depends on its input or the server's state.
#[tokio::test]
async fn no_route_answers_an_unauthenticated_request_from_its_input_or_server_state() {
    let expected = Answer {
        status: 401,
        envelope: Some((
            "unauthenticated".into(),
            "Authentication is required.".into(),
            false,
        )),
    };
    // With and without a compose catalog: core must not consult it first.
    for catalog in [true, false] {
        let sweep = fixture(catalog).await;
        for route in limeos_api::routes::<Core>() {
            if route.access == Access::Public {
                continue;
            }
            let name = format!("{} {}", route.method, route.path);
            let mutate = route.access == Access::Mutate;
            let good = uri(route.path);
            let mut inputs: Vec<(String, Option<String>, &str)> = vec![(
                good.clone(),
                if mutate { body(route.path) } else { None },
                "valid input",
            )];
            if mutate {
                inputs.push((
                    good.clone(),
                    Some(r#"{"unexpected":true}"#.into()),
                    "wrong body",
                ));
                inputs.push((good.clone(), Some("not json".into()), "not JSON"));
            } else {
                inputs.push((format!("{good}?unexpected=%FF"), None, "bad query"));
            }
            if route.path.contains('{') {
                inputs.push((good.replace(ID, "%FF%FE"), None, "bad id"));
            }
            for (uri, body, input) in &inputs {
                for (cookie, who) in [(None, "no cookie"), (Some(UNKNOWN), "unknown session")] {
                    let before = state(&sweep.db);
                    let ask = Ask {
                        cookie,
                        ..Ask::anonymous()
                    };
                    let answer = send(&sweep.core, &route, uri, ask, body.as_deref()).await;
                    assert_eq!(
                        answer, expected,
                        "{name}: {who}, {input}, catalog {catalog}"
                    );
                    let changed = changed(&before, &state(&sweep.db));
                    assert!(changed.is_empty(), "{name}: {who}, {input}: {changed:?}");
                }
            }
        }
    }
}

/// The gate confirms a session before releasing an input refusal on the
/// store's query-only connection: with the writer stalled and its queue
/// full, an unauthenticated bad request still gets 401 promptly and adds
/// nothing to the queue, and a signed-in one still gets its own answer.
#[tokio::test]
async fn a_refusal_is_confirmed_beside_a_stalled_writer_and_a_full_queue() {
    let sweep = fixture(true).await;
    let db = &sweep.core.db;
    let release = crate::operation_tests::stall_writer(db).await;
    let waiting: Vec<_> = (0..limeos_persistence::QUEUE_CAPACITY)
        .map(|_| {
            let db = db.clone().with_call_deadline(Duration::from_secs(60));
            tokio::spawn(async move { db.call(|_| Ok(())).await })
        })
        .collect();
    tokio::time::timeout(Duration::from_secs(10), async {
        while db.writer_activity().queued < limeos_persistence::QUEUE_CAPACITY {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the queue fills");
    let route = |method: &'static str, path: &'static str| {
        limeos_api::routes::<Core>()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("{method} {path} is not mounted"))
    };
    let plan = route("POST", "/api/v1/compose/plans");
    let history = route("GET", "/api/v1/system/history");
    let admin = Ask::signed(&sweep.admin);
    let unknown = Ask {
        cookie: Some(UNKNOWN),
        ..Ask::anonymous()
    };
    let wrong_csrf = Ask {
        csrf: Some(WRONG_CSRF),
        ..admin
    };
    let cases = [
        (
            &plan,
            "/api/v1/compose/plans",
            unknown,
            401,
            "unknown session",
        ),
        (
            &plan,
            "/api/v1/compose/plans",
            wrong_csrf,
            403,
            "wrong CSRF",
        ),
        (&plan, "/api/v1/compose/plans", admin, 400, "signed in"),
        (
            &history,
            "/api/v1/system/history?range=no",
            unknown,
            401,
            "unknown session",
        ),
        (
            &history,
            "/api/v1/system/history?range=no",
            admin,
            400,
            "signed in",
        ),
    ];
    for (route, uri, ask, status, case) in cases {
        let asked = std::time::Instant::now();
        let body = (route.method == "POST").then_some("not json");
        let answer = send(&sweep.core, route, uri, ask, body).await;
        assert_eq!(answer.status, status, "{uri}: {case}: {answer:?}");
        assert!(
            asked.elapsed() < Duration::from_secs(1),
            "{uri}: {case}: {:?}",
            asked.elapsed()
        );
        assert_eq!(
            db.writer_activity().queued,
            limeos_persistence::QUEUE_CAPACITY,
            "{uri}: {case}"
        );
    }
    release.send(()).unwrap();
    for call in waiting {
        call.await.unwrap().unwrap();
    }
}
