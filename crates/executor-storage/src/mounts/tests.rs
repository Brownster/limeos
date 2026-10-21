use super::*;

#[test]
fn namespace_spelling_is_metadata_without_a_filesystem_path() {
    for kind in [
        "net",
        "mnt",
        "uts",
        "ipc",
        "pid",
        "pid_for_children",
        "user",
        "cgroup",
        "time",
        "time_for_children",
    ] {
        let root = format!("{kind}:[4026532191]");
        assert_eq!(
            root_kind(&root, "nsfs", "/run/namespace").unwrap(),
            MountRoot::Namespace {
                kind,
                inode: 4026532191
            }
        );
    }
    assert_eq!(
        root_kind("/subtree", "ext4", "/data").unwrap(),
        MountRoot::Path("/subtree")
    );
}

#[test]
fn malformed_or_mislabelled_namespace_names_refuse() {
    for root in [
        "net:[0]",
        "net:[01]",
        "net:[-1]",
        "net:[+1]",
        "net:[]",
        "net:[18446744073709551616]",
        "other:[1]",
        "NET:[1]",
        "net:[1]]",
        "net:[1]tail",
        "/",
        "net:[ 1]",
        "net:[1/2]",
    ] {
        assert!(root_kind(root, "nsfs", "/run/namespace").is_err(), "{root}");
    }
    assert!(root_kind("net:[1]", "ext4", "/run/namespace").is_err());
    assert!(root_kind("relative", "ext4", "/data").is_err());
    assert!(root_kind("/", "ext4", "relative").is_err());
    assert!(root_kind("net:[1]", "nsfs", "/").is_err());
}

#[test]
fn host_profiles_keep_namespace_rows_and_refuse_ambiguous_roots_or_ids() {
    let base = "27 1 8:1 / / rw - ext4 /dev/vda1 rw\n";
    let namespace =
        "293 26 0:4 net:[4026532191] /run/limeos-nsfs-probe rw shared:196 - nsfs nsfs rw\n";
    for profile in [StorageReadProfile::Standard, StorageReadProfile::CombinedV1] {
        let parsed = parse_with_profile(&format!("{base}{namespace}"), profile).unwrap();
        assert_eq!(
            parsed[1].root_kind().unwrap(),
            MountRoot::Namespace {
                kind: "net",
                inode: 4026532191
            }
        );
        for bad in [
            namespace.replace("/run/limeos-nsfs-probe", "/"),
            namespace.replace("net:[4026532191]", "net:[0]"),
            namespace.replace(" - nsfs ", " - ext4 "),
            format!("{namespace}{namespace}"),
        ] {
            assert!(parse_with_profile(&format!("{base}{bad}"), profile).is_err());
        }
    }
}
