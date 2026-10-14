//! Genuine private-library composition with explicitly synthetic trust inputs.
use super::{CurrentInputs, Failure, RecoveryError, finish, reconstruct};
use crate::{
    custody::{self, CustodyError, CustodyLimit, CustodyLimits, CustodyRoot},
    preparation::{
        self, CleanupFailure, InstalledMetadata, ManagedTarget, PreparationLimits, PreparationRoot,
        PreparedRestore, ResourceSelection, Selection,
    },
    staging::{StagingError, StagingRoot, VerifiedCatalog},
};
use limeos_domain::{
    BackupPreparationBinding,
    backups::{AdmissionPolicy, EntryKind},
};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    os::unix::{
        fs::{MetadataExt, PermissionsExt, symlink},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const WORKSPACE: &str = "4444444444444444444444444444444444444444444444444444444444444444";
const ROOT_ENV: &str = "LIMEOS_TEST_BOUND_RECOVERY_ROOT";
const MODE_ENV: &str = "LIMEOS_TEST_BOUND_RECOVERY_MODE";
const WAIT: Duration = Duration::from_secs(30);
const FOREIGN: &[u8] = b"foreign child must remain untouched";

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(zstd: bool) -> (tempfile::TempDir, Self) {
        let temp = tempfile::Builder::new()
            .prefix("limeos-bound-recovery-")
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
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(scene.path("limeos-config/core.json"), b"live configuration").unwrap();
        fs::write(scene.path("app-config/sentinel"), b"live application").unwrap();
        fs::write(scene.path("unrelated"), b"outside sentinel").unwrap();
        let bytes: &[u8] = if zstd {
            include_bytes!("../../../../tests/fixtures/backup-archives/legacy-valid.tar.zst")
        } else {
            include_bytes!("../../../../tests/fixtures/backup-archives/legacy-valid.tar.gz")
        };
        // The genuine inspector detects content, not this synthetic filename.
        fs::write(scene.path("selected.bin"), bytes).unwrap();
        (temp, scene)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn policy(&self) -> AdmissionPolicy {
        let mut policy: AdmissionPolicy = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/backup-custody/policy.json"
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

    fn binding(&self) -> BackupPreparationBinding {
        let mut retained = custody::retain(
            File::open(self.path("selected.bin")).unwrap(),
            self.policy(),
            &CustodyRoot::open(&self.path("custody")).unwrap(),
            &custody_limits(),
            &|| false,
        )
        .unwrap();
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
        let binding = BackupPreparationBinding {
            custody: retained.binding().clone(),
            preparation_identity: plan.identity().into(),
            selected_resources: plan
                .selection()
                .resources
                .iter()
                .map(|r| r.id.clone())
                .collect(),
            // A synthetic current attestation, never inferred from saved metadata.
            workspace_policy_identity: WORKSPACE.into(),
        };
        catalog.discard().unwrap();
        binding
    }

    fn owners(&self, binding: &BackupPreparationBinding) -> (PreparedRestore, VerifiedCatalog) {
        let mut retained = custody::recover(
            &CustodyRoot::open(&self.path("custody")).unwrap(),
            &binding.custody,
            self.policy(),
            &custody_limits(),
            &|| false,
        )
        .unwrap();
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
        assert_eq!(plan.identity(), binding.preparation_identity);
        let prepared = preparation::prepare(
            &catalog,
            &plan,
            plan.selection(),
            &PreparationRoot::open(&self.path("preparation")).unwrap(),
            plan.limits(),
            &|| false,
        )
        .unwrap();
        (prepared, catalog)
    }

    fn unchanged(&self) {
        assert_eq!(
            fs::read(self.path("limeos-config/core.json")).unwrap(),
            b"live configuration"
        );
        assert_eq!(
            fs::read(self.path("app-config/sentinel")).unwrap(),
            b"live application"
        );
        assert_eq!(
            fs::read(self.path("unrelated")).unwrap(),
            b"outside sentinel"
        );
    }
}

struct Inputs {
    custody_root: CustodyRoot,
    staging_root: StagingRoot,
    preparation_root: PreparationRoot,
    admission_policy: AdmissionPolicy,
    custody_limits: CustodyLimits,
    selection: Selection,
    preparation_limits: PreparationLimits,
    workspace: String,
}

impl Inputs {
    fn new(scene: &Fixture) -> Self {
        Self {
            custody_root: CustodyRoot::open(&scene.path("custody")).unwrap(),
            staging_root: StagingRoot::open(&scene.path("staging")).unwrap(),
            preparation_root: PreparationRoot::open(&scene.path("preparation")).unwrap(),
            admission_policy: scene.policy(),
            custody_limits: custody_limits(),
            selection: scene.selection(),
            preparation_limits: scene.limits(),
            workspace: WORKSPACE.into(),
        }
    }

    fn run(
        &self,
        binding: &BackupPreparationBinding,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PreparedRestore, RecoveryError> {
        reconstruct(
            binding,
            CurrentInputs {
                custody_root: &self.custody_root,
                staging_root: &self.staging_root,
                preparation_root: &self.preparation_root,
                admission_policy: &self.admission_policy,
                custody_limits: &self.custody_limits,
                selection: self.selection.clone(),
                preparation_limits: self.preparation_limits,
                workspace_policy_identity: &self.workspace,
            },
            cancelled,
        )
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

fn only_attempt(path: &Path) -> PathBuf {
    let names = names(path);
    assert_eq!(names.len(), 1);
    path.join(&names[0])
}

#[derive(Debug, PartialEq, Eq)]
struct DirectoryState {
    names: Vec<String>,
    changed: (i64, i64, i64, i64),
}

impl DirectoryState {
    fn read(path: &Path) -> Self {
        let metadata = fs::metadata(path).unwrap();
        Self {
            names: names(path),
            changed: (
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        }
    }
}

fn custody_snapshot(scene: &Fixture) -> Vec<(String, u32, u64, Vec<u8>)> {
    let mut snapshot = Vec::new();
    for directory in names(&scene.path("custody")) {
        let path = scene.path("custody").join(&directory);
        let stat = fs::metadata(&path).unwrap();
        snapshot.push((directory.clone(), stat.mode(), stat.ino(), Vec::new()));
        for file in names(&path) {
            let file_path = path.join(&file);
            let stat = fs::metadata(&file_path).unwrap();
            snapshot.push((
                format!("{directory}/{file}"),
                stat.mode(),
                stat.ino(),
                fs::read(file_path).unwrap(),
            ));
        }
    }
    snapshot
}

fn no_replacements(scene: &Fixture, before: &DirectoryState) {
    assert_eq!(&DirectoryState::read(&scene.path("preparation")), before);
    assert!(names(&scene.path("staging")).is_empty());
    scene.unchanged();
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
    let (mut objects, mut files) = (0, 0);
    for entry in prepared.entries() {
        objects += 1;
        let metadata = entry.metadata();
        let target = selection
            .resources
            .iter()
            .find(|r| r.id == metadata.resource)
            .unwrap()
            .targets
            .iter()
            .find(|t| t.relative_path == metadata.relative_path)
            .unwrap();
        assert_eq!(metadata.installed, target.installed);
        assert_eq!(metadata.kind, target.kind);
        if let Some(mut reader) = entry.reader().unwrap() {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(
                bytes,
                expected[&(metadata.resource.as_str(), metadata.relative_path.as_str())]
            );
            assert_eq!(metadata.size, bytes.len() as u64);
            files += 1;
        }
    }
    assert_eq!((objects, files), (9, 5));
    prepared.revalidate(&|| false).unwrap();
    scene.unchanged();
}

#[test]
fn genuine_gzip_and_zstd_reconstruction_reads_and_discards_only_fresh_copies() {
    for zstd in [false, true] {
        let (_temp, scene) = Fixture::new(zstd);
        let binding = scene.binding();
        let custody = custody_snapshot(&scene);
        for increase_ceiling in [false, true] {
            let mut inputs = Inputs::new(&scene);
            if increase_ceiling {
                inputs.custody_limits.max_archive_bytes *= 2;
                inputs.custody_limits.max_record_bytes *= 2;
            }
            let prepared = inputs.run(&binding, &|| false).unwrap();
            checked(&prepared, &scene);
            assert_eq!(names(&scene.path("preparation")).len(), 1);
            assert!(names(&scene.path("staging")).is_empty());
            prepared.discard().unwrap();
            assert!(names(&scene.path("preparation")).is_empty());
            assert_eq!(custody_snapshot(&scene), custody);
        }
    }
}

#[test]
fn every_saved_binding_field_refuses_before_replacement_creation() {
    let (_temp, scene) = Fixture::new(false);
    let binding = scene.binding();
    let custody = custody_snapshot(&scene);
    let before = DirectoryState::read(&scene.path("preparation"));
    for field in [
        "format",
        "record",
        "archive-hash",
        "archive-bytes",
        "policy",
        "manifest",
        "preparation",
        "selected-order",
        "workspace",
    ] {
        let mut changed = binding.clone();
        match field {
            "format" => changed.custody.format_version += 1,
            "record" => changed.custody.record_id = "a".repeat(32),
            "archive-hash" => changed.custody.archive_sha256 = "b".repeat(64),
            "archive-bytes" => changed.custody.archive_bytes += 1,
            "policy" => changed.custody.policy_identity = "c".repeat(64),
            "manifest" => changed.custody.manifest_identity = "d".repeat(64),
            "preparation" => changed.preparation_identity = "e".repeat(64),
            "selected-order" => changed.selected_resources.reverse(),
            _ => changed.workspace_policy_identity = "f".repeat(64),
        }
        let error = Inputs::new(&scene).run(&changed, &|| false).unwrap_err();
        match field {
            "format" => assert!(matches!(error.failure, Failure::InvalidExpectedBinding)),
            "record" | "archive-hash" | "archive-bytes" | "policy" | "manifest" => {
                assert!(
                    matches!(error.failure, Failure::Custody(_)),
                    "{field}: {error}"
                );
            }
            _ => assert!(
                matches!(error.failure, Failure::BindingMismatch),
                "{field}: {error}"
            ),
        }
        assert!(error.catalog_cleanup.is_none());
        assert!(error.prepared_cleanup.is_none());
        no_replacements(&scene, &before);
        assert_eq!(custody_snapshot(&scene), custody);
    }
}

#[test]
fn same_revision_complete_admission_policy_changes_refuse() {
    let (_temp, scene) = Fixture::new(true);
    let binding = scene.binding();
    let custody = custody_snapshot(&scene);
    let before = DirectoryState::read(&scene.path("preparation"));
    for field in [
        "formats",
        "resources",
        "root",
        "mappings",
        "archive-prefix",
        "mapping-resource",
        "resource-prefix",
        "compressed",
        "decompressed",
        "file",
        "entries",
        "path",
        "depth",
        "component",
        "metadata",
        "total-metadata",
        "zstd-window",
    ] {
        let mut inputs = Inputs::new(&scene);
        let policy = &mut inputs.admission_policy;
        match field {
            "formats" => policy.formats.reverse(),
            "resources" => policy.resources.reverse(),
            "root" => policy.resources[0].destination_root.push_str("-changed"),
            "mappings" => policy.legacy_mappings.reverse(),
            "archive-prefix" => policy.legacy_mappings[0]
                .archive_prefix
                .push_str("-changed"),
            "mapping-resource" => policy.legacy_mappings[0].resource = "limeos-state".into(),
            "resource-prefix" => policy.legacy_mappings[0].resource_prefix = "changed".into(),
            "compressed" => policy.limits.max_compressed_bytes += 1,
            "decompressed" => policy.limits.max_decompressed_bytes += 1,
            "file" => policy.limits.max_file_bytes += 1,
            "entries" => policy.limits.max_entries += 1,
            "path" => policy.limits.max_path_bytes += 1,
            "depth" => policy.limits.max_path_depth += 1,
            "component" => policy.limits.max_component_bytes += 1,
            "metadata" => policy.limits.max_metadata_bytes += 1,
            "total-metadata" => policy.limits.max_total_metadata_bytes += 1,
            _ => policy.limits.max_zstd_window_log += 1,
        }
        assert_eq!(policy.revision, scene.policy().revision);
        policy.validate().unwrap();
        let error = inputs.run(&binding, &|| false).unwrap_err();
        assert!(
            matches!(&error.failure, Failure::Custody(e) if matches!(e.as_ref(), CustodyError::PolicyChanged)),
            "{field}: {error}"
        );
        no_replacements(&scene, &before);
        assert_eq!(custody_snapshot(&scene), custody);
    }
}

#[test]
fn current_selection_metadata_limits_and_workspace_are_bound_before_prepare() {
    let (_temp, scene) = Fixture::new(false);
    let binding = scene.binding();
    let custody = custody_snapshot(&scene);
    let before = DirectoryState::read(&scene.path("preparation"));
    for field in [
        "uid",
        "gid",
        "mode",
        "resource-order",
        "target-order",
        "root",
        "resources-limit",
        "objects-limit",
        "file-limit",
        "total-limit",
        "record-limit",
        "workspace",
        "missing",
        "extra",
        "kind",
        "unknown-resource",
    ] {
        let mut inputs = Inputs::new(&scene);
        match field {
            "uid" => inputs.selection.resources[0].targets[1].installed.uid += 1,
            "gid" => inputs.selection.resources[0].targets[1].installed.gid += 1,
            "mode" => inputs.selection.resources[0].targets[1].installed.mode = 0o600,
            "resource-order" => inputs.selection.resources.reverse(),
            "target-order" => inputs.selection.resources[0].targets.reverse(),
            "root" => inputs.selection.resources[0]
                .destination_root
                .push_str("-changed"),
            "resources-limit" => inputs.preparation_limits.max_resources += 1,
            "objects-limit" => inputs.preparation_limits.max_objects += 1,
            "file-limit" => inputs.preparation_limits.max_file_bytes += 1,
            "total-limit" => inputs.preparation_limits.max_total_bytes += 1,
            "record-limit" => inputs.preparation_limits.max_record_bytes += 1,
            "workspace" => inputs.workspace = "a".repeat(64),
            "missing" => {
                inputs.selection.resources[0].targets.remove(1);
            }
            "extra" => inputs.selection.resources[0].targets.push(ManagedTarget {
                relative_path: "extra.json".into(),
                kind: EntryKind::File,
                installed: InstalledMetadata {
                    uid: 42000,
                    gid: 42001,
                    mode: 0o640,
                },
            }),
            "kind" => inputs.selection.resources[0].targets[1].kind = EntryKind::Directory,
            _ => inputs.selection.resources[0].id = "unknown".into(),
        }
        let error = inputs.run(&binding, &|| false).unwrap_err();
        if ["missing", "extra", "kind", "unknown-resource"].contains(&field) {
            assert!(
                matches!(&error.failure, Failure::Plan(e) if matches!(e.failure, preparation::Failure::SelectionMismatch)),
                "{field}: {error}"
            );
        } else {
            assert!(
                matches!(error.failure, Failure::BindingMismatch),
                "{field}: {error}"
            );
        }
        assert!(error.catalog_cleanup.is_none());
        assert!(error.prepared_cleanup.is_none());
        no_replacements(&scene, &before);
        assert_eq!(custody_snapshot(&scene), custody);
    }
}

#[test]
fn invalid_identity_and_initial_cancellation_precede_unavailable_archive_io() {
    let (_temp, scene) = Fixture::new(false);
    let binding = scene.binding();
    let mut inputs = Inputs::new(&scene);
    let retained = scene.path(&format!("custody/retained-{}", binding.custody.record_id));
    fs::remove_file(retained.join("archive")).unwrap();
    let before = DirectoryState::read(&scene.path("preparation"));
    let mut invalid = binding.clone();
    invalid.preparation_identity = "invalid".into();
    let error = inputs.run(&invalid, &|| false).unwrap_err();
    assert!(matches!(error.failure, Failure::InvalidExpectedBinding));
    for invalid in ["invalid".into(), "A".repeat(64)] {
        inputs.workspace = invalid;
        let error = inputs.run(&binding, &|| false).unwrap_err();
        assert!(matches!(
            error.failure,
            Failure::InvalidCurrentWorkspaceIdentity
        ));
    }
    inputs.workspace = WORKSPACE.into();
    let error = inputs.run(&binding, &|| true).unwrap_err();
    assert!(matches!(error.failure, Failure::Cancelled));
    no_replacements(&scene, &before);
}

#[test]
fn invalid_or_insufficient_operational_custody_ceilings_refuse() {
    let (_temp, scene) = Fixture::new(true);
    let binding = scene.binding();
    let custody = custody_snapshot(&scene);
    let before = DirectoryState::read(&scene.path("preparation"));
    for field in [
        "zero-archive",
        "zero-record",
        "short-archive",
        "short-record",
    ] {
        let mut inputs = Inputs::new(&scene);
        match field {
            "zero-archive" => inputs.custody_limits.max_archive_bytes = 0,
            "zero-record" => inputs.custody_limits.max_record_bytes = 0,
            "short-archive" => {
                inputs.custody_limits.max_archive_bytes = binding.custody.archive_bytes - 1
            }
            _ => inputs.custody_limits.max_record_bytes = 1,
        }
        let error = inputs.run(&binding, &|| false).unwrap_err();
        assert!(
            matches!(&error.failure, Failure::Custody(e) if match field {
                "zero-archive" | "zero-record" => matches!(e.as_ref(), CustodyError::InvalidLimits),
                "short-archive" => matches!(e.as_ref(), CustodyError::Limit(CustodyLimit::Archive)),
                _ => matches!(e.as_ref(), CustodyError::Limit(CustodyLimit::Record)),
            }),
            "{field}: {error}"
        );
        no_replacements(&scene, &before);
        assert_eq!(custody_snapshot(&scene), custody);
    }
}

#[test]
fn mutated_or_unsafe_exact_custody_objects_are_never_authorized_by_saved_facts() {
    for change in ["archive-bytes", "archive-missing", "archive-link", "record"] {
        let (_temp, scene) = Fixture::new(false);
        let binding = scene.binding();
        let directory = scene.path(&format!("custody/retained-{}", binding.custody.record_id));
        let archive = directory.join("archive");
        match change {
            "archive-bytes" => {
                let mut bytes = fs::read(&archive).unwrap();
                bytes[0] ^= 1;
                fs::set_permissions(&archive, fs::Permissions::from_mode(0o600)).unwrap();
                fs::write(&archive, bytes).unwrap();
                fs::set_permissions(&archive, fs::Permissions::from_mode(0o400)).unwrap();
            }
            "archive-missing" => fs::remove_file(&archive).unwrap(),
            "archive-link" => {
                fs::remove_file(&archive).unwrap();
                symlink(scene.path("unrelated"), &archive).unwrap();
            }
            _ => {
                let record = directory.join("record.json");
                fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
                fs::write(&record, b"{}").unwrap();
                fs::set_permissions(&record, fs::Permissions::from_mode(0o400)).unwrap();
            }
        }
        let before = DirectoryState::read(&scene.path("preparation"));
        let error = Inputs::new(&scene).run(&binding, &|| false).unwrap_err();
        assert!(
            matches!(error.failure, Failure::Custody(_)),
            "{change}: {error}"
        );
        no_replacements(&scene, &before);
        assert!(directory.exists());
    }
}

#[test]
fn observed_copy_cancellation_preserves_primary_and_real_foreign_cleanup_failure() {
    for foreign in [false, true] {
        let (_temp, scene) = Fixture::new(false);
        let binding = scene.binding();
        let custody = custody_snapshot(&scene);
        let triggered = Cell::new(false);
        let cancel = || {
            if triggered.get() {
                return true;
            }
            for attempt in names(&scene.path("preparation")) {
                let attempt = scene.path("preparation").join(attempt);
                if fs::read_dir(&attempt).unwrap().any(|entry| {
                    let metadata = entry.unwrap().metadata().unwrap();
                    metadata.is_file() && metadata.len() > 0
                }) {
                    if foreign {
                        fs::write(attempt.join("foreign"), FOREIGN).unwrap();
                    }
                    triggered.set(true);
                    return true;
                }
            }
            false
        };
        let error = Inputs::new(&scene).run(&binding, &cancel).unwrap_err();
        assert!(triggered.get(), "no actual replacement bytes observed");
        let Failure::Preparation(primary) = &error.failure else {
            panic!("{error}")
        };
        assert!(matches!(primary.failure, preparation::Failure::Cancelled));
        assert!(error.catalog_cleanup.is_none());
        assert!(error.prepared_cleanup.is_none());
        assert!(names(&scene.path("staging")).is_empty());
        if foreign {
            assert!(matches!(
                primary.cleanup.as_ref().unwrap().failure,
                CleanupFailure::ForeignObjects
            ));
            let attempt = only_attempt(&scene.path("preparation"));
            assert_eq!(names(&attempt), vec!["foreign"]);
            assert_eq!(fs::read(attempt.join("foreign")).unwrap(), FOREIGN);
        } else {
            assert!(primary.cleanup.is_none());
            assert!(names(&scene.path("preparation")).is_empty());
        }
        assert_eq!(custody_snapshot(&scene), custody);
        scene.unchanged();
    }
}

#[test]
fn actual_unlinked_opened_roots_preserve_stage_and_preparation_io_refusals() {
    for root in ["staging", "preparation"] {
        let (_temp, scene) = Fixture::new(true);
        let binding = scene.binding();
        let custody = custody_snapshot(&scene);
        let inputs = Inputs::new(&scene);
        fs::remove_dir(scene.path(root)).unwrap();
        let error = inputs.run(&binding, &|| false).unwrap_err();
        let source = match &error.failure {
            Failure::Custody(e) if root == "staging" => match e.as_ref() {
                CustodyError::Staging(StagingError::Io { source, .. }) => source,
                _ => panic!("{error}"),
            },
            Failure::Preparation(e) if root == "preparation" => match &e.failure {
                preparation::Failure::Io { source, .. } => source,
                _ => panic!("{error}"),
            },
            _ => panic!("{error}"),
        };
        assert_eq!(source.raw_os_error(), Some(2));
        assert!(error.catalog_cleanup.is_none());
        assert!(error.prepared_cleanup.is_none());
        assert!(!scene.path(root).exists());
        let other = if root == "staging" {
            "preparation"
        } else {
            "staging"
        };
        assert!(names(&scene.path(other)).is_empty());
        assert_eq!(custody_snapshot(&scene), custody);
        scene.unchanged();
    }
}

#[test]
fn real_catalog_discard_failure_consumes_prepared_owner_and_keeps_cleanup_ambiguity() {
    for prepared_foreign in [false, true] {
        let (_temp, scene) = Fixture::new(false);
        let binding = scene.binding();
        let custody = custody_snapshot(&scene);
        let (prepared, catalog) = scene.owners(&binding);
        let catalog_attempt = only_attempt(&scene.path("staging"));
        fs::write(catalog_attempt.join("foreign"), FOREIGN).unwrap();
        let prepared_attempt = only_attempt(&scene.path("preparation"));
        if prepared_foreign {
            fs::write(prepared_attempt.join("foreign"), FOREIGN).unwrap();
        }
        let error = finish(prepared, catalog, &|| false).unwrap_err();
        assert!(
            matches!(&error.failure, Failure::CatalogDiscard(e) if matches!(e.as_ref(), StagingError::Io { .. }))
        );
        assert!(error.catalog_cleanup.is_none());
        assert_eq!(names(&catalog_attempt), vec!["foreign"]);
        assert_eq!(fs::read(catalog_attempt.join("foreign")).unwrap(), FOREIGN);
        if prepared_foreign {
            let cleanup = error.prepared_cleanup.as_ref().unwrap();
            assert!(matches!(
                cleanup.failure,
                preparation::Failure::DiscardFailed
            ));
            assert!(matches!(
                cleanup.cleanup.as_ref().unwrap().failure,
                CleanupFailure::ForeignObjects
            ));
            assert_eq!(names(&prepared_attempt), vec!["foreign"]);
            assert_eq!(fs::read(prepared_attempt.join("foreign")).unwrap(), FOREIGN);
        } else {
            assert!(error.prepared_cleanup.is_none());
            assert!(names(&scene.path("preparation")).is_empty());
        }
        assert_eq!(custody_snapshot(&scene), custody);
        scene.unchanged();
    }
}

#[test]
fn finalization_cancellation_consumes_both_owners_without_losing_cleanup_errors() {
    let (_temp, scene) = Fixture::new(false);
    let binding = scene.binding();
    let custody = custody_snapshot(&scene);
    let (prepared, catalog) = scene.owners(&binding);
    let catalog_attempt = only_attempt(&scene.path("staging"));
    let prepared_attempt = only_attempt(&scene.path("preparation"));
    fs::write(catalog_attempt.join("foreign"), FOREIGN).unwrap();
    fs::write(prepared_attempt.join("foreign"), FOREIGN).unwrap();
    let error = finish(prepared, catalog, &|| true).unwrap_err();
    assert!(matches!(error.failure, Failure::Cancelled));
    assert!(matches!(
        error.catalog_cleanup.as_deref().unwrap(),
        StagingError::Io { .. }
    ));
    let cleanup = error.prepared_cleanup.as_ref().unwrap();
    assert!(matches!(
        cleanup.failure,
        preparation::Failure::DiscardFailed
    ));
    assert!(matches!(
        cleanup.cleanup.as_ref().unwrap().failure,
        CleanupFailure::ForeignObjects
    ));
    for attempt in [&catalog_attempt, &prepared_attempt] {
        assert_eq!(names(attempt), vec!["foreign"]);
        assert_eq!(fs::read(attempt.join("foreign")).unwrap(), FOREIGN);
    }
    assert_eq!(custody_snapshot(&scene), custody);
    scene.unchanged();

    let (_temp, scene) = Fixture::new(true);
    let binding = scene.binding();
    let (prepared, catalog) = scene.owners(&binding);
    // Observe actual catalog removal; this targets final delivery without poll-count guesses.
    let error = finish(prepared, catalog, &|| {
        names(&scene.path("staging")).is_empty()
    })
    .unwrap_err();
    assert!(matches!(error.failure, Failure::Cancelled));
    assert!(error.catalog_cleanup.is_none());
    assert!(error.prepared_cleanup.is_none());
    assert!(names(&scene.path("staging")).is_empty());
    assert!(names(&scene.path("preparation")).is_empty());
    scene.unchanged();
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
                "recovery::tests::subprocess_driver",
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

    fn kill_at_owner(&mut self, scene: &Fixture) {
        let start = Instant::now();
        loop {
            if scene.path("barrier.json").exists() {
                break;
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "child exited: {}",
                self.logs()
            );
            assert!(
                start.elapsed() < WAIT,
                "owner barrier timed out: {}",
                self.logs()
            );
            thread::sleep(Duration::from_millis(10));
        }
        let barrier: Value =
            serde_json::from_slice(&fs::read(scene.path("barrier.json")).unwrap()).unwrap();
        assert_eq!(barrier["stage"], "private-owner");
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
    let binding: BackupPreparationBinding =
        serde_json::from_slice(&fs::read(scene.path("binding.json")).unwrap()).unwrap();
    let custody = custody_snapshot(&scene);
    let stale = names(&scene.path("preparation"));
    let prepared = Inputs::new(&scene).run(&binding, &|| false).unwrap();
    checked(&prepared, &scene);
    assert!(names(&scene.path("staging")).is_empty());
    let attempts = names(&scene.path("preparation"));
    assert_eq!(attempts.len(), stale.len() + 1);
    assert!(stale.iter().all(|name| attempts.contains(name)));
    if mode == "killed" {
        save(
            &scene.path("barrier.json"),
            &json!({"stage":"private-owner", "attempts": attempts, "record_sha256": prepared.record_sha256()}),
        );
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
        panic!("owner barrier unexpectedly resumed");
    }
    assert_eq!(mode, "fresh");
    let barrier: Value =
        serde_json::from_slice(&fs::read(scene.path("barrier.json")).unwrap()).unwrap();
    assert_eq!(barrier["attempts"], json!(stale));
    assert_eq!(barrier["record_sha256"], prepared.record_sha256());
    prepared.discard().unwrap();
    assert_eq!(names(&scene.path("preparation")), stale);
    assert_eq!(custody_snapshot(&scene), custody);
    save(
        &scene.path("fresh.json"),
        &json!({"stale":stale,"attempts":attempts,"discarded":true}),
    );
    scene.unchanged();
}

#[test]
fn sigkill_private_owner_is_not_adopted_by_fresh_process_reconstruction() {
    let (_temp, scene) = Fixture::new(true);
    let binding = scene.binding();
    save(&scene.path("binding.json"), &json!(binding));
    let custody = custody_snapshot(&scene);
    OwnedChild::spawn(&scene, "killed").kill_at_owner(&scene);
    let stale = names(&scene.path("preparation"));
    assert_eq!(stale.len(), 1);
    let stale_attempt = scene.path("preparation").join(&stale[0]);
    let stale_objects: Vec<_> = names(&stale_attempt)
        .into_iter()
        .map(|name| {
            let path = stale_attempt.join(&name);
            let metadata = fs::metadata(&path).unwrap();
            let bytes = if metadata.is_file() {
                fs::read(path).unwrap()
            } else {
                Vec::new()
            };
            (name, metadata.ino(), metadata.mode(), bytes)
        })
        .collect();
    OwnedChild::spawn(&scene, "fresh").finish();
    assert_eq!(names(&scene.path("preparation")), stale);
    for (name, inode, mode, bytes) in stale_objects {
        let path = stale_attempt.join(name);
        let metadata = fs::metadata(&path).unwrap();
        assert_eq!((metadata.ino(), metadata.mode()), (inode, mode));
        if metadata.is_file() {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
    assert_eq!(custody_snapshot(&scene), custody);
    assert!(names(&scene.path("staging")).is_empty());
    assert!(scene.path("fresh.json").exists());
    scene.unchanged();
    // TempDir teardown deletes test-owned leftovers; reconstruction never does.
}
