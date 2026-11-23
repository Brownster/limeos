use super::*;
use crate::topology::{Node, Topology};
use std::collections::BTreeSet;

fn number(minor: u32) -> DeviceNumber {
    DeviceNumber { major: 8, minor }
}
fn node(name: &str, parents: &[u32], serial: Option<&str>) -> Node {
    Node {
        name: name.into(),
        parents: parents.iter().map(|p| number(*p)).collect(),
        serial: serial.map(str::to_string),
    }
}
#[test]
fn boot_graph_excludes_siblings_aliases_and_all_mapper_backings() {
    let t = Topology(BTreeMap::from([
        (number(0), node("sda", &[], Some("boot"))),
        (number(1), node("sda1", &[0], None)),
        (number(2), node("sda2", &[0], None)),
        (number(16), node("sdb", &[], Some("data"))),
        (number(17), node("sdb1", &[16], None)),
        (number(32), node("sdc", &[], Some("other"))),
        (number(40), node("dm-0", &[1, 32], None)),
    ]));
    let protected = t.connected(&BTreeSet::from([number(40)])).unwrap();
    assert_eq!(
        protected,
        BTreeSet::from([number(0), number(1), number(2), number(32), number(40)])
    );
    assert!(!protected.contains(&number(17)));
    assert!(t.serial_matches(number(17), "data"));
    assert!(!t.serial_matches(number(17), "boot"));
    assert!(!t.serial_matches(number(40), "boot")); // Multiple backing serials are ambiguous.
    assert!(t.connected(&BTreeSet::from([number(99)])).is_err());
    assert!(t.ancestors(number(40), &mut BTreeSet::new(), 0).is_ok());
    let cycle = Topology(BTreeMap::from([
        (number(0), node("dm-0", &[1], None)),
        (number(1), node("dm-1", &[0], None)),
    ]));
    assert!(cycle.ancestors(number(0), &mut BTreeSet::new(), 0).is_err());
}
#[test]
fn mount_parser_preserves_literal_paths_and_rejects_ambiguous_evidence() {
    let text = "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 8:17 / /mnt/TV\\040Shows rw - ext4 /dev/sdb1 rw\n3 1 8:2 / /boot/firmware ro - vfat /dev/sda2 ro\n";
    let m = mounts::parse(text).unwrap();
    assert_eq!(m[1].path, "/mnt/TV Shows");
    assert_eq!(
        mounts::protected(&m).unwrap(),
        BTreeSet::from([number(1), number(2)])
    );
    assert!(!m[2].writable);
    assert!(mounts::parse(&(text.to_string() + "2 1 8:17 / /mnt/other rw - ext4 x rw\n")).is_err());
    for text in [
        "",
        "1 2 nope / / rw - ext4 x rw",
        "1 2 8:1 / /mnt/\\123 rw - ext4 x rw",
    ] {
        assert!(mounts::parse(text).is_err());
    }
    assert!(
        mounts::protected(&mounts::parse("2 1 8:1 / /mnt/data rw - ext4 x rw").unwrap()).is_err()
    );
}

#[test]
fn namespace_proof_requires_kernel_mount_identity_not_equal_paths_or_devices() {
    let host = mounts::parse("27 0 8:1 / / rw - ext4 /dev/sda1 rw").unwrap();
    let clone = mounts::parse("39 0 8:1 / / rw - ext4 /dev/sda1 rw").unwrap();
    assert_eq!(host_view(&host, &host).unwrap(), 27);
    assert_eq!(
        host_view(&clone, &host).unwrap_err(),
        Failure::WrongNamespace
    );
    assert_eq!(host_view(&[], &host).unwrap_err(), Failure::Unavailable);
    let duplicate =
        mounts::parse("27 0 8:1 / / rw - ext4 /dev/sda1 rw\n28 0 8:2 / / rw - ext4 /dev/sda2 rw")
            .unwrap();
    assert_eq!(
        host_view(&duplicate, &host).unwrap_err(),
        Failure::Unavailable
    );
}
#[test]
fn descriptor_walk_rejects_writable_ancestors_links_and_escaping_paths() {
    // /tmp's writable ancestor rejects even a protected leaf. The privileged
    // VM covers valid protected paths, renamed directories and mounted roots.
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("plan.json");
    std::fs::write(&p, "{}").unwrap();
    assert!(matches!(path::plan(&p), Err(Failure::UnsafePath)));
    for p in [
        "relative",
        "/etc/../etc",
        "/etc//passwd",
        "/etc/./passwd",
        "/etc/",
    ] {
        assert!(matches!(
            path::open(Path::new(p), false, true),
            Err(Failure::InvalidPlan)
        ));
    }
    assert!(path::open(Path::new("/etc/passwd/child"), true, true).is_err());
}
#[test]
fn owned_json_walk_refuses_links_foreign_owners_writable_modes_and_extra_names() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let owner = rustix::process::geteuid().as_raw();
    let anchor = tempfile::tempdir().unwrap();
    let root = anchor.path();
    let mode = |p: &Path, m| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m));
    mode(root, 0o755).unwrap();
    std::fs::create_dir_all(root.join("etc/policy")).unwrap();
    mode(&root.join("etc"), 0o755).unwrap();
    mode(&root.join("etc/policy"), 0o755).unwrap();
    std::fs::write(root.join("etc/policy/c.json"), r#"{"a":1}"#).unwrap();
    mode(&root.join("etc/policy/c.json"), 0o640).unwrap();
    let read = |file: &str, uid| {
        read_owned_json_beneath::<serde_json::Value>(root, Path::new(file), uid)
            .map(|v| v["a"].clone())
    };
    assert_eq!(read("/etc/policy/c.json", owner), Ok(serde_json::json!(1)));
    // Another owner than the expected one, at the leaf or above it.
    assert_eq!(
        read("/etc/policy/c.json", owner + 1),
        Err(Failure::UnsafePath)
    );
    assert_eq!(
        read("/etc/policy/absent.json", owner),
        Err(Failure::NotReady)
    );
    // A symlinked intermediate directory, even to the genuine one.
    symlink("policy", root.join("etc/linked")).unwrap();
    assert_eq!(read("/etc/linked/c.json", owner), Err(Failure::UnsafePath));
    // A symlinked leaf.
    symlink("c.json", root.join("etc/policy/l.json")).unwrap();
    assert_eq!(read("/etc/policy/l.json", owner), Err(Failure::UnsafePath));
    // A second hard link.
    std::fs::hard_link(
        root.join("etc/policy/c.json"),
        root.join("etc/policy/h.json"),
    )
    .unwrap();
    assert_eq!(read("/etc/policy/c.json", owner), Err(Failure::InvalidPlan));
    std::fs::remove_file(root.join("etc/policy/h.json")).unwrap();
    assert_eq!(read("/etc/policy/c.json", owner), Ok(serde_json::json!(1)));
    // A group- or world-writable directory anywhere on the way, or leaf.
    mode(&root.join("etc"), 0o775).unwrap();
    assert_eq!(read("/etc/policy/c.json", owner), Err(Failure::UnsafePath));
    mode(&root.join("etc"), 0o755).unwrap();
    mode(&root.join("etc/policy/c.json"), 0o646).unwrap();
    assert_eq!(read("/etc/policy/c.json", owner), Err(Failure::UnsafePath));
    mode(&root.join("etc/policy/c.json"), 0o644).unwrap();
    mode(root, 0o777).unwrap();
    assert_eq!(read("/etc/policy/c.json", owner), Err(Failure::UnsafePath));
    mode(root, 0o755).unwrap();
    // Not a regular file, or not JSON.
    assert_eq!(read("/etc/policy", owner), Err(Failure::InvalidPlan));
    std::fs::write(root.join("etc/policy/c.json"), b"{").unwrap();
    assert_eq!(read("/etc/policy/c.json", owner), Err(Failure::InvalidPlan));
    // Names that would escape or alias are refused before any open.
    assert_eq!(
        read("/etc/../etc/policy/c.json", owner),
        Err(Failure::InvalidPlan)
    );
    assert_eq!(read("etc/policy/c.json", owner), Err(Failure::InvalidPlan));
}
#[test]
fn filesystem_types_are_explicit_and_multidevice_btrfs_stays_closed() {
    assert!(filesystem_matches(StorageFilesystem::Ext4, "ext4"));
    assert!(!filesystem_matches(
        StorageFilesystem::Ext4,
        "fuse.mergerfs"
    ));
    assert!(!filesystem_matches(StorageFilesystem::Ext4, "xfs"));
    assert!(!filesystem_matches(StorageFilesystem::Btrfs, "btrfs"));
    assert!(!filesystem_matches(StorageFilesystem::Ntfs, "fuseblk"));
}
