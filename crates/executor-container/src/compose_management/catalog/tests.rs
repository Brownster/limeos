use super::*;
use crate::compose_management::tests::{configuration, refusal};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    panic::{AssertUnwindSafe, catch_unwind},
};
use tempfile::TempDir;
fn parse(value: Value) -> Result<ManagementCatalog> {
    let c: ManagementCatalog = serde_json::from_value(value).map_err(|_| invalid())?;
    c.validate()?;
    Ok(c)
}
struct Fixture {
    tree: TempDir,
}
impl Fixture {
    fn new() -> Self {
        let tree = tempfile::tempdir().unwrap();
        fs::create_dir(tree.path().join("config")).unwrap();
        fs::set_permissions(
            tree.path().join("config"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let f = Self { tree };
        f.write(&serde_json::to_vec(&configuration()).unwrap());
        f
    }
    fn path(&self) -> std::path::PathBuf {
        self.tree.path().join("config/catalog.json")
    }
    fn write(&self, bytes: &[u8]) {
        fs::write(self.path(), bytes).unwrap();
        fs::set_permissions(self.path(), fs::Permissions::from_mode(0o600)).unwrap();
    }
    fn acquire(&self) -> Result<ManagementCatalogEvidence> {
        local_catalog(self.tree.path(), Path::new("/config/catalog.json"))
    }
}

#[test]
fn closed_and_duplicate_fields_wrong_types_and_missing_values_refuse() {
    for text in [
        "{}",
        "null",
        "[]",
        "{\"version\":1,\"version\":1,\"engine_id\":\"fixture-engine\",\"stacks\":[]}",
    ] {
        assert!(serde_json::from_str::<ManagementCatalog>(text).is_err());
    }
    for path in ["version", "engine_id", "stacks"] {
        let mut v = configuration();
        v.as_object_mut().unwrap().remove(path);
        assert!(parse(v).is_err());
    }
    for field in [
        "stack",
        "project",
        "working_directory",
        "operator",
        "managed",
        "services",
    ] {
        let mut v = configuration();
        v["stacks"][0].as_object_mut().unwrap().remove(field);
        assert!(parse(v).is_err());
    }
    let mut v = configuration();
    v["unknown"] = json!([1, 2]);
    assert!(parse(v).is_err());
    let mut v = configuration();
    v["stacks"][0]["managed"] = json!({"state":"absent","sha256":"a".repeat(64)});
    assert!(parse(v).is_err());
    let raw = serde_json::to_string(&configuration()).unwrap().replace(
        "\"stack\":\"media-stack\"",
        "\"stack\":\"media-stack\",\"sta\\u0063k\":\"media-stack\"",
    );
    assert!(serde_json::from_str::<ManagementCatalog>(&raw).is_err());
}
#[test]
fn counts_and_strings_refuse_before_excess_decoding_or_growth() {
    let mut v = configuration();
    let stacks = (0..8)
        .map(|i| {
            let mut s = v["stacks"][0].clone();
            s["stack"] = json!(format!("stack-{i}"));
            s["project"] = json!(format!("project-{i}"));
            s["working_directory"] = json!(format!("/opt/stacks/project-{i}"));
            s
        })
        .collect::<Vec<_>>();
    v["stacks"] = json!(stacks);
    parse(v.clone()).unwrap();
    v["stacks"].as_array_mut().unwrap().push(Value::Null);
    assert!(parse(v).is_err());
    let mut v = configuration();
    v["stacks"][0]["services"] = json!((0..16).map(|i| format!("service-{i}")).collect::<Vec<_>>());
    parse(v.clone()).unwrap();
    v["stacks"][0]["services"]
        .as_array_mut()
        .unwrap()
        .push(json!(["do not decode"]));
    assert!(parse(v).is_err());
    for (field, bytes) in [("stack", 65), ("project", 65), ("working_directory", 513)] {
        let mut v = configuration();
        v["stacks"][0][field] = json!("a".repeat(bytes));
        assert!(parse(v).is_err());
    }
    let mut v = configuration();
    v["engine_id"] = json!("a".repeat(128));
    parse(v.clone()).unwrap();
    v["engine_id"] = json!("a".repeat(129));
    assert!(parse(v).is_err());
    let mut v = configuration();
    v["stacks"][0]["services"] = json!(["a".repeat(65)]);
    assert!(parse(v).is_err());
}
#[test]
fn malformed_names_digests_paths_and_explicit_empty_configuration_refuse() {
    for field in ["stack", "project"] {
        for name in ["", "../media", "MEDIA", "a b", "a/b", "a\nb"] {
            let mut v = configuration();
            v["stacks"][0][field] = json!(name);
            assert!(parse(v).is_err());
        }
    }
    for hash in ["", "abc", &"A".repeat(64), &"a".repeat(65)] {
        for managed in [false, true] {
            let mut v = configuration();
            if managed {
                v["stacks"][0]["managed"]["sha256"] = json!(hash);
            } else {
                v["stacks"][0]["operator"]["sha256"] = json!(hash);
            }
            assert!(parse(v).is_err());
        }
    }
    for path in ["/", "relative", "/a//b", "/a/../b", "/a/", "/a,b"] {
        let mut v = configuration();
        v["stacks"][0]["working_directory"] = json!(path);
        assert!(parse(v).is_err());
    }
    for field in ["stacks", "services"] {
        let mut v = configuration();
        if field == "stacks" {
            v[field] = json!([]);
        } else {
            v["stacks"][0][field] = json!([]);
        }
        assert!(parse(v).is_err());
    }
    let mut v = configuration();
    v["version"] = json!(2);
    assert!(parse(v).is_err());
    let mut v = configuration();
    v["engine_id"] = json!("bad/id");
    assert!(parse(v).is_err());
}
#[test]
fn invalid_unselected_duplicate_or_nested_registration_refuses_complete_catalog() {
    for field in [
        "stack",
        "project",
        "working_directory",
        "services",
        "operator",
    ] {
        let mut v = configuration();
        let mut other = v["stacks"][0].clone();
        other["stack"] = json!("other");
        other["project"] = json!("other");
        other["working_directory"] = json!("/opt/stacks/other");
        other[field] = v["stacks"][0][field].clone();
        if field == "services" {
            other[field] = json!(["media", "media"]);
        }
        if field == "operator" {
            other[field]["sha256"] = json!("bad");
        }
        v["stacks"].as_array_mut().unwrap().push(other);
        assert!(parse(v).is_err());
    }
    let mut v = configuration();
    let mut other = v["stacks"][0].clone();
    other["stack"] = json!("other");
    other["project"] = json!("other");
    other["working_directory"] = json!("/opt/stacks/media/child");
    v["stacks"].as_array_mut().unwrap().push(other);
    assert!(parse(v).is_err());
    let mut v = configuration();
    v["stacks"][0]["managed"] = json!({"state":"absent"});
    parse(v).unwrap();
}
#[test]
fn protected_catalog_types_modes_ownership_and_missing_leaf_refuse() {
    for kind in 0..5 {
        let f = Fixture::new();
        fs::remove_file(f.path()).unwrap();
        match kind {
            0 => {}
            1 => symlink("missing", f.path()).unwrap(),
            2 => fs::create_dir(f.path()).unwrap(),
            3 => rustix::fs::mknodat(
                rustix::fs::CWD,
                f.path(),
                rustix::fs::FileType::Fifo,
                Mode::RUSR,
                0,
            )
            .unwrap(),
            _ => {
                f.write(b"{}");
                fs::hard_link(f.path(), f.tree.path().join("alias")).unwrap();
            }
        }
        assert!(f.acquire().is_err());
    }
    for name in ["", "config", "config/catalog.json"] {
        let f = Fixture::new();
        fs::set_permissions(f.tree.path().join(name), fs::Permissions::from_mode(0o722)).unwrap();
        refusal(f.acquire(), ErrorCode::Unavailable);
    }
    let f = Fixture::new();
    let e = f.acquire().unwrap();
    refusal(identity(&e.fd, e.owner + 1), ErrorCode::Unavailable);
    fs::rename(f.tree.path().join("config"), f.tree.path().join("old")).unwrap();
    symlink("old", f.tree.path().join("config")).unwrap();
    assert!(f.acquire().is_err());
}
#[test]
fn raw_whitespace_changes_named_replacement_and_ancestor_changes_poison() {
    for change in 0..3 {
        let f = Fixture::new();
        let e = f.acquire().unwrap();
        match change {
            0 => {
                let mut raw = fs::read(f.path()).unwrap();
                raw.push(b' ');
                f.write(&raw);
            }
            1 => {
                fs::rename(f.path(), f.tree.path().join("old")).unwrap();
                f.write(&serde_json::to_vec(&configuration()).unwrap());
            }
            _ => {
                fs::rename(f.tree.path().join("config"), f.tree.path().join("old")).unwrap();
                fs::create_dir(f.tree.path().join("config")).unwrap();
                f.write(&serde_json::to_vec(&configuration()).unwrap());
            }
        }
        assert!(e.revalidate_local(f.tree.path(), &|| false).is_err());
        refusal(
            e.revalidate_local(f.tree.path(), &|| false),
            ErrorCode::Conflict,
        );
    }
}
#[test]
fn late_catalog_name_or_directory_replacement_after_hashes_refuses() {
    for directory in [false, true] {
        let f = Fixture::new();
        let e = f.acquire().unwrap();
        let before = e.budget.spent.load(Ordering::Relaxed);
        let changed = Cell::new(false);
        let cancelled = || {
            if !changed.get()
                && e.budget.spent.load(Ordering::Relaxed)
                    >= before + 2 * (e.snapshot.identity.size + 1)
            {
                changed.set(true);
                if directory {
                    fs::rename(f.tree.path().join("config"), f.tree.path().join("old")).unwrap();
                    fs::create_dir(f.tree.path().join("config")).unwrap();
                } else {
                    fs::rename(f.path(), f.tree.path().join("old")).unwrap();
                }
                f.write(&serde_json::to_vec(&configuration()).unwrap());
            }
            false
        };
        refusal(
            e.revalidate_local(f.tree.path(), &cancelled),
            ErrorCode::Conflict,
        );
        assert!(changed.get());
    }
}
#[test]
fn exact_catalog_file_and_encoded_report_boundaries_and_unrelated_entries() {
    let f = Fixture::new();
    let mut raw = serde_json::to_vec(&configuration()).unwrap();
    raw.resize(FILE_BYTES as usize, b' ');
    f.write(&raw);
    let e = f.acquire().unwrap();
    assert_eq!(e.snapshot.identity.size, FILE_BYTES);
    fs::write(f.tree.path().join("config/unrelated"), b"allowed").unwrap();
    e.revalidate_local(f.tree.path(), &|| false).unwrap();
    let size = serde_json::to_vec(e.snapshot()).unwrap().len();
    serde_json::to_writer(
        &mut Count {
            bytes: 0,
            max: size,
        },
        e.snapshot(),
    )
    .unwrap();
    assert!(
        serde_json::to_writer(
            &mut Count {
                bytes: 0,
                max: size - 1
            },
            e.snapshot()
        )
        .is_err()
    );
    raw.push(b' ');
    f.write(&raw);
    refusal(f.acquire(), ErrorCode::Unavailable);
}
#[test]
fn original_catalog_age_cancellation_read_budget_reentrancy_and_unwind_poison() {
    let f = Fixture::new();
    let e = f.acquire().unwrap();
    let at = e.snapshot.observed_at;
    refusal(
        e.checked(|| {
            e.validate_from(
                &local_root(f.tree.path()),
                at + 5,
                Instant::now() + WORK,
                &|| false,
            )
        }),
        ErrorCode::Expired,
    );
    refusal(
        e.revalidate_local(f.tree.path(), &|| false),
        ErrorCode::Conflict,
    );
    let e = f.acquire().unwrap();
    refusal(
        e.revalidate_local(f.tree.path(), &|| true),
        ErrorCode::Conflict,
    );
    let e = f.acquire().unwrap();
    e.budget.spent.store(1024 * 1024, Ordering::Relaxed);
    refusal(
        e.revalidate_local(f.tree.path(), &|| false),
        ErrorCode::Unavailable,
    );
    let e = f.acquire().unwrap();
    refusal(
        e.checked(|| {
            refusal(e.checked(|| Ok(())), ErrorCode::Conflict);
            Ok(())
        }),
        ErrorCode::Conflict,
    );
    let e = f.acquire().unwrap();
    assert!(
        catch_unwind(AssertUnwindSafe(
            || e.checked(|| panic!("owned catalog unwind"))
        ))
        .is_err()
    );
    refusal(
        e.revalidate_local(f.tree.path(), &|| false),
        ErrorCode::Conflict,
    );
    let mut e = f.acquire().unwrap();
    e.budget.expires = Instant::now();
    refusal(
        e.revalidate_local(f.tree.path(), &|| false),
        ErrorCode::Expired,
    );
}
#[test]
fn catalog_path_admission_and_maximum_retained_handles() {
    for path in [
        "", "/", "relative", "/a/", "/a//b", "/a/../b", "/a,b", "/a\nb",
    ] {
        assert!(admit(Path::new(path)).is_err());
    }
    let f = Fixture::new();
    let dirs = std::iter::repeat_n("a", 15).collect::<Vec<_>>().join("/");
    fs::create_dir_all(f.tree.path().join(&dirs)).unwrap();
    fs::copy(f.path(), f.tree.path().join(&dirs).join("catalog.json")).unwrap();
    let e = local_catalog(f.tree.path(), Path::new(&format!("/{dirs}/catalog.json"))).unwrap();
    assert_eq!(e.retained_descriptor_count(), 17);
    assert_eq!(e.retained_descriptor_count() + 1, 18);
    assert!(admit(Path::new(&format!("/{dirs}/a/catalog.json"))).is_err());
}
#[test]
fn public_catalog_nonroot_refusal_precedes_callback_and_target_access() {
    assert!(!rustix::process::geteuid().is_root());
    let called = Cell::new(false);
    refusal(
        inspect_management_catalog(
            Path::new("/limeos-unregistered-missing-catalog.json"),
            &|| {
                called.set(true);
                false
            },
        ),
        ErrorCode::Forbidden,
    );
    assert!(!called.get());
}
