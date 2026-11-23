//! The shared ceiling of the host, container and storage-reader roles, read
//! afresh for every request. Each read walks every path component without
//! following symlinks and checks root ownership and mode
//! (`limeos_executor_storage::read_root_owned_json`), then checks the
//! ceiling's form, refuses `core_uid` 0, and checks that the ceiling suits
//! this executor's role. A request whose read fails is refused: there is no
//! earlier copy to fall back to, so a revocation takes effect on the next
//! request and a broken file refuses everything until it is fixed.
//!
//! What the process set up at start from its first read stays as it was
//! until a restart: whether the receipt store is open, which observation
//! collector runs, and whether the storage reader's stack scan exists (the
//! scan itself rereads the ceiling before every pass). So a narrowed ceiling
//! takes effect at once, while a capability added later that needs one of
//! those (writes on an executor started without them, say) is refused until
//! the executor restarts.
use limeos_domain::{Error, ErrorCode};
use limeos_executor_container::ContainerCeiling;
use limeos_executor_protocol::Ceiling;
use limeos_executor_storage::Failure;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Which executor reads the ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Host,
    Container,
    StorageReader,
}

/// The executor's role, fixed by argv at start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Role {
    pub kind: Kind,
    /// The shadow's read-only mode: a writable ceiling is refused.
    pub read_only: bool,
}

/// Why a ceiling read was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The file does not exist.
    Missing,
    /// A path component is a symlink, or is not root-owned, or is group- or
    /// world-writable.
    UnsafePath,
    /// Not a regular file with one link and at most 64 KiB, or not a ceiling.
    Malformed,
    /// The file could not be read.
    Unavailable,
    /// `core_uid` is 0.
    RootCore,
    /// Fails `Ceiling::configuration_valid`.
    Invalid,
    /// The storage reader's ceiling grants no storage read.
    ReaderWithoutReads,
    /// The storage reader's read flags on another role.
    ReaderFlagsElsewhere,
    /// Writes or logs on a role that never performs them.
    WritesOnReadOnlyRole,
}
impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Refusal::Missing => "the ceiling file does not exist",
            Refusal::UnsafePath => {
                "a component of the ceiling path is a symlink, not root-owned, or group- or world-writable"
            }
            Refusal::Malformed => {
                "the ceiling is not a single-link regular file of at most 64 KiB holding a valid ceiling"
            }
            Refusal::Unavailable => "the ceiling could not be read",
            Refusal::RootCore => {
                "the ceiling names core_uid 0; core runs as its own unprivileged user, so root is never accepted"
            }
            Refusal::Invalid => "the ceiling fails validation",
            Refusal::ReaderWithoutReads => {
                "the storage reader requires a dedicated read-only ceiling"
            }
            Refusal::ReaderFlagsElsewhere => {
                "storage, stack and container-health reads belong only to the storage reader"
            }
            Refusal::WritesOnReadOnlyRole => {
                "this executor cannot mutate containers or read their logs"
            }
        }
    }
    /// The error a refused request carries: `Unavailable` while the file
    /// cannot be read at all, `Forbidden` when it is there but refused.
    pub fn error(self) -> Error {
        Error(match self {
            Refusal::Missing | Refusal::Unavailable => ErrorCode::Unavailable,
            _ => ErrorCode::Forbidden,
        })
    }
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Whether `ceiling` suits `role`; the checks the daemon made once at start.
pub fn admit(role: Role, ceiling: &Ceiling) -> Result<(), Refusal> {
    let reader_flags = ceiling.allow_storage_read
        || ceiling.allow_stack_read
        || ceiling.allow_container_health_read;
    if role.read_only && ceiling.can_write() {
        return Err(Refusal::WritesOnReadOnlyRole);
    }
    match role.kind {
        Kind::StorageReader if !reader_flags => return Err(Refusal::ReaderWithoutReads),
        Kind::Host | Kind::Container if reader_flags => {
            return Err(Refusal::ReaderFlagsElsewhere);
        }
        _ => {}
    }
    if role.kind != Kind::Container && (ceiling.can_write() || ceiling.allow_container_logs) {
        return Err(Refusal::WritesOnReadOnlyRole);
    }
    Ok(())
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
    role: Role,
    /// The last refusal logged, so a broken file logs once, not per request.
    last: Mutex<Option<Refusal>>,
}
impl CeilingFile {
    /// The installed file at `path`: walked from `/`, every component
    /// root-owned.
    pub fn installed(path: &Path, role: Role) -> Self {
        Self::beneath(Path::new("/"), path, 0, role)
    }
    /// `path` walked from `anchor`, every component owned by `owner`; for
    /// tests, which cannot create root-owned files. The checks are the same.
    pub fn beneath(anchor: &Path, path: &Path, owner: u32, role: Role) -> Self {
        Self {
            anchor: anchor.to_path_buf(),
            path: path.to_path_buf(),
            owner,
            role,
            last: Mutex::new(None),
        }
    }
    /// The ceiling as the file says now. Each call rereads the file; a
    /// refusal is logged when it first appears and when it clears.
    pub fn load(&self) -> Result<Ceiling, Refusal> {
        let result = self.read();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let now = result.as_ref().err().copied();
        if *last != now {
            match now {
                Some(refusal) => tracing::warn!(
                    reason = refusal.message(),
                    "executor ceiling refused; every request is refused until it is fixed"
                ),
                None => tracing::info!("executor ceiling accepted again"),
            }
            *last = now;
        }
        result
    }
    /// The ceiling for one request from `peer_uid`, read now. Refused when
    /// the file is refused or names another UID as core; called before the
    /// request frame is read, so an unauthenticated peer costs no frame.
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
                .map_err(|failure| match failure {
                    Failure::NotReady => Refusal::Missing,
                    Failure::UnsafePath => Refusal::UnsafePath,
                    Failure::InvalidPlan => Refusal::Malformed,
                    _ => Refusal::Unavailable,
                })?;
        if ceiling.core_uid == 0 {
            return Err(Refusal::RootCore);
        }
        if !ceiling.configuration_valid() {
            return Err(Refusal::Invalid);
        }
        admit(self.role, &ceiling)?;
        Ok(ceiling)
    }
}

#[cfg(test)]
mod tests;
