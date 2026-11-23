//! The shared ceiling of the host, container and storage-reader roles, read
//! afresh for every request. Each read walks every path component without
//! following symlinks and checks root ownership and mode
//! (`limeos_executor_storage::read_root_owned_json`), then runs
//! `Ceiling::admit` for this executor's role: `core_uid` 0 refused, the
//! ceiling well formed, its flags suited to the role. `limeosctl
//! check-ceiling` runs the same two steps. A request whose read fails is
//! refused with `Unavailable`: there is no earlier copy to fall back to, so a
//! revocation takes effect on the next request and a broken file refuses
//! everything until it is fixed. `Forbidden` is kept for a ceiling that was
//! read and says no, so core never takes a broken or half-written file for a
//! definite refusal.
//!
//! A connection reads the ceiling twice: once when it is accepted, to refuse
//! a peer that is not core before any frame is read, and again once the
//! request frame has arrived. Only the second read authorizes the request.
//!
//! What the process set up at start from its first read stays as it was
//! until a restart: whether the receipt store is open, which observation
//! collector runs, and whether the storage reader's stack scan exists (the
//! scan itself rereads the ceiling before every pass). A request that a later
//! ceiling grants but that needs one of those is answered `Unavailable`
//! (`shared_role`), never with an empty answer.
use limeos_domain::{Error, ErrorCode};
use limeos_executor_container::ContainerCeiling;
use limeos_executor_protocol::{Ceiling, CeilingRefusal, CeilingRole};
use limeos_executor_storage::ProtectedRead;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Why a ceiling read was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The file could not be read safely.
    Read(ProtectedRead),
    /// It was read, but is not a ceiling this role may serve.
    Content(CeilingRefusal),
}
impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Refusal::Read(read) => read.message(),
            Refusal::Content(content) => content.message(),
        }
    }
    /// The error a refused request carries. Always `Unavailable`: the
    /// executor has no ceiling to answer from, which core must not take for
    /// the ceiling's "no" (core withdraws an operator's resolve request on
    /// `Forbidden`, but keeps it pending on `Unavailable`).
    pub fn error(self) -> Error {
        Error(ErrorCode::Unavailable)
    }
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Logs a refusal when its kind changes, and the recovery, so a broken file
/// writes one line per change, not one per request.
pub struct Transitions<R> {
    what: &'static str,
    last: Mutex<Option<R>>,
}
impl<R: Copy + Eq + std::fmt::Display> Transitions<R> {
    pub const fn new(what: &'static str) -> Self {
        Self {
            what,
            last: Mutex::new(None),
        }
    }
    pub fn note<T>(&self, result: &Result<T, R>) {
        let now = result.as_ref().err().copied();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if *last != now {
            match now {
                Some(refusal) => tracing::warn!(
                    ceiling = self.what,
                    reason = %refusal,
                    "executor ceiling refused; every request is refused until it is fixed"
                ),
                None => tracing::info!(ceiling = self.what, "executor ceiling accepted again"),
            }
            *last = now;
        }
    }
}

/// The container ceiling the receipt store authorizes against, as the
/// daemon derives it from the shared ceiling.
pub fn container_ceiling(ceiling: &Ceiling) -> ContainerCeiling {
    ContainerCeiling {
        version: ceiling.version,
        core_uid: ceiling.core_uid,
        allow_restart: ceiling.allow_restart,
        allow_start: ceiling.allow_start,
        allow_stop: ceiling.allow_stop,
        allow_stack: ceiling.allow_stack,
        managed_containers: ceiling.managed_containers.clone(),
        managed_stacks: ceiling.managed_stacks.clone(),
    }
}

/// One role's ceiling file.
pub struct CeilingFile {
    anchor: PathBuf,
    path: PathBuf,
    owner: u32,
    role: CeilingRole,
    log: Transitions<Refusal>,
}
impl CeilingFile {
    /// The installed file at `path`: walked from `/`, every component
    /// root-owned.
    pub fn installed(path: &Path, role: CeilingRole) -> Self {
        Self::at(Path::new("/"), path, 0, role)
    }
    /// `path` walked from `anchor`, every component owned by `owner`; for
    /// tests, which cannot create root-owned files. The checks are the same.
    #[cfg(test)]
    pub(crate) fn beneath(anchor: &Path, path: &Path, owner: u32, role: CeilingRole) -> Self {
        Self::at(anchor, path, owner, role)
    }
    fn at(anchor: &Path, path: &Path, owner: u32, role: CeilingRole) -> Self {
        Self {
            anchor: anchor.to_path_buf(),
            path: path.to_path_buf(),
            owner,
            role,
            log: Transitions::new("shared"),
        }
    }
    /// The ceiling as the file says now. Each call rereads the file; a
    /// change of refusal kind, and the recovery, is logged.
    pub fn load(&self) -> Result<Ceiling, Refusal> {
        let result = self.read();
        self.log.note(&result);
        result
    }
    /// The ceiling for a request from `peer_uid`, read now: `Unavailable`
    /// when the file is refused, `Forbidden` when it names another UID as
    /// core.
    pub fn for_peer(&self, peer_uid: u32) -> Result<Ceiling, Error> {
        let ceiling = self.load().map_err(Refusal::error)?;
        if peer_uid != ceiling.core_uid {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(ceiling)
    }
    fn read(&self) -> Result<Ceiling, Refusal> {
        let ceiling: Ceiling =
            limeos_executor_storage::read_owned_json_beneath(&self.anchor, &self.path, self.owner)
                .map_err(Refusal::Read)?;
        ceiling.admit(self.role).map_err(Refusal::Content)?;
        Ok(ceiling)
    }
}

#[cfg(test)]
mod tests;
