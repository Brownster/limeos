#![cfg(feature = "read-worker-probe")]
use limeos_executor::{
    read_launch::{ConfigurationSelection, SealedExecutable},
    read_supervision::{Refusal, Supervisor},
};
use limeos_executor_storage::configuration::ConfigurationPaths;
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

struct ImageFile(PathBuf);
impl ImageFile {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "combined-launch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let file = directory.join("image");
        std::fs::copy(env!("CARGO_BIN_EXE_limeos-combined-read-probe"), &file).unwrap();
        assert!(
            Command::new("/usr/bin/strip")
                .args(["--strip-debug", "--strip-unneeded"])
                .arg(&file)
                .status()
                .unwrap()
                .success()
        );
        Self(file)
    }
}
impl Drop for ImageFile {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.0.parent().unwrap()).unwrap();
    }
}
fn selection() -> ConfigurationSelection {
    ConfigurationSelection::new(ConfigurationPaths {
        contract: Path::new("/not-opened/contract.json"),
        pools: Path::new("/not-opened/pools.json"),
        protection: Path::new("/not-opened/protection.json"),
        shares: Path::new("/not-opened/shares.json"),
    })
    .unwrap()
}
#[test]
fn sealed_guardian_and_worker_reach_genuine_nonroot_collection_refusal() {
    assert!(!rustix::process::geteuid().is_root());
    let source = ImageFile::new();
    let image = SealedExecutable::capture(&File::open(&source.0).unwrap()).unwrap();
    let manager = Supervisor::default();
    let ticket = image.start(&manager, selection()).unwrap();
    drop(image); // Monitor owns the executable lease through cleanup.
    assert!(matches!(ticket.wait(), Err(Refusal::WorkerRefused)));
}
#[test]
fn on_disk_internal_roles_refuse_before_any_collection() {
    for role in ["guardian", "worker"] {
        let output = Command::new(env!("CARGO_BIN_EXE_limeos-combined-read-probe"))
            .arg(role)
            .env_clear()
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn default_contract_has_one_final_event_without_host_io() {
    let output = Command::new(env!("CARGO_BIN_EXE_limeos-combined-read-probe"))
        .env_clear()
        .output()
        .unwrap();
    assert!(output.status.success());
    let lines = String::from_utf8(output.stdout).unwrap();
    assert_eq!(lines.lines().count(), 1);
    let json = lines
        .trim_end()
        .strip_prefix("LIMEOS_RW040_COMBINED_JSON=")
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["phase"], "final");
}
#[test]
fn invalid_probe_selection_refuses_before_protected_image_read() {
    let output = Command::new(env!("CARGO_BIN_EXE_limeos-combined-read-probe"))
        .arg("probe")
        .env_clear()
        .env("LIMEOS_READ_CONTRACT", "relative")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
