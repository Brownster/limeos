//! Descriptor-relative traversal. No path component may be a symlink.
use crate::{Failure, Result};
use rustix::{
    fd::OwnedFd,
    fs::{AtFlags, Mode, OFlags, Statx, StatxFlags},
};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path},
};

pub(crate) fn stat(fd: &OwnedFd) -> Result<Statx> {
    let s = rustix::fs::statx(
        fd,
        "",
        AtFlags::EMPTY_PATH,
        StatxFlags::BASIC_STATS | StatxFlags::MNT_ID,
    )
    .map_err(|_| Failure::Unavailable)?;
    if s.stx_mask & (StatxFlags::BASIC_STATS | StatxFlags::MNT_ID).bits()
        != (StatxFlags::BASIC_STATS | StatxFlags::MNT_ID).bits()
    {
        return Err(Failure::Unavailable);
    }
    Ok(s)
}

pub(crate) fn open(path: &Path, directory: bool, protected_leaf: bool) -> Result<OwnedFd> {
    open_beneath(Path::new("/"), path, 0, directory, protected_leaf)
}

/// Why a walk stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalkError {
    /// Not a plain absolute path of at most 512 bytes.
    InvalidName,
    /// A component does not exist.
    Missing,
    /// A component is a symlink or of the wrong type (ELOOP, ENOTDIR, ...),
    /// or not owned by the expected owner, or group- or world-writable.
    Unsafe,
    /// The kernel refused access (EACCES, EPERM): this process may not
    /// search or read the path, which says nothing about its safety.
    Denied,
    /// Anything else: resource exhaustion (EMFILE, ENFILE, ENOMEM),
    /// interruption (EINTR), I/O errors, or a stat that failed.
    Unavailable,
}
impl WalkError {
    fn failure(self) -> Failure {
        match self {
            WalkError::InvalidName => Failure::InvalidPlan,
            WalkError::Missing => Failure::NotReady,
            WalkError::Unsafe => Failure::UnsafePath,
            WalkError::Denied | WalkError::Unavailable => Failure::Unavailable,
        }
    }
}
fn classify(errno: rustix::io::Errno) -> WalkError {
    use rustix::io::Errno;
    match errno {
        Errno::NOENT => WalkError::Missing,
        Errno::LOOP | Errno::NOTDIR | Errno::NXIO | Errno::NODEV | Errno::ISDIR => {
            WalkError::Unsafe
        }
        Errno::ACCESS | Errno::PERM => WalkError::Denied,
        _ => WalkError::Unavailable,
    }
}

/// The walk of [`open`], from `anchor` instead of `/` and with `owner` in
/// place of root.
pub(crate) fn open_beneath(
    anchor: &Path,
    path: &Path,
    owner: u32,
    directory: bool,
    protected_leaf: bool,
) -> Result<OwnedFd> {
    walk_beneath(anchor, path, owner, directory, protected_leaf).map_err(WalkError::failure)
}

/// [`open`] with the reason it stopped. `path` is written as an absolute
/// path but resolved beneath `anchor`. The anchor itself is opened by name
/// (it is trusted) and must pass the same ownership and mode check as every
/// component below it. Production callers walk from `/` with owner 0;
/// tests, which cannot create root-owned files, use their own directory and
/// UID.
pub(crate) fn walk_beneath(
    anchor: &Path,
    path: &Path,
    owner: u32,
    directory: bool,
    protected_leaf: bool,
) -> std::result::Result<OwnedFd, WalkError> {
    let text = path.to_str().ok_or(WalkError::InvalidName)?;
    if !text.starts_with('/')
        || text.len() > 512
        || text.chars().any(char::is_control)
        || (text != "/" && text[1..].split('/').any(|s| matches!(s, "" | "." | "..")))
    {
        return Err(WalkError::InvalidName);
    }
    let mut fd = rustix::fs::open(
        anchor,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| WalkError::Unavailable)?;
    let checked = |fd: &OwnedFd| -> std::result::Result<Statx, WalkError> {
        stat(fd).map_err(|_| WalkError::Unavailable)
    };
    if !protected(&checked(&fd)?, owner) {
        return Err(WalkError::Unsafe);
    }
    let parts: Vec<_> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    for (i, part) in parts.iter().enumerate() {
        let leaf = i + 1 == parts.len();
        let flags = if !leaf || directory {
            OFlags::PATH | OFlags::DIRECTORY
        } else {
            OFlags::RDONLY | OFlags::NONBLOCK
        };
        fd = rustix::fs::openat(
            &fd,
            *part,
            flags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(classify)?;
        let s = checked(&fd)?;
        if (!leaf || protected_leaf) && !protected(&s, owner) {
            return Err(WalkError::Unsafe);
        }
    }
    Ok(fd)
}

fn protected(s: &Statx, owner: u32) -> bool {
    s.stx_uid == owner && s.stx_mode & 0o022 == 0
}

pub(crate) fn plan(path: &Path) -> Result<limeos_domain::StorageMountWaitPlan> {
    let fd = open(path, false, true)?;
    let s = stat(&fd)?;
    if s.stx_mode & 0o170000 != 0o100000 || s.stx_size > 65536 {
        return Err(Failure::InvalidPlan);
    }
    let mut bytes = Vec::new();
    File::from(fd)
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::InvalidPlan)?;
    if bytes.len() > 65536 {
        return Err(Failure::InvalidPlan);
    }
    let plan: limeos_domain::StorageMountWaitPlan =
        serde_json::from_slice(&bytes).map_err(|_| Failure::InvalidPlan)?;
    plan.validate().map_err(|_| Failure::InvalidPlan)?;
    Ok(plan)
}
