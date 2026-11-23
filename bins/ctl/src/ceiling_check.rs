//! `limeosctl check-ceiling FILE CORE_UID [ROLE]`: the checks an executor
//! makes on every read of its shared ceiling, run by the operator before an
//! edit goes live. Executors reread the file per request, so a file this
//! refuses makes a running executor answer every request `Unavailable`,
//! with nothing failing visibly; the next restart or package configure then
//! fails. The walk is `limeos_executor_storage::read_root_owned_json` and
//! the role check is `Ceiling::admit`, the same code the executors run.
use limeos_executor_protocol::{Ceiling, CeilingRole};
use std::path::Path;

pub const USAGE: &str = "check-ceiling FILE CORE_UID [host|container|container-read-only|storage]: check an executor ceiling as its executor would (every path component root-owned, not group- or world-writable, no symlink; a single-link regular file of at most 64 KiB; core_uid not 0 and equal to CORE_UID; flags that suit the role). The role may be left out only for the installed paths under /etc/limeos/system-policy or /etc/limeos-shadow/system-policy (the shadow container.json is container-read-only).";

/// The role named on the command line, or implied by an installed path.
/// Only the packages' own paths imply a role; anything else (a proposal in
/// `/root`, a copy, a relative path) must name it, so a file is never
/// checked for a role its executor does not run.
pub fn role(file: &str, named: Option<&str>) -> Result<CeilingRole, String> {
    if let Some(name) = named {
        return CeilingRole::parse(name).ok_or_else(|| {
            format!("unknown executor role {name:?}; use host, container, container-read-only or storage")
        });
    }
    Ok(match file {
        "/etc/limeos/system-policy/host.json" | "/etc/limeos-shadow/system-policy/host.json" => {
            CeilingRole::Host
        }
        "/etc/limeos/system-policy/container.json" => CeilingRole::Container,
        // The shadow package's container executor runs read-only.
        "/etc/limeos-shadow/system-policy/container.json" => CeilingRole::ContainerReadOnly,
        "/etc/limeos/system-policy/storage.json"
        | "/etc/limeos-shadow/system-policy/storage.json" => CeilingRole::StorageReader,
        _ => {
            return Err(
                "name the executor role: limeosctl check-ceiling FILE CORE_UID host|container|container-read-only|storage"
                    .into(),
            );
        }
    })
}

/// Reads `file` as the executors do (walked from `/`, every component
/// root-owned) and checks it for `role` and `core_uid`.
pub fn check_file(file: &Path, core_uid: u32, role: CeilingRole) -> Result<(), String> {
    check_file_beneath(Path::new("/"), file, 0, core_uid, role)
}

/// [`check_file`] walked from `anchor` with `owner` in place of root; only
/// tests pass anything else.
fn check_file_beneath(
    anchor: &Path,
    file: &Path,
    owner: u32,
    core_uid: u32,
    role: CeilingRole,
) -> Result<(), String> {
    let ceiling: Ceiling = limeos_executor_storage::read_owned_json_beneath(anchor, file, owner)
        .map_err(|refusal| {
            format!(
                "Unsafe or unreadable executor ceiling: {refusal}. Preserve and repair the original file."
            )
        })?;
    check(&ceiling, core_uid, role)
}

/// A parsed ceiling, for `role` and `core_uid`.
pub fn check(ceiling: &Ceiling, core_uid: u32, role: CeilingRole) -> Result<(), String> {
    ceiling.admit(role).map_err(|refusal| {
        format!(
            "Executor ceiling refused for the {role:?} role: {refusal}. Repair the original file."
        )
    })?;
    if ceiling.core_uid != core_uid {
        return Err(format!(
            "Executor ceiling names core_uid {}, but core runs as {core_uid}. Repair the original file.",
            ceiling.core_uid
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn owner() -> u32 {
        rustix::process::geteuid().as_raw()
    }
    struct Tree(tempfile::TempDir);
    impl Tree {
        fn new() -> Self {
            let tree = Self(tempfile::tempdir().unwrap());
            for directory in ["", "etc", "etc/policy"] {
                let path = tree.0.path().join(directory);
                std::fs::create_dir_all(&path).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            tree
        }
        fn write(&self, name: &str, body: &str, mode: u32) {
            let path = self.0.path().join("etc/policy").join(name);
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        fn check(&self, file: &str, uid: u32, role: CeilingRole) -> Result<(), String> {
            check_file_beneath(self.0.path(), Path::new(file), owner(), uid, role)
        }
    }
    const CONTAINER: &str = r#"{"version":1,"core_uid":1001,"allow_health":true,"allow_container_read":true,"allow_restart":true,"managed_containers":[]}"#;

    #[test]
    fn it_accepts_what_the_container_executor_would_and_refuses_the_rest() {
        let tree = Tree::new();
        tree.write("container.json", CONTAINER, 0o644);
        let file = "/etc/policy/container.json";
        assert_eq!(tree.check(file, 1001, CeilingRole::Container), Ok(()));
        // Another core UID, or a role it does not suit.
        assert!(
            tree.check(file, 1002, CeilingRole::Container)
                .unwrap_err()
                .contains("core_uid 1001")
        );
        let refused = tree
            .check(file, 1001, CeilingRole::ContainerReadOnly)
            .unwrap_err();
        assert!(refused.contains("cannot mutate"), "{refused}");
        assert!(tree.check(file, 1001, CeilingRole::Host).is_err());
        assert!(tree.check(file, 1001, CeilingRole::StorageReader).is_err());
        // core_uid 0, even when the caller names 0.
        tree.write("container.json", &CONTAINER.replace("1001", "0"), 0o644);
        assert!(
            tree.check(file, 0, CeilingRole::Container)
                .unwrap_err()
                .contains("core_uid 0")
        );
        // Unsafe modes, links and paths: the executor's walk.
        tree.write("container.json", CONTAINER, 0o664);
        assert!(
            tree.check(file, 1001, CeilingRole::Container)
                .unwrap_err()
                .contains("Unsafe")
        );
        tree.write("container.json", CONTAINER, 0o644);
        symlink("policy", tree.0.path().join("etc/linked")).unwrap();
        assert!(
            tree.check("/etc/linked/container.json", 1001, CeilingRole::Container)
                .is_err()
        );
        symlink(
            "container.json",
            tree.0.path().join("etc/policy/alias.json"),
        )
        .unwrap();
        assert!(
            tree.check("/etc/policy/alias.json", 1001, CeilingRole::Container)
                .is_err()
        );
        std::fs::hard_link(
            tree.0.path().join("etc/policy/container.json"),
            tree.0.path().join("etc/policy/second.json"),
        )
        .unwrap();
        assert!(tree.check(file, 1001, CeilingRole::Container).is_err());
        std::fs::remove_file(tree.0.path().join("etc/policy/second.json")).unwrap();
        tree.write("big.json", &" ".repeat(65537), 0o644);
        assert!(
            tree.check("/etc/policy/big.json", 1001, CeilingRole::Container)
                .is_err()
        );
        assert!(
            tree.check("/etc/policy/absent.json", 1001, CeilingRole::Container)
                .unwrap_err()
                .contains("does not exist")
        );
        std::fs::set_permissions(
            tree.0.path().join("etc"),
            std::fs::Permissions::from_mode(0o775),
        )
        .unwrap();
        assert!(tree.check(file, 1001, CeilingRole::Container).is_err());
    }

    #[test]
    fn the_role_is_named_or_implied_only_by_an_installed_path() {
        for (path, implied) in [
            ("/etc/limeos/system-policy/host.json", CeilingRole::Host),
            (
                "/etc/limeos/system-policy/container.json",
                CeilingRole::Container,
            ),
            (
                "/etc/limeos/system-policy/storage.json",
                CeilingRole::StorageReader,
            ),
            (
                "/etc/limeos-shadow/system-policy/host.json",
                CeilingRole::Host,
            ),
            (
                "/etc/limeos-shadow/system-policy/container.json",
                CeilingRole::ContainerReadOnly,
            ),
            (
                "/etc/limeos-shadow/system-policy/storage.json",
                CeilingRole::StorageReader,
            ),
        ] {
            assert_eq!(role(path, None), Ok(implied), "{path}");
        }
        // Anything else asks rather than guesses from the file name.
        for path in [
            "/root/container-proposed.json",
            "/root/container.json",
            "container.json",
            "/etc/limeos/system-policy/../system-policy/container.json",
            "/etc/limeos-shadow/container.json",
        ] {
            assert!(
                role(path, None)
                    .unwrap_err()
                    .contains("name the executor role"),
                "{path}"
            );
        }
        assert_eq!(
            role("/root/container.json", Some("container-read-only")),
            Ok(CeilingRole::ContainerReadOnly)
        );
        assert!(role("/x/host.json", Some("storage-reader")).is_err());
    }

    #[test]
    fn a_write_enabled_ceiling_fails_the_shadow_check_its_executor_would_fail() {
        let tree = Tree::new();
        tree.write("container.json", CONTAINER, 0o644);
        let shadow = role("/etc/limeos-shadow/system-policy/container.json", None).unwrap();
        let refused = tree
            .check("/etc/policy/container.json", 1001, shadow)
            .unwrap_err();
        assert!(refused.contains("cannot mutate"), "{refused}");
    }

    #[test]
    fn the_installed_tree_is_walked_from_root() {
        // From `/` as root, a file under a user's directory is refused: the
        // same answer the executor would give it.
        let tree = Tree::new();
        tree.write("container.json", CONTAINER, 0o644);
        let file = tree.0.path().join("etc/policy/container.json");
        if owner() != 0 {
            assert!(check_file(&file, 1001, CeilingRole::Container).is_err());
        }
    }
}
