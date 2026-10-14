//! Genuine API composition with synthetic private policy, never installed restore.
use limeos_backup_archive::{
    custody::{self, CustodyLimits, CustodyRoot, RetainedArchive},
    preparation::{
        self, InstalledMetadata, ManagedTarget, PreparationLimits, PreparationPlan,
        PreparationRoot, PreparedRestore, ResourceSelection, Selection,
    },
    recovery::{self, CurrentInputs},
    staging::{StagingRoot, VerifiedCatalog},
};
use limeos_domain::{
    BackupPreparationBinding, BackupPreparationJob, ErrorCode, JobState, PlannedBackupPreparation,
    Principal,
    backups::{AdmissionPolicy, EntryKind},
};
use limeos_persistence::Store;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const ROOT_ENV: &str = "LIMEOS_TEST_BACKUP_BINDING_ROOT";
const MODE_ENV: &str = "LIMEOS_TEST_BACKUP_BINDING_MODE";
const KEY: &str = "private-preparation-fixture";
const WAIT: Duration = Duration::from_secs(30);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new(zstd: bool) -> (tempfile::TempDir, Self) {
        let temp = tempfile::Builder::new()
            .prefix("limeos-backup-binding-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        let scene = Self {
            root: temp.path().to_owned(),
        };
        for name in [
            "custody",
            "staging",
            "preparation",
            "limeos-config",
            "limeos-state",
            "stacks",
            "app-config",
        ] {
            let path = scene.path(name);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(
            scene.path("limeos-config/core.json"),
            b"live configuration unchanged",
        )
        .unwrap();
        fs::write(
            scene.path("app-config/sentinel"),
            b"live application unchanged",
        )
        .unwrap();
        fs::write(scene.path("unrelated"), b"outside sentinel").unwrap();
        let bytes: &[u8] = if zstd {
            include_bytes!("../../../tests/fixtures/backup-archives/legacy-valid.tar.zst")
        } else {
            include_bytes!("../../../tests/fixtures/backup-archives/legacy-valid.tar.gz")
        };
        // Content detection, rather than a filename suffix, chooses the format.
        fs::write(scene.path("selected.bin"), bytes).unwrap();
        (temp, scene)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    fn policy(&self) -> AdmissionPolicy {
        let mut policy: AdmissionPolicy = serde_json::from_str(include_str!(
            "../../../tests/fixtures/backup-custody/policy.json"
        ))
        .unwrap();
        for resource in &mut policy.resources {
            resource.destination_root = self.path(&resource.id).to_str().unwrap().into();
        }
        policy
    }
    fn limits(&self) -> PreparationLimits {
        PreparationLimits {
            max_resources: 4,
            max_objects: 32,
            max_file_bytes: 1 << 20,
            max_total_bytes: 2 << 20,
            max_record_bytes: 1 << 20,
        }
    }
    fn selection(&self) -> Selection {
        let target = |path: &str, kind| ManagedTarget {
            relative_path: path.into(),
            kind,
            installed: InstalledMetadata {
                uid: 42000,
                gid: 42001,
                mode: if kind == EntryKind::File {
                    0o640
                } else {
                    0o750
                },
            },
        };
        Selection {
            resources: vec![
                ResourceSelection {
                    id: "limeos-config".into(),
                    destination_root: self.path("limeos-config").to_str().unwrap().into(),
                    targets: vec![
                        target("", EntryKind::Directory),
                        target("core.json", EntryKind::File),
                        target("credentials.env", EntryKind::File),
                        target("media_layout.json", EntryKind::File),
                        target("storage_plugins", EntryKind::Directory),
                        target("storage_plugins/mergerfs.json", EntryKind::File),
                    ],
                },
                ResourceSelection {
                    id: "app-config".into(),
                    destination_root: self.path("app-config").to_str().unwrap().into(),
                    targets: vec![
                        target("", EntryKind::Directory),
                        target("sonarr", EntryKind::Directory),
                        target("sonarr/config.xml", EntryKind::File),
                    ],
                },
            ],
        }
    }
    fn retain(&self) -> RetainedArchive {
        custody::retain(
            File::open(self.path("selected.bin")).unwrap(),
            self.policy(),
            &CustodyRoot::open(&self.path("custody")).unwrap(),
            &custody_limits(),
            &|| false,
        )
        .unwrap()
    }
    fn stage_plan(&self, retained: &mut RetainedArchive) -> (VerifiedCatalog, PreparationPlan) {
        let catalog = retained
            .stage(
                &self.policy(),
                &StagingRoot::open(&self.path("staging")).unwrap(),
                &|| false,
            )
            .unwrap();
        let plan = preparation::plan(
            &catalog,
            retained.manifest().clone(),
            self.selection(),
            self.limits(),
        )
        .unwrap();
        (catalog, plan)
    }
    fn binding(
        &self,
        retained: &RetainedArchive,
        plan: &PreparationPlan,
    ) -> BackupPreparationBinding {
        BackupPreparationBinding {
            custody: retained.binding().clone(),
            preparation_identity: plan.identity().into(),
            selected_resources: plan
                .selection()
                .resources
                .iter()
                .map(|r| r.id.clone())
                .collect(),
            // A trusted synthetic identity, not a production workspace/ancestor proof.
            workspace_policy_identity: self
                .current_workspace_identity(plan.selection(), plan.limits()),
        }
    }
    fn current_workspace_identity(
        &self,
        selection: &Selection,
        limits: &PreparationLimits,
    ) -> String {
        limeos_identity::digest(
            &json!({"root": self.root,
            "policy": self.policy(), "selection": selection, "limits": limits})
            .to_string(),
        )
    }
    fn store(&self) -> (Store, Principal) {
        let path = self.path("core.sqlite");
        let existed = path.exists();
        let mut store = Store::open(&path).unwrap();
        if !existed {
            store.issue_bootstrap("fixture-bootstrap", 100).unwrap();
            store
                .enroll("fixture-bootstrap", "alice", "$argon2id$fixture", 101)
                .unwrap();
        }
        let principal = store.login_record("alice").unwrap().unwrap().principal;
        (store, principal)
    }
    fn unchanged(&self) {
        assert_eq!(
            fs::read(self.path("limeos-config/core.json")).unwrap(),
            b"live configuration unchanged"
        );
        assert_eq!(
            fs::read(self.path("app-config/sentinel")).unwrap(),
            b"live application unchanged"
        );
        assert_eq!(
            fs::read(self.path("unrelated")).unwrap(),
            b"outside sentinel"
        );
    }
}
fn custody_limits() -> CustodyLimits {
    CustodyLimits {
        max_archive_bytes: 1 << 20,
        max_record_bytes: 1 << 20,
    }
}
fn names(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}
fn save(path: &Path, value: &Value) {
    let pending = path.with_extension("pending");
    let mut file = File::create(&pending).unwrap();
    serde_json::to_writer(&mut file, value).unwrap();
    file.sync_all().unwrap();
    fs::rename(pending, path).unwrap();
    File::open(path.parent().unwrap())
        .unwrap()
        .sync_all()
        .unwrap();
}
fn state(scene: &Fixture) -> (PlannedBackupPreparation, String) {
    let value: Value =
        serde_json::from_slice(&fs::read(scene.path("state.json")).unwrap()).unwrap();
    (
        serde_json::from_value(value["proposal"].clone()).unwrap(),
        value["approval"].as_str().unwrap().into(),
    )
}
fn approved(
    store: &mut Store,
    p: &Principal,
    binding: &BackupPreparationBinding,
) -> (PlannedBackupPreparation, String) {
    let proposal = store.plan_backup_preparation(p, binding, 200).unwrap();
    let approval = store
        .approve_backup_preparation(p, &proposal.plan.id, &proposal.digest, binding, 201)
        .unwrap();
    (proposal, approval.token)
}
fn expected_job(proposal: &PlannedBackupPreparation) -> BackupPreparationJob {
    BackupPreparationJob {
        id: proposal.plan.action.clone(),
        state: JobState::Queued,
        plan: proposal.plan.clone(),
    }
}
fn snapshot(conn: &Connection) -> Vec<Vec<Vec<String>>> {
    [
        "meta",
        "events",
        "jobs",
        "job_resources",
        "resource_locks",
        "backup_preparation_plans",
    ]
    .iter()
    .map(|table| {
        let mut query = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = query.column_count();
        query
            .query_map([], |row| {
                Ok((0..columns)
                    .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                    .collect())
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
    .collect()
}
fn check_resources(conn: &Connection, job: &BackupPreparationJob) {
    let mut query = conn
        .prepare("SELECT resource FROM job_resources WHERE job=? ORDER BY resource")
        .unwrap();
    let actual = query
        .query_map([&job.id], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        actual,
        vec![
            format!("backup:custody:{}", job.plan.binding.custody.record_id),
            "backup:preparation".into(),
            "storage:configuration".into()
        ]
    );
    assert_eq!(
        conn.query_row("SELECT resource FROM jobs WHERE id=?", [&job.id], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "backup:preparation"
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM resource_locks WHERE job=?",
            [&job.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
fn checked(prepared: &PreparedRestore, scene: &Fixture) {
    let expected: BTreeMap<(&str, &str), &[u8]> = BTreeMap::from([
        (
            ("limeos-config", "core.json"),
            b"{\"version\":1,\"listen\":\"127.0.0.1:8003\"}\n".as_slice(),
        ),
        (
            ("limeos-config", "credentials.env"),
            b"PIHEALTH_USER=synthetic\n".as_slice(),
        ),
        (
            ("limeos-config", "media_layout.json"),
            b"{\"profile\":\"single_disk\"}\n".as_slice(),
        ),
        (
            ("limeos-config", "storage_plugins/mergerfs.json"),
            b"{\"pools\":[]}\n".as_slice(),
        ),
        (
            ("app-config", "sonarr/config.xml"),
            b"<Config><Port>8989</Port></Config>\n".as_slice(),
        ),
    ]);
    let selection = scene.selection();
    let mut files = 0;
    let mut objects = 0;
    for entry in prepared.entries() {
        objects += 1;
        let m = entry.metadata();
        let resource = selection
            .resources
            .iter()
            .find(|r| r.id == m.resource)
            .unwrap();
        assert_eq!(
            m.installed,
            resource
                .targets
                .iter()
                .find(|t| t.relative_path == m.relative_path)
                .unwrap()
                .installed
        );
        if let Some(mut reader) = entry.reader().unwrap() {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(
                bytes,
                expected[&(m.resource.as_str(), m.relative_path.as_str())]
            );
            assert_eq!(m.size, bytes.len() as u64);
            files += 1;
        } else {
            assert_eq!(m.kind, EntryKind::Directory);
        }
    }
    assert_eq!((objects, files), (9, 5));
    prepared.revalidate(&|| false).unwrap();
    scene.unchanged();
}
fn barrier(scene: &Fixture, stage: &str) {
    save(&scene.path("barrier.json"), &json!({"stage": stage}));
    // Parent retains this pipe, then SIGKILLs this exact Child at the barrier.
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
    panic!("barrier unexpectedly resumed");
}
struct OwnedChild {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}
impl OwnedChild {
    fn spawn(scene: &Fixture, mode: &str) -> Self {
        let stdout = scene.path(&format!("{mode}.stdout"));
        let stderr = scene.path(&format!("{mode}.stderr"));
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "subprocess_driver",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROOT_ENV, &scene.root)
            .env(MODE_ENV, mode)
            .env("TMPDIR", "/tmp")
            .stdin(Stdio::piped())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        Self {
            child,
            stdout,
            stderr,
        }
    }
    fn logs(&self) -> String {
        format!(
            "stdout={}\nstderr={}",
            fs::read_to_string(&self.stdout).unwrap(),
            fs::read_to_string(&self.stderr).unwrap()
        )
    }
    fn kill_at(&mut self, scene: &Fixture, stage: &str) {
        let start = Instant::now();
        loop {
            if let Ok(bytes) = fs::read(scene.path("barrier.json")) {
                if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                    if value["stage"] == stage {
                        break;
                    }
                }
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "child exited before barrier: {}",
                self.logs()
            );
            assert!(start.elapsed() < WAIT, "barrier timed out: {}", self.logs());
            thread::sleep(Duration::from_millis(10));
        }
        self.child.kill().unwrap();
        assert_eq!(self.child.wait().unwrap().signal(), Some(9));
    }
    fn finish(&mut self) {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "child failed: {}", self.logs());
                return;
            }
            assert!(start.elapsed() < WAIT, "child timed out: {}", self.logs());
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

#[test]
fn subprocess_driver() {
    let Some(root) = std::env::var_os(ROOT_ENV) else {
        return;
    };
    let scene = Fixture { root: root.into() };
    let mode = std::env::var(MODE_ENV).unwrap();
    let (mut store, p) = scene.store();
    if ["published", "queued", "prepared"].contains(&mode.as_str()) {
        let mut retained = scene.retain();
        save(
            &scene.path("retained-binding.json"),
            &json!(retained.binding()),
        );
        if mode == "published" {
            barrier(&scene, "published");
        }
        let (catalog, plan) = scene.stage_plan(&mut retained);
        let binding = scene.binding(&retained, &plan);
        let (proposal, approval) = approved(&mut store, &p, &binding);
        save(
            &scene.path("state.json"),
            &json!({"proposal": proposal, "approval": approval}),
        );
        let job = store
            .queue_backup_preparation(&p, KEY, &proposal, &approval, &binding, 202)
            .unwrap();
        assert_eq!(job, expected_job(&proposal));
        if mode == "queued" {
            barrier(&scene, "queued");
        }
        let prepared = preparation::prepare(
            &catalog,
            &plan,
            &scene.selection(),
            &PreparationRoot::open(&scene.path("preparation")).unwrap(),
            &scene.limits(),
            &|| false,
        )
        .unwrap();
        checked(&prepared, &scene);
        save(
            &scene.path("killed-prepared.json"),
            &json!({"record_sha256": prepared.record_sha256(),
            "attempts": names(&scene.path("preparation"))}),
        );
        barrier(&scene, "prepared");
    }
    let (proposal, approval) = state(&scene);
    let observer = Connection::open(scene.path("core.sqlite")).unwrap();
    let before = snapshot(&observer);
    let job = store
        .replay_backup_preparation(&p, KEY, &proposal, &approval)
        .unwrap()
        .unwrap();
    assert_eq!(job, expected_job(&proposal));
    assert_eq!(store.backup_preparation_job(&p, &job.id).unwrap(), job);
    // Expiry alone cannot invalidate a consumed lost-response replay.
    assert_eq!(
        store
            .queue_backup_preparation(
                &p,
                KEY,
                &proposal,
                &approval,
                &proposal.plan.binding,
                proposal.plan.expires_at + 1
            )
            .unwrap(),
        job
    );
    check_resources(&observer, &job);
    assert_eq!(snapshot(&observer), before);
    if mode == "verify-db-only" {
        return;
    }
    assert_eq!(mode, "verify");
    let stale = names(&scene.path("preparation"));
    let stale_staging = names(&scene.path("staging"));
    let selection = scene.selection();
    let limits = scene.limits();
    let current_workspace_identity = scene.current_workspace_identity(&selection, &limits);
    let prepared = recovery::reconstruct(
        &job.plan.binding,
        CurrentInputs {
            custody_root: &CustodyRoot::open(&scene.path("custody")).unwrap(),
            staging_root: &StagingRoot::open(&scene.path("staging")).unwrap(),
            preparation_root: &PreparationRoot::open(&scene.path("preparation")).unwrap(),
            admission_policy: &scene.policy(),
            custody_limits: &custody_limits(),
            selection,
            preparation_limits: limits,
            workspace_policy_identity: &current_workspace_identity,
        },
        &|| false,
    )
    .unwrap();
    checked(&prepared, &scene);
    let fresh = names(&scene.path("preparation"));
    assert_eq!(fresh.len(), stale.len() + 1);
    if let Ok(bytes) = fs::read(scene.path("killed-prepared.json")) {
        let old: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json!(prepared.record_sha256()), old["record_sha256"]);
        assert_eq!(json!(stale), old["attempts"]);
    }
    prepared.discard().unwrap();
    assert_eq!(names(&scene.path("preparation")), stale);
    assert_eq!(names(&scene.path("staging")), stale_staging);
    assert_eq!(snapshot(&observer), before);
    scene.unchanged();
}

#[test]
fn actual_commit_lost_response_and_process_restart_create_new_private_owners() {
    for zstd in [false, true] {
        let (_temp, scene) = Fixture::new(zstd);
        OwnedChild::spawn(&scene, "queued").kill_at(&scene, "queued");
        OwnedChild::spawn(&scene, "verify").finish();
        scene.unchanged();
    }
}

#[test]
fn killed_prepared_owner_is_not_a_durable_receipt_or_an_adopted_attempt() {
    let (_temp, scene) = Fixture::new(false);
    OwnedChild::spawn(&scene, "prepared").kill_at(&scene, "prepared");
    let stale = names(&scene.path("preparation"));
    assert_eq!(stale.len(), 1);
    OwnedChild::spawn(&scene, "verify").finish();
    assert_eq!(names(&scene.path("preparation")), stale);
    let (store, p) = scene.store();
    let (proposal, _) = state(&scene);
    assert_eq!(
        store
            .backup_preparation_job(&p, &proposal.plan.action)
            .unwrap()
            .state,
        JobState::Queued
    );
}

#[test]
fn unavailable_custody_cannot_interfere_with_exact_read_only_database_replay() {
    let (_temp, scene) = Fixture::new(true);
    OwnedChild::spawn(&scene, "queued").kill_at(&scene, "queued");
    fs::rename(scene.path("custody"), scene.path("custody-unavailable")).unwrap();
    // A replay which opens custody would fail: the trusted root no longer exists.
    OwnedChild::spawn(&scene, "verify-db-only").finish();
    let (proposal, _) = state(&scene);
    assert!(CustodyRoot::open(&scene.path("custody")).is_err());
    fs::rename(scene.path("custody-unavailable"), scene.path("custody")).unwrap();
    let archive = scene.path(&format!(
        "custody/retained-{}/archive",
        proposal.plan.binding.custody.record_id
    ));
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&archive, b"replaced retained archive").unwrap();
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o400)).unwrap();
    OwnedChild::spawn(&scene, "verify-db-only").finish();
    assert!(
        custody::recover(
            &CustodyRoot::open(&scene.path("custody")).unwrap(),
            &proposal.plan.binding.custody,
            scene.policy(),
            &custody_limits(),
            &|| false
        )
        .is_err()
    );
    scene.unchanged();
}

#[test]
fn current_full_policy_and_selection_limits_cannot_reuse_approved_preparation() {
    let (_temp, scene) = Fixture::new(false);
    let mut retained = scene.retain();
    let (catalog, original) = scene.stage_plan(&mut retained);
    let binding = scene.binding(&retained, &original);
    let (mut store, principal) = scene.store();
    let (proposal, token) = approved(&mut store, &principal, &binding);
    let observer = Connection::open(scene.path("core.sqlite")).unwrap();
    let before = snapshot(&observer);
    let root = CustodyRoot::open(&scene.path("custody")).unwrap();
    for field in ["revision", "admission-limit", "root", "mapping", "formats"] {
        let mut changed = scene.policy();
        match field {
            "revision" => changed.revision += 1,
            "admission-limit" => changed.limits.max_file_bytes += 1,
            "root" => changed.resources[0].destination_root.push_str("-changed"),
            "mapping" => changed.legacy_mappings[0].resource_prefix = "changed".into(),
            _ => changed.formats.reverse(),
        }
        assert!(
            custody::recover(
                &root,
                retained.binding(),
                changed,
                &custody_limits(),
                &|| false
            )
            .is_err(),
            "{field}"
        );
    }
    let tighter = CustodyLimits {
        max_archive_bytes: retained.binding().archive_bytes - 1,
        ..custody_limits()
    };
    assert!(
        custody::recover(&root, retained.binding(), scene.policy(), &tighter, &|| {
            false
        })
        .is_err()
    );
    let preparation_root = PreparationRoot::open(&scene.path("preparation")).unwrap();
    for field in [
        "uid",
        "gid",
        "mode",
        "resource-order",
        "root",
        "preparation-limit",
    ] {
        let mut selection = scene.selection();
        let mut limits = scene.limits();
        match field {
            "uid" => selection.resources[0].targets[1].installed.uid += 1,
            "gid" => selection.resources[0].targets[1].installed.gid += 1,
            "mode" => selection.resources[0].targets[1].installed.mode = 0o600,
            "resource-order" => selection.resources.reverse(),
            "root" => selection.resources[0].destination_root.push_str("-changed"),
            _ => limits.max_file_bytes += 1,
        }
        let fresh = preparation::plan(
            &catalog,
            retained.manifest().clone(),
            selection.clone(),
            limits,
        )
        .unwrap();
        assert_ne!(fresh.identity(), original.identity(), "{field}");
        let changed_binding = scene.binding(&retained, &fresh);
        assert_eq!(
            store
                .approve_backup_preparation(
                    &principal,
                    &proposal.plan.id,
                    &proposal.digest,
                    &changed_binding,
                    202
                )
                .unwrap_err()
                .0,
            ErrorCode::Conflict,
            "{field}"
        );
        assert_eq!(
            store
                .queue_backup_preparation(&principal, KEY, &proposal, &token, &changed_binding, 202)
                .unwrap_err()
                .0,
            ErrorCode::Conflict,
            "{field}"
        );
        assert_eq!(snapshot(&observer), before);
        let refused = preparation::prepare(
            &catalog,
            &original,
            &selection,
            &preparation_root,
            &limits,
            &|| false,
        )
        .unwrap_err();
        assert!(
            matches!(refused.failure, preparation::Failure::PolicyChanged),
            "{field}"
        );
        assert!(names(&scene.path("preparation")).is_empty());
    }
    catalog.discard().unwrap();
    retained.discard().unwrap();
    scene.unchanged();
}

#[test]
fn actual_audit_rollback_and_precommit_kill_leave_inert_custody_orphans() {
    let (_temp, scene) = Fixture::new(false);
    let mut retained = scene.retain();
    let (catalog, plan) = scene.stage_plan(&mut retained);
    let binding = scene.binding(&retained, &plan);
    let (mut store, p) = scene.store();
    let (proposal, token) = approved(&mut store, &p, &binding);
    let observer = Connection::open(scene.path("core.sqlite")).unwrap();
    let before = snapshot(&observer);
    observer.execute_batch("CREATE TRIGGER fixture_audit_failure BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'fixture audit fault'); END;").unwrap();
    assert_eq!(
        store
            .queue_backup_preparation(&p, KEY, &proposal, &token, &binding, 202)
            .unwrap_err()
            .0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(snapshot(&observer), before);
    observer
        .execute_batch("DROP TRIGGER fixture_audit_failure;")
        .unwrap();
    let published = names(&scene.path("custody"));
    catalog.discard().unwrap();
    drop(retained);
    drop(store);
    let (store, p) = scene.store();
    assert!(
        store
            .replay_backup_preparation(&p, KEY, &proposal, &token)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.job_state(&proposal.plan.action).unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(names(&scene.path("custody")), published);
    assert!(names(&scene.path("preparation")).is_empty());
    drop(store);
    let (_temp2, killed) = Fixture::new(true);
    OwnedChild::spawn(&killed, "published").kill_at(&killed, "published");
    let published = names(&killed.path("custody"));
    let (store, _) = killed.store();
    let observer = Connection::open(killed.path("core.sqlite")).unwrap();
    assert_eq!(
        observer
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(names(&killed.path("custody")), published);
    assert!(!killed.path("state.json").exists());
    drop(store);
    scene.unchanged();
    killed.unchanged();
}
