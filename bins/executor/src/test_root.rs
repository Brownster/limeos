//! A directory tree owned by the test user, standing in for `/`, for tests
//! of code that walks root-owned paths.
use crate::ceiling::CeilingFile;
use limeos_executor_protocol::CeilingRole;
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

pub(crate) const FILE: &str = "/etc/limeos/system-policy/container.json";

/// Removed on drop.
pub(crate) struct Root(pub(crate) PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Root {
    pub(crate) fn new() -> Self {
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
    pub(crate) fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
    pub(crate) fn directory(&self, relative: &str) {
        let path = self.path(relative);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    /// Writes `body` as an operator's editor does: a new file renamed over
    /// the old one.
    pub(crate) fn write(&self, relative: &str, body: &str) {
        let path = self.path(relative);
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, body).unwrap();
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::rename(&temporary, &path).unwrap();
    }
    pub(crate) fn file(&self, role: CeilingRole) -> CeilingFile {
        CeilingFile::beneath(
            &self.0,
            Path::new(FILE),
            rustix::process::geteuid().as_raw(),
            role,
        )
    }
}
pub(crate) fn id(c: char) -> String {
    c.to_string().repeat(64)
}
pub(crate) fn container_ceiling_json(core_uid: u32, containers: &[&String]) -> String {
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
