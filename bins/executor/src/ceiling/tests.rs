use super::*;
use crate::test_root::{FILE, Root, container_ceiling_json, id};
use limeos_contracts::VERSION;
use limeos_domain::{ContainerAction, LogOptions};
use limeos_executor_protocol::Request;
use std::os::unix::fs::{PermissionsExt, symlink};

const CORE: u32 = 1001;

const CONTAINER: CeilingRole = CeilingRole::Container;
fn logs(id: &str) -> Request {
    Request::ContainerLogs {
        version: VERSION,
        resource: format!("container:{id}"),
        options: LogOptions { tail: 10 },
    }
}
fn restart(ceiling: &Ceiling, id: &str) -> Result<(), Error> {
    container_ceiling(ceiling).authorize(CORE, &format!("container:{id}"), ContainerAction::Restart)
}

#[test]
fn revoking_a_container_takes_effect_on_the_next_request_without_a_restart() {
    let root = Root::new();
    let (a, b) = (id('a'), id('b'));
    root.write(&FILE[1..], &container_ceiling_json(CORE, &[&a, &b]));
    // One file object, as the daemon holds for its whole life.
    let file = root.file(CONTAINER);
    let first = file.for_peer(CORE).unwrap();
    assert!(first.validate(CORE, &logs(&a)).is_ok());
    assert!(restart(&first, &a).is_ok());

    // The operator removes `a`; the next request sees it.
    root.write(&FILE[1..], &container_ceiling_json(CORE, &[&b]));
    let next = file.for_peer(CORE).unwrap();
    assert_eq!(
        next.validate(CORE, &logs(&a)).err(),
        Some(Error(ErrorCode::Forbidden))
    );
    assert_eq!(restart(&next, &a), Err(Error(ErrorCode::Forbidden)));
    assert!(next.validate(CORE, &logs(&b)).is_ok());
    assert!(restart(&next, &b).is_ok());

    // An in-place edit (same inode) is seen as well.
    std::fs::write(root.path(&FILE[1..]), container_ceiling_json(CORE, &[])).unwrap();
    let emptied = file.for_peer(CORE).unwrap();
    assert_eq!(restart(&emptied, &b), Err(Error(ErrorCode::Forbidden)));

    // And a changed core UID refuses the old peer before any frame.
    root.write(&FILE[1..], &container_ceiling_json(CORE + 1, &[&b]));
    assert_eq!(file.for_peer(CORE).err(), Some(Error(ErrorCode::Forbidden)));
    assert!(file.for_peer(CORE + 1).is_ok());
}

#[test]
fn a_ceiling_that_cannot_be_read_refuses_the_request_and_nothing_is_kept() {
    let root = Root::new();
    let a = id('a');
    root.write(&FILE[1..], &container_ceiling_json(CORE, &[&a]));
    let file = root.file(CONTAINER);
    assert!(file.load().is_ok());

    // Half-written, as an in-place edit leaves it for a moment: refused as
    // unavailable, never as the ceiling's "no".
    root.write(&FILE[1..], "{");
    assert_eq!(
        file.load().err(),
        Some(Refusal::Read(ProtectedRead::Malformed))
    );
    assert_eq!(
        file.for_peer(CORE).err(),
        Some(Error(ErrorCode::Unavailable))
    );

    std::fs::remove_file(root.path(&FILE[1..])).unwrap();
    assert_eq!(
        file.load().err(),
        Some(Refusal::Read(ProtectedRead::Missing))
    );
    assert_eq!(
        file.for_peer(CORE).err(),
        Some(Error(ErrorCode::Unavailable))
    );

    // Writable by its group: refused, not read.
    root.write(&FILE[1..], &container_ceiling_json(CORE, &[&a]));
    std::fs::set_permissions(
        root.path(&FILE[1..]),
        std::fs::Permissions::from_mode(0o664),
    )
    .unwrap();
    assert_eq!(
        file.load().err(),
        Some(Refusal::Read(ProtectedRead::UnsafePath))
    );
    assert_eq!(
        file.for_peer(CORE).err(),
        Some(Error(ErrorCode::Unavailable))
    );

    // Fixed again: accepted on the next request.
    root.write(&FILE[1..], &container_ceiling_json(CORE, &[&a]));
    assert!(restart(&file.for_peer(CORE).unwrap(), &a).is_ok());
}

#[test]
fn a_ceiling_naming_root_as_core_is_refused() {
    let root = Root::new();
    root.write(&FILE[1..], &container_ceiling_json(0, &[&id('a')]));
    let file = root.file(CONTAINER);
    assert_eq!(
        file.load().err(),
        Some(Refusal::Content(CeilingRefusal::RootCore))
    );
    assert!(CeilingRefusal::RootCore.message().contains("core_uid 0"));
    // Not even a root peer passes.
    assert_eq!(file.for_peer(0).err(), Some(Error(ErrorCode::Unavailable)));
    // Every role refuses it.
    root.write(
        &FILE[1..],
        r#"{"version":1,"core_uid":0,"allow_health":true,"allow_host_read":true}"#,
    );
    for role in [CeilingRole::Host, CeilingRole::StorageReader] {
        assert_eq!(
            root.file(role).load().err(),
            Some(Refusal::Content(CeilingRefusal::RootCore))
        );
    }
}

#[test]
fn a_symlinked_directory_or_leaf_on_the_ceiling_path_is_refused() {
    let root = Root::new();
    let a = id('a');
    // The genuine policy directory moved aside and linked back.
    std::fs::rename(
        root.path("etc/limeos/system-policy"),
        root.path("etc/limeos/real-policy"),
    )
    .unwrap();
    root.write(
        "etc/limeos/real-policy/container.json",
        &container_ceiling_json(CORE, &[&a]),
    );
    symlink("real-policy", root.path("etc/limeos/system-policy")).unwrap();
    let file = root.file(CONTAINER);
    assert_eq!(
        file.load().err(),
        Some(Refusal::Read(ProtectedRead::UnsafePath))
    );
    assert_eq!(
        file.for_peer(CORE).err(),
        Some(Error(ErrorCode::Unavailable))
    );

    // A real directory whose ceiling is itself a symlink.
    std::fs::remove_file(root.path("etc/limeos/system-policy")).unwrap();
    root.directory("etc/limeos/system-policy");
    symlink("../real-policy/container.json", root.path(&FILE[1..])).unwrap();
    assert_eq!(
        file.load().err(),
        Some(Refusal::Read(ProtectedRead::UnsafePath))
    );

    // A symlink higher up, at `etc`.
    std::fs::remove_file(root.path(&FILE[1..])).unwrap();
    root.write(&FILE[1..], &container_ceiling_json(CORE, &[&a]));
    assert!(file.load().is_ok());
    std::fs::rename(root.path("etc"), root.path("etc-real")).unwrap();
    symlink("etc-real", root.path("etc")).unwrap();
    assert_eq!(
        file.load().err(),
        Some(Refusal::Read(ProtectedRead::UnsafePath))
    );
}

/// What a per-request reread costs, at the installed path's depth and with
/// the largest ceiling the format allows (64 container IDs, 16 stacks).
/// Run by hand, on the target too:
/// `cargo test -p limeos-executor --lib -- --ignored --nocapture ceiling_read_cost`
#[test]
#[ignore = "timing; run by hand"]
fn ceiling_read_cost() {
    let root = Root::new();
    let containers: Vec<String> = (0..64).map(|i| format!("{i:064x}")).collect();
    let mut json: serde_json::Value = serde_json::from_str(&container_ceiling_json(
        CORE,
        &containers.iter().collect::<Vec<_>>(),
    ))
    .unwrap();
    json["allow_stack"] = true.into();
    json["managed_stacks"] = (0..16).map(|i| format!("stack{i}")).collect();
    root.write(&FILE[1..], &json.to_string());
    let file = root.file(CONTAINER);
    assert!(file.load().is_ok());
    let rounds = 10_000;
    let started = std::time::Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(file.for_peer(CORE).unwrap());
    }
    let each = started.elapsed() / rounds;
    println!(
        "ceiling read: {each:?} per request ({} bytes)",
        json.to_string().len()
    );
}
