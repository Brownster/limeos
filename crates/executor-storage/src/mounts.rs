use crate::{Failure, Result, topology::DeviceNumber};
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
