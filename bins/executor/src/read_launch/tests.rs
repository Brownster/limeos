use super::*;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "read-launch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn file(&self) -> std::path::PathBuf {
        self.0.join("image")
    }
    fn elf(&self) {
        std::fs::copy("/usr/bin/true", self.file()).unwrap();
        std::fs::set_permissions(self.file(), std::fs::Permissions::from_mode(0o500)).unwrap();
    }
    fn root(&self) -> File {
        File::from(
            rustix::fs::open(
                &self.0,
                OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap(),
        )
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn paths() -> ConfigurationPaths<'static> {
    ConfigurationPaths {
        contract: Path::new("/not-opened/contract.json"),
        pools: Path::new("/not-opened/pools.json"),
        protection: Path::new("/not-opened/protection.json"),
        shares: Path::new("/not-opened/shares.json"),
    }
}

#[test]
fn selection_refuses_noncanonical_amplified_duplicate_and_missing_role_names() {
    assert!(ConfigurationSelection::new(paths()).is_ok());
    for path in [
        "relative", "/", "/a/", "/a//b", "/a/./b", "/a/../b", "/a\n/b",
    ] {
        let mut p = paths();
        p.contract = Path::new(path);
        assert!(matches!(
            ConfigurationSelection::new(p),
            Err(Refusal::InvalidSelection)
        ));
    }
    for path in [
        format!("/{}", "x".repeat(512)),
        format!("/{}", "x/".repeat(16) + "x"),
    ] {
        let mut p = paths();
        p.contract = Path::new(&path);
        assert!(matches!(
            ConfigurationSelection::new(p),
            Err(Refusal::InvalidSelection)
        ));
    }
    let mut p = paths();
    p.shares = p.contract;
    assert!(matches!(
        ConfigurationSelection::new(p),
        Err(Refusal::InvalidSelection)
    ));
}
#[test]
fn sealed_bytes_survive_source_mutation_and_name_replacement() {
    let scratch = Scratch::new();
    scratch.elf();
    let source = File::open(scratch.file()).unwrap();
    let image = SealedExecutable::capture(&source).unwrap();
    let digest = image.metadata.sha256.clone();
    std::fs::set_permissions(scratch.file(), std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(scratch.file(), b"changed source").unwrap();
    std::fs::rename(scratch.file(), scratch.0.join("old")).unwrap();
    std::fs::write(scratch.file(), b"replacement").unwrap();
    let mut bytes = vec![0; image.metadata.bytes as usize];
    image.file.read_exact_at(&mut bytes, 0).unwrap();
    assert_eq!(hex(&Sha256::digest(&bytes)), digest);
    let mut command = Command::new(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        image.file.as_raw_fd()
    ));
    command.env_clear();
    assert!(command.status().unwrap().success());
}
#[test]
fn all_content_seals_are_active_and_cannot_be_removed() {
    let image = SealedExecutable::capture(&File::open("/usr/bin/true").unwrap()).unwrap();
    assert!(
        rustix::fs::fcntl_get_seals(&image.file)
            .unwrap()
            .contains(seals())
    );
    assert!(image.file.write_at(b"x", 0).is_err());
    assert!(image.file.set_len(1).is_err());
    assert!(image.file.set_len(image.metadata.bytes + 1).is_err());
    assert!(rustix::fs::fcntl_add_seals(&image.file, SealFlags::FUTURE_WRITE).is_err());
}
#[test]
fn malformed_architecture_scripts_and_oversized_inputs_refuse() {
    let scratch = Scratch::new();
    std::fs::write(scratch.file(), b"#!/bin/sh\nexit 0\n").unwrap();
    assert!(matches!(
        SealedExecutable::capture(&File::open(scratch.file()).unwrap()),
        Err(Refusal::ImageLimit)
    ));
    let mut header = [0; 64];
    header[..4].copy_from_slice(b"\x7fELF");
    header[4] = 2;
    header[5] = 1;
    header[6] = 1;
    header[16] = 3;
    header[18] = if cfg!(target_arch = "x86_64") {
        183
    } else {
        62
    };
    header[52] = 64;
    assert_eq!(elf(&header), Err(Refusal::ImageFormat));
    let file = File::create(scratch.file()).unwrap();
    file.set_len(MAX_IMAGE + 1).unwrap();
    assert!(matches!(
        SealedExecutable::capture(&File::open(scratch.file()).unwrap()),
        Err(Refusal::ImageLimit)
    ));
}
#[test]
fn private_walk_fixture_preserves_root_policy_without_public_owner_override() {
    let scratch = Scratch::new();
    scratch.elf();
    let owner = rustix::process::geteuid().as_raw();
    assert!(protected_image(scratch.root(), "image", owner).is_ok());
    assert!(matches!(
        protected_image(scratch.root(), "image", owner + 1),
        Err(Refusal::UntrustedImage)
    ));
    std::fs::set_permissions(scratch.file(), std::fs::Permissions::from_mode(0o722)).unwrap();
    assert!(matches!(
        protected_image(scratch.root(), "image", owner),
        Err(Refusal::UntrustedImage)
    ));
    std::fs::set_permissions(scratch.file(), std::fs::Permissions::from_mode(0o500)).unwrap();
    std::fs::hard_link(scratch.file(), scratch.0.join("alias")).unwrap();
    assert!(matches!(
        protected_image(scratch.root(), "image", owner),
        Err(Refusal::UntrustedImage)
    ));
}
#[test]
fn protected_walk_rejects_symlink_leaf_ancestor_directory_and_unprotected_root() {
    let scratch = Scratch::new();
    scratch.elf();
    let owner = rustix::process::geteuid().as_raw();
    symlink("image", scratch.0.join("link")).unwrap();
    assert!(matches!(
        protected_image(scratch.root(), "link", owner),
        Err(Refusal::UntrustedImage)
    ));
    symlink(".", scratch.0.join("parent")).unwrap();
    assert!(matches!(
        protected_image(scratch.root(), "parent/image", owner),
        Err(Refusal::UntrustedImage)
    ));
    std::fs::create_dir(scratch.0.join("directory")).unwrap();
    assert!(matches!(
        protected_image(scratch.root(), "directory", owner),
        Err(Refusal::UntrustedImage)
    ));
    std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        protected_image(scratch.root(), "image", owner),
        Err(Refusal::UntrustedImage)
    ));
}
#[test]
fn directory_revalidation_uses_the_original_permissions_even_if_still_protected() {
    let scratch = Scratch::new();
    let root = scratch.root();
    let before = root.metadata().unwrap();
    let owner = rustix::process::geteuid().as_raw();
    assert!(revalidate_directory(&root, &before, owner).is_ok());
    std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o500)).unwrap();
    assert_eq!(
        revalidate_directory(&root, &before, owner),
        Err(Refusal::Changed)
    );
    std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o700)).unwrap();
}
#[test]
fn launch_environment_is_closed_and_expired_preparation_cannot_restart() {
    let image = SealedExecutable::capture(&File::open("/usr/bin/true").unwrap()).unwrap();
    let selection = ConfigurationSelection::new(paths()).unwrap();
    let command = image.command("guardian", &selection);
    assert_eq!(command.get_envs().count(), 5);
    assert!(command.get_envs().all(|(key, value)| value.is_some()
        && (KEYS.iter().any(|expected| key == *expected) || key == DIGEST_KEY)));
    let launch = ProtectedLaunch {
        image,
        selection,
        deadline: Instant::now() - Duration::from_nanos(1),
    };
    assert!(matches!(
        launch.start(&Supervisor::default()),
        Err(crate::read_supervision::Refusal::Deadline)
    ));
}
