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
    let text = path.to_str().ok_or(Failure::InvalidPlan)?;
    if !text.starts_with('/')
        || text.len() > 512
        || text.chars().any(char::is_control)
        || (text != "/" && text[1..].split('/').any(|s| matches!(s, "" | "." | "..")))
    {
        return Err(Failure::InvalidPlan);
    }
    let mut fd = rustix::fs::open(
        "/",
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Failure::Unavailable)?;
    protected(&stat(&fd)?)?;
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
        .map_err(|e| {
            if e == rustix::io::Errno::NOENT {
                Failure::NotReady
            } else {
                Failure::UnsafePath
            }
        })?;
        let s = stat(&fd)?;
        if !leaf || protected_leaf {
            protected(&s)?;
        }
    }
    Ok(fd)
}

fn protected(s: &Statx) -> Result<()> {
    if s.stx_uid != 0 || s.stx_mode & 0o022 != 0 {
        Err(Failure::UnsafePath)
    } else {
        Ok(())
    }
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
