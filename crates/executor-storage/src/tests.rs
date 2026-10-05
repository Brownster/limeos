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
