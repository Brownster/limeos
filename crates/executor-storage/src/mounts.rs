use crate::{Failure, Result, StorageReadProfile, read_admission, topology::DeviceNumber};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Mount {
    pub id: u64,
    pub device: DeviceNumber,
    pub root: String,
    pub path: String,
    pub filesystem: String,
    pub writable: bool,
}
pub(crate) fn parse(text: &str) -> Result<Vec<Mount>> {
    let mut mounts = Vec::new();
    let mut ids = BTreeSet::new();
    for line in text.lines() {
        if mounts.len() >= 4096 {
            return Err(Failure::Unavailable);
        }
        let (left, right) = line.split_once(" - ").ok_or(Failure::Unavailable)?;
        let l: Vec<_> = left.split_whitespace().collect();
        let r: Vec<_> = right.split_whitespace().collect();
        if l.len() < 6 || r.len() != 3 {
            return Err(Failure::Unavailable);
        }
        let id = l[0].parse().map_err(|_| Failure::Unavailable)?;
        if !ids.insert(id) {
            return Err(Failure::Unavailable);
        }
        mounts.push(Mount {
            id,
            device: DeviceNumber::parse(l[2])?,
            root: unescape(l[3])?,
            path: unescape(l[4])?,
            filesystem: r[0].into(),
            writable: l[5].split(',').any(|s| s == "rw") && r[2].split(',').any(|s| s == "rw"),
        });
    }
    if mounts.is_empty() {
        return Err(Failure::Unavailable);
    }
    Ok(mounts)
}

pub(crate) fn parse_with_profile(text: &str, profile: StorageReadProfile) -> Result<Vec<Mount>> {
    if profile == StorageReadProfile::Standard {
        return parse(text);
    }
    if text.is_empty() || text.len() > profile.table_bytes() || !text.ends_with('\n') {
        return Err(Failure::Unavailable);
    }
    let rows = text.lines().count();
    if rows > profile.table_rows() {
        return Err(Failure::Unavailable);
    }
    if rows
        .checked_mul(size_of::<Mount>())
        .is_none_or(|n| n > 128 * 1024)
    {
        return Err(Failure::Unavailable);
    }
    let mut mounts = Vec::<Mount>::new();
    mounts
        .try_reserve_exact(rows)
        .map_err(|_| Failure::Unavailable)?;
    let mut payload = mounts
        .capacity()
        .checked_mul(size_of::<Mount>())
        .ok_or(Failure::Unavailable)?;
    if payload > 128 * 1024 {
        return Err(Failure::Unavailable);
    }
    let mut ids = BTreeSet::new();
    for line in text.lines() {
        let (left, right) = line.split_once(" - ").ok_or(Failure::Unavailable)?;
        let l = fixed_fields::<6>(left, true)?;
        let r = fixed_fields::<3>(right, false)?;
        let id = l[0].parse().map_err(|_| Failure::Unavailable)?;
        let _: u64 = l[1].parse().map_err(|_| Failure::Unavailable)?;
        if id == 0 || !ids.insert(id) {
            return Err(Failure::Unavailable);
        }
        let device = DeviceNumber::parse(l[2])?;
        let mut allocate = |token: &str, path: bool| -> Result<String> {
            // Decoded bytes never exceed raw bytes. Limit requested space before
            // allocation; the complete table's raw cap also bounds this scratch.
            if payload
                .checked_add(token.len())
                .is_none_or(|n| n > 128 * 1024)
            {
                return Err(Failure::Unavailable);
            }
            let value = if path {
                read_admission::decode_path(token, 512)?
            } else {
                read_admission::copy_text(token, 64)?
            };
            payload = payload
                .checked_add(value.capacity())
                .filter(|n| *n <= 128 * 1024)
                .ok_or(Failure::Unavailable)?;
            Ok(value)
        };
        let root = allocate(l[3], true)?;
        let path = allocate(l[4], true)?;
        if !root.starts_with('/') || !path.starts_with('/') {
            return Err(Failure::Unavailable);
        }
        let filesystem = allocate(r[0], false)?;
        let writable = |options: &str| -> Result<bool> {
            let rw = options.split(',').any(|s| s == "rw");
            let ro = options.split(',').any(|s| s == "ro");
            if rw == ro {
                Err(Failure::Unavailable)
            } else {
                Ok(rw)
            }
        };
        let vfs_rw = writable(l[5])?;
        let fs_rw = writable(r[2])?;
        mounts.push(Mount {
            id,
            device,
            root,
            path,
            filesystem,
            writable: vfs_rw && fs_rw,
        });
    }
    Ok(mounts)
}

pub(crate) fn fixed_fields<const N: usize>(text: &str, optional: bool) -> Result<[&str; N]> {
    let mut tokens = text.split_whitespace();
    let mut fields = [""; N];
    for field in &mut fields {
        *field = tokens.next().ok_or(Failure::Unavailable)?;
    }
    for (i, _) in tokens.enumerate() {
        if !optional || i >= 32 {
            return Err(Failure::Unavailable);
        }
    }
    Ok(fields)
}
pub(crate) fn unescape(text: &str) -> Result<String> {
    let mut bytes = Vec::new();
    let mut i = 0;
    let raw = text.as_bytes();
    while i < raw.len() {
        if raw[i] == b'\\' {
            let escape = raw.get(i..i + 4).ok_or(Failure::Unavailable)?;
            bytes.push(match escape {
                b"\\040" => b' ',
                b"\\011" => b'\t',
                b"\\012" => b'\n',
                b"\\134" => b'\\',
                _ => return Err(Failure::Unavailable),
            });
            i += 4;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    String::from_utf8(bytes).map_err(|_| Failure::Unavailable)
}
pub(crate) fn protected(mounts: &[Mount]) -> Result<BTreeSet<DeviceNumber>> {
    let roots: Vec<_> = mounts.iter().filter(|m| m.path == "/").collect();
    if roots.len() != 1 {
        return Err(Failure::Unavailable);
    }
    Ok(mounts
        .iter()
        .filter(|m| {
            m.path == "/"
                || m.path == "/boot"
                || m.path.starts_with("/boot/")
                || m.path == "/efi"
                || m.path.starts_with("/efi/")
        })
        .map(|m| m.device)
        .collect())
}
