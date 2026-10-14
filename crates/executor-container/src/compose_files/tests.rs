use super::*;
use std::{
    cell::Cell,
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    },
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Barrier, atomic::AtomicBool},
};
use tempfile::TempDir;

const OPERATOR: &[u8] = include_bytes!("../../../../tests/fixtures/compose-files/operator.yaml");
const MANAGED: &[u8] =
    include_bytes!("../../../../tests/fixtures/compose-files/limeos.override.json");
fn modes(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
fn selection(name: ComposeFileName, managed: bool) -> ComposeFileSelection {
    ComposeFileSelection::new(Path::new("/stacks/demo"), name, managed).unwrap()
}
fn refusal<T>(result: Result<T>, code: ErrorCode) {
    match result {
        Err(error) => assert_eq!(error.0, code),
        Ok(_) => panic!("expected refusal: {code:?}"),
    }
}

fn json_fixture(managed: bool) -> Fixture {
    let f = Fixture::new(ComposeFileName::ComposeYaml, managed);
    f.write(
        "compose.yaml",
        include_bytes!("../../../../tests/fixtures/compose-source/operator.json"),
    );
    if managed {
        f.write(
            COMPOSE_MANAGED_FILE,
            include_bytes!("../../../../tests/fixtures/compose-override/merge-delta.override.json"),
        );
    }
    f
}

#[test]
fn typed_body_read_uses_original_ledger_and_preserves_issues_and_descriptors() {
    for managed in [false, true] {
        let f = json_fixture(managed);
        let e = f.acquire();
        let before = e.budget.spent.load(Ordering::Relaxed);
        let count = e.retained_descriptor_count();
        let sources = e
            .admit_json_with(&|| false, || Ok(f.root()), || Ok(101))
            .unwrap();
        assert!(e.budget.spent.load(Ordering::Relaxed) > before);
        assert_eq!(e.snapshot.observed_at, 100);
        assert_eq!(e.retained_descriptor_count(), count);
        assert_eq!(
            sources.summary().operator_fingerprint.sha256,
            e.snapshot.files[0].sha256
        );
        assert_eq!(sources.summary().managed_fingerprint.is_some(), managed);
        e.budget.spent.store(READ_BYTES, Ordering::Relaxed);
        refusal(
            e.admit_json_with(&|| false, || Ok(f.root()), || Ok(101)),
            ErrorCode::Unavailable,
        );
        refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    }
}

#[test]
fn held_body_reads_charge_requested_bytes_and_eof_and_refuse_changed_bytes() {
    let f = json_fixture(false);
    let e = f.acquire();
    let before = e.budget.spent.load(Ordering::Relaxed);
    let body = read_body(
        &e.files[0],
        e.owner,
        &e.budget,
        Instant::now() + WORK,
        &|| false,
    )
    .unwrap();
    assert_eq!(body, fs::read(f.path("compose.yaml")).unwrap());
    assert_eq!(
        e.budget.spent.load(Ordering::Relaxed) - before,
        body.len() as u64 + 2
    );
    f.write("compose.yaml", b"{}");
    refusal(
        read_body(
            &e.files[0],
            e.owner,
            &e.budget,
            Instant::now() + WORK,
            &|| false,
        ),
        ErrorCode::Conflict,
    );
}

#[test]
fn typed_admission_parse_failure_cancel_expiry_reentrancy_and_unwind_poison() {
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let e = f.acquire();
    refusal(
        e.admit_json_with(&|| false, || Ok(f.root()), || Ok(101)),
        ErrorCode::InvalidInput,
    );
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    for kind in 0..4 {
        let f = json_fixture(false);
        let e = f.acquire();
        match kind {
            0 => refusal(
                e.admit_json_with(&|| true, || Ok(f.root()), || Ok(101)),
                ErrorCode::Conflict,
            ),
            1 => refusal(
                e.admit_json_with(&|| false, || Ok(f.root()), || Ok(105)),
                ErrorCode::Expired,
            ),
            2 => {
                let called = Cell::new(false);
                refusal(
                    e.admit_json_with(
                        &|| {
                            if !called.replace(true) {
                                refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
                            }
                            false
                        },
                        || Ok(f.root()),
                        || Ok(101),
                    ),
                    ErrorCode::Conflict,
                );
            }
            _ => assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    let _ = e.admit_json_with(
                        &|| panic!("owned typed admission unwind"),
                        || Ok(f.root()),
                        || Ok(101),
                    );
                }))
                .is_err()
            ),
        }
        refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    }
}

#[test]
fn late_named_replacement_after_body_admission_refuses_and_poisoned_owner_cannot_retry() {
    let f = json_fixture(false);
    let e = f.acquire();
    let roots = Cell::new(0);
    refusal(
        e.admit_json_with(
            &|| false,
            || {
                let root = f.root();
                roots.set(roots.get() + 1);
                if roots.get() == 2 {
                    fs::rename(f.path("compose.yaml"), f.path("old.json")).unwrap();
                    f.write(
                        "compose.yaml",
                        include_bytes!("../../../../tests/fixtures/compose-source/operator.json"),
                    );
                }
                Ok(root)
            },
            || Ok(101),
        ),
        ErrorCode::Conflict,
    );
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
}

#[test]
fn held_body_maximum_sentinel_growth_and_identity_change_during_read_refuse() {
    let f = json_fixture(false);
    let mut raw = br#"{"services":{}}"#.to_vec();
    raw.resize(FILE_BYTES as usize, b' ');
    f.write("compose.yaml", &raw);
    let e = f.acquire();
    assert_eq!(
        read_body(
            &e.files[0],
            e.owner,
            &e.budget,
            Instant::now() + WORK,
            &|| false
        )
        .unwrap(),
        raw
    );
    for grow in [false, true] {
        let f = json_fixture(false);
        let e = f.acquire();
        let polls = Cell::new(0);
        refusal(
            read_body(
                &e.files[0],
                e.owner,
                &e.budget,
                Instant::now() + WORK,
                &|| {
                    polls.set(polls.get() + 1);
                    if polls.get() == 2 {
                        let raw = if grow {
                            let mut raw = fs::read(f.path("compose.yaml")).unwrap();
                            raw.push(b' ');
                            raw
                        } else {
                            b"changed held bytes during read".to_vec()
                        };
                        fs::write(f.path("compose.yaml"), raw).unwrap();
                    }
                    false
                },
            ),
            ErrorCode::Conflict,
        );
    }
}
struct Fixture {
    tree: TempDir,
    selection: ComposeFileSelection,
}
impl Fixture {
    fn new(name: ComposeFileName, managed: bool) -> Self {
        let tree = tempfile::tempdir().unwrap();
        fs::create_dir_all(tree.path().join("stacks/demo")).unwrap();
        for path in [
            tree.path().to_owned(),
            tree.path().join("stacks"),
            tree.path().join("stacks/demo"),
        ] {
            modes(&path, 0o700);
        }
        let f = Self {
            tree,
            selection: selection(name, managed),
        };
        f.write(base_name(name), OPERATOR);
        if managed {
            f.write(COMPOSE_MANAGED_FILE, MANAGED);
        }
        f
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.tree.path().join("stacks/demo").join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        fs::write(self.path(name), bytes).unwrap();
        modes(&self.path(name), 0o600);
    }
    fn root(&self) -> Root {
        Root {
            fd: rustix::fs::open(
                self.tree.path(),
                OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap(),
            view: None,
        }
    }
    fn raw(&self) -> Result<ComposeFileEvidence> {
        collect(
            self.root(),
            &self.selection,
            rustix::process::geteuid().as_raw(),
            100,
            Budget {
                spent: AtomicU64::new(0),
                expires: Instant::now() + LIFETIME,
            },
            Instant::now() + WORK,
            &|| false,
        )
    }
    fn validate(
        &self,
        evidence: &ComposeFileEvidence,
        at: i64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        evidence
            .checked(|| evidence.validate_from(&self.root(), at, Instant::now() + WORK, cancelled))
    }
    fn acquire(&self) -> ComposeFileEvidence {
        let e = self.raw().unwrap();
        self.validate(&e, 101, &|| false).unwrap();
        e
    }
}

#[test]
fn recognized_bases_optional_managed_and_opaque_bytes_preserve_original_facts() {
    for name in [
        ComposeFileName::ComposeYaml,
        ComposeFileName::ComposeYml,
        ComposeFileName::DockerComposeYaml,
        ComposeFileName::DockerComposeYml,
    ] {
        for managed in [false, true] {
            let f = Fixture::new(name, managed);
            let e = f.acquire();
            assert_eq!(e.snapshot.version, 1);
            assert_eq!(e.snapshot.observed_at, 100);
            assert_eq!(e.snapshot.selection.operator(), name);
            assert_eq!(
                e.snapshot.selection.working_directory(),
                Path::new("/stacks/demo")
            );
            assert_eq!(e.snapshot.selection.includes_managed(), managed);
            assert_eq!(e.snapshot.files.len(), 1 + usize::from(managed));
            assert_eq!(
                e.snapshot.files[0].path,
                format!("/stacks/demo/{}", base_name(name))
            );
            assert_eq!(
                e.snapshot.files[0].sha256,
                hex::encode(Sha256::digest(OPERATOR))
            );
            assert_eq!(e.snapshot.files[0].identity.size, OPERATOR.len() as u64);
            if managed {
                assert_eq!(
                    e.snapshot.files[1].sha256,
                    hex::encode(Sha256::digest(MANAGED))
                );
            }
            assert_eq!(e.snapshot.absent_names.len(), 4 - usize::from(managed));
            let expiry = e.budget.expires;
            let spent = e.budget.spent.load(Ordering::Relaxed);
            f.validate(&e, 102, &|| false).unwrap();
            assert_eq!(e.budget.expires, expiry);
            assert!(e.budget.spent.load(Ordering::Relaxed) > spent);
            let report = serde_json::to_string(e.snapshot()).unwrap();
            assert!(!report.contains("SYNTHETIC_SECRET"));
            assert!(!report.contains("services"));
            assert_eq!(fs::read(f.path(base_name(name))).unwrap(), OPERATOR);
            assert_eq!(e.retained_descriptor_count(), 4 + usize::from(managed));
        }
    }
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    f.write("compose.yaml", &[0, 255, 1, 128]);
    assert_eq!(
        f.acquire().snapshot.files[0].sha256,
        hex::encode(Sha256::digest([0, 255, 1, 128]))
    );
}

#[test]
fn selection_admits_literal_paths_and_refuses_normalization_aliases_and_excess() {
    for path in [
        "",
        "/",
        "relative",
        "/stacks/",
        "/stacks//demo",
        "/stacks/./demo",
        "/stacks/../demo",
        "/a\nb",
        "/a,b",
        "/a\0b",
    ] {
        refusal(
            ComposeFileSelection::new(Path::new(path), ComposeFileName::ComposeYaml, false),
            ErrorCode::InvalidInput,
        );
    }
    assert!(
        ComposeFileSelection::new(
            Path::new("/literal space/UPPER-é"),
            ComposeFileName::ComposeYaml,
            false
        )
        .is_ok()
    );
    let path = format!("/{}", "a/".repeat(14) + "a");
    assert!(
        ComposeFileSelection::new(Path::new(&path), ComposeFileName::ComposeYaml, true).is_ok()
    );
    refusal(
        ComposeFileSelection::new(
            Path::new(&(path + "/a")),
            ComposeFileName::ComposeYaml,
            true,
        ),
        ErrorCode::InvalidInput,
    );
    let longest = BASES
        .into_iter()
        .chain([COMPOSE_MANAGED_FILE])
        .map(str::len)
        .max()
        .unwrap();
    let remaining = 512 - 1 - longest;
    let path = format!("/{}/{}", "a".repeat(240), "b".repeat(remaining - 242));
    assert_eq!(path.len() + 1 + longest, 512);
    assert!(
        ComposeFileSelection::new(Path::new(&path), ComposeFileName::ComposeYaml, true).is_ok()
    );
    refusal(
        ComposeFileSelection::new(Path::new(&(path + "b")), ComposeFileName::ComposeYaml, true),
        ErrorCode::InvalidInput,
    );
    assert!(
        ComposeFileSelection::new(
            Path::new(&format!("/{}", "a".repeat(255))),
            ComposeFileName::ComposeYaml,
            false
        )
        .is_ok()
    );
    refusal(
        ComposeFileSelection::new(
            Path::new(&format!("/{}", "a".repeat(256))),
            ComposeFileName::ComposeYaml,
            false,
        ),
        ErrorCode::InvalidInput,
    );
    let bad = std::ffi::OsString::from_vec(vec![b'/', 255]);
    refusal(
        ComposeFileSelection::new(Path::new(&bad), ComposeFileName::ComposeYaml, false),
        ErrorCode::InvalidInput,
    );
}

#[test]
fn all_alternate_objects_refuse_without_reading_them() {
    for alternate in &BASES[1..] {
        for kind in 0..4 {
            let f = Fixture::new(ComposeFileName::ComposeYaml, false);
            let p = f.path(alternate);
            match kind {
                0 => f.write(alternate, b"alternate"),
                1 => symlink("missing", &p).unwrap(),
                2 => rustix::fs::mknodat(
                    rustix::fs::CWD,
                    &p,
                    rustix::fs::FileType::Fifo,
                    Mode::RUSR | Mode::WUSR,
                    0,
                )
                .unwrap(),
                _ => fs::create_dir(&p).unwrap(),
            }
            refusal(f.raw(), ErrorCode::Conflict);
        }
    }
}
#[test]
fn mandatory_operator_and_explicit_managed_presence_never_default() {
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    fs::remove_file(f.path("compose.yaml")).unwrap();
    refusal(f.raw(), ErrorCode::Conflict);
    let f = Fixture::new(ComposeFileName::ComposeYaml, true);
    fs::remove_file(f.path(COMPOSE_MANAGED_FILE)).unwrap();
    refusal(f.raw(), ErrorCode::Conflict);
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    f.write(COMPOSE_MANAGED_FILE, MANAGED);
    refusal(f.raw(), ErrorCode::Conflict);
}
#[test]
fn selected_special_symlink_directory_and_hardlink_leaves_refuse_before_content() {
    for managed in [false, true] {
        for kind in 0..4 {
            let f = Fixture::new(ComposeFileName::ComposeYaml, managed);
            let name = if managed {
                COMPOSE_MANAGED_FILE
            } else {
                "compose.yaml"
            };
            let p = f.path(name);
            fs::remove_file(&p).unwrap();
            match kind {
                0 => symlink("missing", &p).unwrap(),
                1 => rustix::fs::mknodat(
                    rustix::fs::CWD,
                    &p,
                    rustix::fs::FileType::Fifo,
                    Mode::RUSR | Mode::WUSR,
                    0,
                )
                .unwrap(),
                2 => fs::create_dir(&p).unwrap(),
                _ => {
                    f.write(name, b"body");
                    fs::hard_link(&p, f.path("alias")).unwrap();
                }
            }
            refusal(f.raw(), ErrorCode::Unavailable);
        }
    }
}
#[test]
fn unprotected_modes_and_wrong_owner_refuse_root_ancestors_and_files() {
    for mode in [0o620, 0o602] {
        for path in ["", "stacks", "stacks/demo", "stacks/demo/compose.yaml"] {
            let f = Fixture::new(ComposeFileName::ComposeYaml, false);
            modes(&f.tree.path().join(path), mode);
            refusal(f.raw(), ErrorCode::Unavailable);
        }
    }
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let e = f.acquire();
    refusal(directory(&e.root.fd, e.owner + 1), ErrorCode::Unavailable);
    refusal(
        file_identity(&e.files[0].fd, e.owner + 1),
        ErrorCode::Unavailable,
    );
}
#[test]
fn symlink_and_non_directory_ancestors_refuse() {
    for kind in 0..2 {
        let f = Fixture::new(ComposeFileName::ComposeYaml, false);
        fs::rename(f.tree.path().join("stacks"), f.tree.path().join("old")).unwrap();
        if kind == 0 {
            symlink("old", f.tree.path().join("stacks")).unwrap();
        } else {
            fs::write(f.tree.path().join("stacks"), b"not directory").unwrap();
        }
        refusal(f.raw(), ErrorCode::Unavailable);
    }
}
#[test]
fn changed_absence_or_removed_selection_permanently_invalidates() {
    for name in BASES.into_iter().chain([COMPOSE_MANAGED_FILE]) {
        let f = Fixture::new(ComposeFileName::ComposeYaml, false);
        let e = f.acquire();
        if name == "compose.yaml" {
            fs::remove_file(f.path(name)).unwrap();
        } else {
            f.write(name, b"new");
        }
        assert!(f.validate(&e, 101, &|| false).is_err());
        if name == "compose.yaml" {
            f.write(name, OPERATOR);
        } else {
            fs::remove_file(f.path(name)).unwrap();
        }
        refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    }
}
#[test]
fn changed_bytes_metadata_or_named_inode_permanently_invalidates() {
    for kind in 0..6 {
        let f = Fixture::new(ComposeFileName::ComposeYaml, true);
        let e = f.acquire();
        let path = f.path(COMPOSE_MANAGED_FILE);
        match kind {
            0 => f.write(COMPOSE_MANAGED_FILE, &[b'x'; MANAGED.len()]),
            1 => f.write(COMPOSE_MANAGED_FILE, b"short"),
            2 => {
                fs::rename(&path, f.path("old")).unwrap();
                f.write(COMPOSE_MANAGED_FILE, MANAGED);
            }
            3 => modes(&path, 0o640),
            4 => {
                fs::hard_link(&path, f.path("alias")).unwrap();
            }
            _ => {
                fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_len(FILE_BYTES + 1)
                    .unwrap();
            }
        }
        assert!(f.validate(&e, 101, &|| false).is_err());
        refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    }
}
#[test]
fn detached_changed_ancestor_refuses_but_unrelated_entries_are_allowed() {
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let e = f.acquire();
    f.write("unrelated", b"entry");
    f.validate(&e, 101, &|| false).unwrap();
    fs::rename(
        f.tree.path().join("stacks/demo"),
        f.tree.path().join("stacks/old"),
    )
    .unwrap();
    fs::create_dir(f.tree.path().join("stacks/demo")).unwrap();
    f.write("compose.yaml", OPERATOR);
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
}
#[test]
fn changes_during_collection_are_refused_by_original_identity_checks() {
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let polls = Cell::new(0);
    let cancelled = || {
        let n = polls.get() + 1;
        polls.set(n);
        if n == 14 {
            f.write("compose.yml", b"late alternate");
        }
        false
    };
    let result = collect(
        f.root(),
        &f.selection,
        rustix::process::geteuid().as_raw(),
        100,
        Budget {
            spent: AtomicU64::new(0),
            expires: Instant::now() + LIFETIME,
        },
        Instant::now() + WORK,
        &cancelled,
    );
    assert!(polls.get() >= 14);
    refusal(result, ErrorCode::Conflict);
}
#[test]
fn replacement_after_fresh_read_is_detected_before_validation_returns() {
    for complete_reads in [1, 2] {
        let f = Fixture::new(ComposeFileName::ComposeYaml, false);
        let e = f.acquire();
        let replaced = Cell::new(false);
        let start = e.budget.spent.load(Ordering::Relaxed);
        let cancelled = || {
            if !replaced.get()
                && e.budget.spent.load(Ordering::Relaxed)
                    >= start + complete_reads * (OPERATOR.len() as u64 + 1)
            {
                replaced.set(true);
                fs::rename(f.path("compose.yaml"), f.path("old")).unwrap();
                f.write("compose.yaml", OPERATOR);
            }
            false
        };
        refusal(f.validate(&e, 101, &cancelled), ErrorCode::Conflict);
        assert!(replaced.get());
    }
}
#[test]
fn late_directory_or_absence_change_after_content_checks_refuses() {
    for detach_directory in [false, true] {
        let f = Fixture::new(ComposeFileName::ComposeYaml, false);
        let e = f.acquire();
        let changed = Cell::new(false);
        let start = e.budget.spent.load(Ordering::Relaxed);
        let cancelled = || {
            if !changed.get()
                && e.budget.spent.load(Ordering::Relaxed) >= start + 2 * (OPERATOR.len() as u64 + 1)
            {
                changed.set(true);
                if detach_directory {
                    fs::rename(
                        f.tree.path().join("stacks/demo"),
                        f.tree.path().join("stacks/old"),
                    )
                    .unwrap();
                    fs::create_dir(f.tree.path().join("stacks/demo")).unwrap();
                    f.write("compose.yaml", OPERATOR);
                } else {
                    symlink("missing", f.path("compose.yml")).unwrap();
                }
            }
            false
        };
        refusal(f.validate(&e, 101, &cancelled), ErrorCode::Conflict);
        assert!(changed.get());
        refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    }
}
#[test]
fn exact_empty_maximum_file_bounds_and_next_byte_refusal() {
    for bytes in [0, FILE_BYTES as usize] {
        let f = Fixture::new(ComposeFileName::ComposeYaml, true);
        f.write("compose.yaml", &vec![0xff; bytes]);
        f.write(COMPOSE_MANAGED_FILE, &vec![0; bytes]);
        let e = f.acquire();
        assert_eq!(e.snapshot.files[0].identity.size, bytes as u64);
        assert_eq!(e.snapshot.files[1].identity.size, bytes as u64);
    }
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    f.write("compose.yaml", &vec![0; FILE_BYTES as usize + 1]);
    refusal(f.raw(), ErrorCode::Unavailable);
}
#[test]
fn maximum_depth_and_fixed_retained_descriptor_bound() {
    let mut f = Fixture::new(ComposeFileName::ComposeYaml, true);
    let relative = std::iter::repeat_n("a", 15).collect::<Vec<_>>().join("/");
    let path = f.tree.path().join(&relative);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("compose.yaml"), OPERATOR).unwrap();
    fs::write(path.join(COMPOSE_MANAGED_FILE), MANAGED).unwrap();
    f.selection = ComposeFileSelection::new(
        Path::new(&format!("/{relative}")),
        ComposeFileName::ComposeYaml,
        true,
    )
    .unwrap();
    let e = f.acquire();
    assert_eq!(e.ancestors.len(), 15);
    assert_eq!(e.retained_descriptor_count(), 18);
    // Production adds its retained namespace reference to the same 18 file/root descriptors.
    assert_eq!(e.retained_descriptor_count() + 1, RETAINED_FDS);
}
#[test]
fn spent_read_ledger_never_renews_and_fails_before_excess_read() {
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let e = f.acquire();
    e.budget
        .spent
        .store(READ_BYTES - OPERATOR.len() as u64 - 1, Ordering::Relaxed);
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Unavailable);
    assert_eq!(e.budget.spent.load(Ordering::Relaxed), READ_BYTES);
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    let budget = Budget {
        spent: AtomicU64::new(READ_BYTES - 1),
        expires: Instant::now() + LIFETIME,
    };
    budget.charge(1).unwrap();
    refusal(budget.charge(1), ErrorCode::Unavailable);
    assert_eq!(budget.spent.load(Ordering::Relaxed), READ_BYTES);
}
#[test]
fn compact_report_count_uses_encoded_bytes_at_exact_boundary() {
    let f = Fixture::new(ComposeFileName::ComposeYaml, true);
    let mut e = f.acquire();
    e.snapshot.selection.working_directory = "/quote\"/é".into();
    let size = serde_json::to_vec(&e.snapshot).unwrap().len();
    check_report(&e.snapshot, size).unwrap();
    refusal(check_report(&e.snapshot, size - 1), ErrorCode::Unavailable);
    check_report(&e.snapshot, REPORT_BYTES).unwrap();
}
#[test]
fn original_wall_and_monotonic_lifetimes_and_work_deadline_do_not_renew() {
    for at in [99, 105] {
        let f = Fixture::new(ComposeFileName::ComposeYaml, false);
        let e = f.acquire();
        refusal(f.validate(&e, at, &|| false), ErrorCode::Expired);
        refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    }
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let e = f.acquire();
    let expiry = e.budget.expires;
    f.validate(&e, 104, &|| false).unwrap();
    assert_eq!(e.budget.expires, expiry);
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let mut e = f.acquire();
    e.budget.expires = Instant::now();
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Expired);
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let e = f.acquire();
    refusal(
        e.checked(|| e.validate_from(&f.root(), 101, Instant::now(), &|| false)),
        ErrorCode::Expired,
    );
}
#[test]
fn cancellation_between_read_chunks_and_retry_permanently_refuse() {
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    f.write("compose.yaml", &vec![7; FILE_BYTES as usize]);
    let e = f.acquire();
    let before = e.budget.spent.load(Ordering::Relaxed);
    refusal(
        f.validate(&e, 101, &|| e.budget.spent.load(Ordering::Relaxed) > before),
        ErrorCode::Conflict,
    );
    assert_eq!(e.budget.spent.load(Ordering::Relaxed), before + 4096);
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    let cancelled = || true;
    refusal(
        collect(
            f.root(),
            &f.selection,
            rustix::process::geteuid().as_raw(),
            100,
            Budget {
                spent: AtomicU64::new(0),
                expires: Instant::now() + LIFETIME,
            },
            Instant::now() + WORK,
            &cancelled,
        ),
        ErrorCode::Conflict,
    );
}
#[test]
fn reentrancy_and_unwinding_cannot_restore_readiness() {
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let e = f.acquire();
    refusal(
        e.checked(|| {
            refusal(e.checked(|| Ok(())), ErrorCode::Conflict);
            Ok(())
        }),
        ErrorCode::Conflict,
    );
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    let e = f.acquire();
    assert!(
        catch_unwind(AssertUnwindSafe(
            || e.checked(|| panic!("owned validation unwind"))
        ))
        .is_err()
    );
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
}
#[test]
fn concurrent_validation_refusal_invalidates_both_checks() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<ComposeFileEvidence>();
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let e = f.acquire();
    let entered = Barrier::new(2);
    let release = Barrier::new(2);
    let once = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            f.validate(&e, 101, &|| {
                if !once.swap(true, Ordering::SeqCst) {
                    entered.wait();
                    release.wait();
                }
                false
            })
        });
        entered.wait();
        refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
        release.wait();
        refusal(first.join().unwrap(), ErrorCode::Conflict);
    });
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
}
#[test]
fn root_identity_or_mount_view_disagreement_refuses() {
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    let mut e = f.acquire();
    e.snapshot.root.mount_id += 1;
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    let mut e = f.acquire();
    e.snapshot.mount_view = Some(ComposeMountView {
        device: 1,
        inode: 1,
    });
    refusal(f.validate(&e, 101, &|| false), ErrorCode::Conflict);
    let other = Fixture::new(ComposeFileName::ComposeYaml, false);
    let e = f.acquire();
    refusal(
        e.checked(|| e.validate_from(&other.root(), 101, Instant::now() + WORK, &|| false)),
        ErrorCode::Conflict,
    );
}
#[test]
fn namespace_identity_primitive_requires_real_nsfs_reference() {
    let f = Fixture::new(ComposeFileName::ComposeYaml, false);
    refusal(view_identity(&f.root().fd), ErrorCode::Unavailable);
    let open = || {
        rustix::fs::open(
            "/proc/self/ns/mnt",
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap()
    };
    let first = open();
    let second = open();
    assert_eq!(
        view_identity(&first).unwrap(),
        view_identity(&second).unwrap()
    );
}
#[test]
fn public_nonroot_refusal_precedes_namespace_and_target_access() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let selection = ComposeFileSelection::new(
        Path::new("/limeos-owned-missing-file-fixture"),
        ComposeFileName::ComposeYaml,
        true,
    )
    .unwrap();
    let touched = Cell::new(false);
    refusal(
        inspect_compose_files(&selection, &|| {
            touched.set(true);
            false
        }),
        ErrorCode::Forbidden,
    );
    assert!(!touched.get());
}
