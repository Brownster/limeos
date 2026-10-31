#![cfg(feature = "read-worker-probe")]
use limeos_executor::read_launch::{LoadedHealthRole, SealedExecutable};
use std::{
    fs::File,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
struct Image(PathBuf);
impl Image {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "health-image-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("image");
        std::fs::copy(env!("CARGO_BIN_EXE_limeos-stack-read-worker"), &path).unwrap();
        assert!(
            Command::new("/usr/bin/strip")
                .args(["--strip-debug", "--strip-unneeded"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        Self(path)
    }
}
impl Drop for Image {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.0.parent().unwrap()).unwrap();
    }
}
#[test]
fn actual_packaged_worker_roles_reach_genuine_nonroot_health_refusal_with_confirmed_cleanup() {
    assert!(!rustix::process::geteuid().is_root());
    let source = Image::new();
    let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile);
    let output = Command::new(env!("CARGO_BIN_EXE_limeos-read-worker-probe"))
        .arg("health-launch-observer")
        .env_clear()
        .env("LIMEOS_TEST_HEALTH_IMAGE", &source.0)
        .output()
        .unwrap();
    assert_eq!(
        rustix::process::getrlimit(rustix::process::Resource::Nofile),
        limit
    );
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("LIMEOS_HEALTH_REAL_WORKER_REFUSED_CLEANLY soft_nofile=256")
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
}
#[test]
fn direct_on_disk_health_roles_and_unsealed_loader_cannot_collect() {
    assert!(LoadedHealthRole::current().is_err());
    let source = Image::new();
    let image = SealedExecutable::capture(&File::open(&source.0).unwrap()).unwrap();
    let metadata = serde_json::to_value(image.metadata()).unwrap();
    for role in ["health-guardian", "health-worker"] {
        let output = Command::new(&source.0)
            .arg(role)
            .env_clear()
            .env(
                "LIMEOS_READ_IMAGE_DIGEST",
                metadata["sha256"].as_str().unwrap(),
            )
            .env("LIMEOS_HEALTH_NONCE", "a".repeat(64))
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
