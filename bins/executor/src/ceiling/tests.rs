use super::*;
use limeos_contracts::VERSION;
use limeos_domain::{ContainerAction, LogOptions};
use limeos_executor_protocol::Request;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    sync::atomic::{AtomicUsize, Ordering},
};

const FILE: &str = "/etc/limeos/system-policy/container.json";
const CORE: u32 = 1001;

/// A directory tree owned by the test user, standing in for `/`; removed
/// on drop.
struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Self(std::env::temp_dir().join(format!(
            "executor-ceiling-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        std::fs::create_dir(&root.0).unwrap();
        root.directory("");
        root.directory("etc");
        root.directory("etc/limeos");
        root.directory("etc/limeos/system-policy");
        root
    }
    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
    fn directory(&self, relative: &str) {
        let path = self.path(relative);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    /// Writes `body` as an operator's editor does: a new file renamed over
    /// the old one.
    fn write(&self, relative: &str, body: &str) {
        let path = self.path(relative);
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, body).unwrap();
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::rename(&temporary, &path).unwrap();
    }
    fn file(&self, role: Role) -> CeilingFile {
        CeilingFile::beneath(
            &self.0,
            Path::new(FILE),
            rustix::process::geteuid().as_raw(),
            role,
        )
    }
}
const CONTAINER: Role = Role {
    kind: Kind::Container,
    read_only: false,
};
fn id(c: char) -> String {
    c.to_string().repeat(64)
}
fn container_ceiling_json(core_uid: u32, containers: &[&String]) -> String {
    serde_json::json!({
        "version": 1,
        "core_uid": core_uid,
        "allow_health": true,
        "allow_container_read": true,
        "allow_container_logs": true,
        "allow_restart": true,
        "managed_containers": containers,
    })
    .to_string()
}
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

    root.write(&FILE[1..], "{");
    assert_eq!(file.load().err(), Some(Refusal::Malformed));
    assert_eq!(file.for_peer(CORE).err(), Some(Error(ErrorCode::Forbidden)));

    std::fs::remove_file(root.path(&FILE[1..])).unwrap();
    assert_eq!(file.load().err(), Some(Refusal::Missing));
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
    assert_eq!(file.load().err(), Some(Refusal::UnsafePath));

    // Fixed again: accepted on the next request.
    root.write(&FILE[1..], &container_ceiling_json(CORE, &[&a]));
    assert!(restart(&file.for_peer(CORE).unwrap(), &a).is_ok());
}

#[test]
fn a_ceiling_naming_root_as_core_is_refused() {
    let root = Root::new();
    root.write(&FILE[1..], &container_ceiling_json(0, &[&id('a')]));
    let file = root.file(CONTAINER);
    assert_eq!(file.load().err(), Some(Refusal::RootCore));
    assert!(Refusal::RootCore.message().contains("core_uid 0"));
    // Not even a root peer passes.
    assert_eq!(file.for_peer(0).err(), Some(Error(ErrorCode::Forbidden)));
    // Every role refuses it.
    for kind in [Kind::Host, Kind::StorageReader] {
        root.write(
            &FILE[1..],
            r#"{"version":1,"core_uid":0,"allow_health":true,"allow_host_read":true}"#,
        );
        let role = Role {
            kind,
            read_only: false,
        };
        assert_eq!(root.file(role).load().err(), Some(Refusal::RootCore));
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
    assert_eq!(file.load().err(), Some(Refusal::UnsafePath));
    assert_eq!(file.for_peer(CORE).err(), Some(Error(ErrorCode::Forbidden)));

    // A real directory whose ceiling is itself a symlink.
    std::fs::remove_file(root.path("etc/limeos/system-policy")).unwrap();
    root.directory("etc/limeos/system-policy");
    symlink("../real-policy/container.json", root.path(&FILE[1..])).unwrap();
    assert_eq!(file.load().err(), Some(Refusal::UnsafePath));

    // A symlink higher up, at `etc`.
    std::fs::remove_file(root.path(&FILE[1..])).unwrap();
    root.write(&FILE[1..], &container_ceiling_json(CORE, &[&a]));
    assert!(file.load().is_ok());
    std::fs::rename(root.path("etc"), root.path("etc-real")).unwrap();
    symlink("etc-real", root.path("etc")).unwrap();
    assert_eq!(file.load().err(), Some(Refusal::UnsafePath));
}

#[test]
fn each_role_refuses_a_ceiling_meant_for_another() {
    let parse = |json: &str| serde_json::from_str::<Ceiling>(json).unwrap();
    let role = |kind, read_only| Role { kind, read_only };
    let writer = parse(&container_ceiling_json(CORE, &[&id('a')]));
    assert_eq!(admit(role(Kind::Container, false), &writer), Ok(()));
    assert_eq!(
        admit(role(Kind::Container, true), &writer),
        Err(Refusal::WritesOnReadOnlyRole)
    );
    assert_eq!(
        admit(role(Kind::Host, false), &writer),
        Err(Refusal::WritesOnReadOnlyRole)
    );
    let host = parse(r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_host_read":true}"#);
    assert_eq!(admit(role(Kind::Host, false), &host), Ok(()));
    assert_eq!(
        admit(role(Kind::StorageReader, false), &host),
        Err(Refusal::ReaderWithoutReads)
    );
    let reader =
        parse(r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_stack_read":true}"#);
    assert_eq!(admit(role(Kind::StorageReader, false), &reader), Ok(()));
    for kind in [Kind::Host, Kind::Container] {
        assert_eq!(
            admit(role(kind, false), &reader),
            Err(Refusal::ReaderFlagsElsewhere)
        );
    }
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
