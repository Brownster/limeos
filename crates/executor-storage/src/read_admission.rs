//! Closed admission for trusted storage readers; no runtime/wire selector.
use crate::{Failure, Result, bounded_read, topology::Topology};
use limeos_domain::ContainerStorageInventory;
use serde::Serialize;
use std::{
    io::{self, Write},
    path::Path,
};

/// Profiles are selected by trusted code and cannot be imported from a report.
///
/// ```compile_fail
/// use limeos_executor_storage::StorageReadProfile;
/// let profile: StorageReadProfile = serde_json::from_str("\"combined_v1\"").unwrap();
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageReadProfile {
    Standard,
    CombinedV1,
}

impl StorageReadProfile {
    pub(crate) fn table_bytes(self) -> usize {
        match self {
            Self::Standard => 1024 * 1024,
            Self::CombinedV1 => 64 * 1024,
        }
    }
    pub(crate) fn table_rows(self) -> usize {
        match self {
            Self::Standard => 4096,
            Self::CombinedV1 => 512,
        }
    }
    pub(crate) fn nodes(self) -> usize {
        match self {
            Self::Standard => 128,
            Self::CombinedV1 => 48,
        }
    }
    pub(crate) fn references(self) -> usize {
        match self {
            Self::Standard => 512,
            Self::CombinedV1 => 128,
        }
    }
    pub(crate) fn admit_sources(self, inventory: &ContainerStorageInventory) -> Result<usize> {
        if self == Self::Standard
            && inventory.containers.len() > limeos_domain::CONTAINER_STORAGE_MAX_CONTAINERS
        {
            return Err(Failure::InvalidPlan);
        }
        if self == Self::CombinedV1 && inventory.containers.len() > 16 {
            return Err(Failure::Unavailable);
        }
        let mounts = inventory
            .containers
            .iter()
            .try_fold(0usize, |n, c| n.checked_add(c.mounts.len()))
            .ok_or(Failure::Unavailable)?;
        if self == Self::CombinedV1
            && (mounts > 32
                || inventory
                    .containers
                    .iter()
                    .filter(|c| c.container.running)
                    .count()
                    > 8)
        {
            return Err(Failure::Unavailable);
        }
        Ok(mounts)
    }
    pub(crate) fn admit_probes(self, topology: &Topology) -> Result<usize> {
        let count = topology
            .0
            .values()
            .filter(|node| probe_candidate(&node.name))
            .count();
        if self == Self::CombinedV1 && count > 40 {
            return Err(Failure::Unavailable);
        }
        Ok(count)
    }
    pub(crate) fn read(self, path: &Path, limit: usize) -> Result<String> {
        if self == Self::Standard {
            return bounded_read(path, limit as u64);
        }
        let mut file = std::fs::File::open(path).map_err(|_| Failure::Unavailable)?;
        String::from_utf8(crate::processes::read_bounded(
            &mut file,
            limit,
            &|| Ok(()),
        )?)
        .map_err(|_| Failure::Unavailable)
    }
    pub(crate) fn digest(self, value: &impl Serialize) -> Result<String> {
        if self == Self::CombinedV1 {
            crate::processes::digest_json(value, 256 * 1024)
        } else {
            crate::inventory::digest(value)
        }
    }
}
pub(crate) fn probe_candidate(name: &str) -> bool {
    !["loop", "ram", "zram", "sr"]
        .iter()
        .any(|p| name.starts_with(p))
}

// Bounded serialization without making the rejected JSON body first.
pub(crate) fn json_limit(value: &impl Serialize, limit: usize) -> Result<()> {
    struct Count {
        used: usize,
        limit: usize,
    }
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit - self.used {
                return Err(io::ErrorKind::OutOfMemory.into());
            }
            self.used += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Count { used: 0, limit }, value).map_err(|_| Failure::Unavailable)
}

pub(crate) fn copy_text(text: &str, limit: usize) -> Result<String> {
    if text.len() > limit {
        return Err(Failure::Unavailable);
    }
    let mut value = String::new();
    value
        .try_reserve_exact(text.len())
        .map_err(|_| Failure::Unavailable)?;
    value.push_str(text);
    Ok(value)
}

pub(crate) fn decode_path(text: &str, limit: usize) -> Result<String> {
    let mut length = 0;
    scan_path(text, |_| {
        length += 1;
        if length > limit {
            Err(Failure::Unavailable)
        } else {
            Ok(())
        }
    })?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| Failure::Unavailable)?;
    scan_path(text, |b| {
        bytes.push(b);
        Ok(())
    })?;
    String::from_utf8(bytes).map_err(|_| Failure::Unavailable)
}
fn scan_path(text: &str, mut emit: impl FnMut(u8) -> Result<()>) -> Result<()> {
    let raw = text.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        let b = if raw[i] == b'\\' {
            let escaped = raw.get(i..i + 4).ok_or(Failure::Unavailable)?;
            i += 4;
            match escaped {
                b"\\040" => b' ',
                b"\\011" => b'\t',
                b"\\012" => b'\n',
                b"\\134" => b'\\',
                _ => return Err(Failure::Unavailable),
            }
        } else {
            let b = raw[i];
            i += 1;
            b
        };
        emit(b)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
