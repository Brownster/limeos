//! Authenticate the retained host root without granting ptrace capability.
use super::{ComposeObjectIdentity, conflict, unavailable};
use limeos_domain::Result;
use rustix::{
    fd::OwnedFd,
    fs::{Mode, OFlags},
};

const TABLE_BYTES: usize = 64 * 1024;
const ROW_BYTES: usize = 4096;
const ROWS: usize = 512;

#[derive(Debug, PartialEq, Eq)]
struct RootMount {
    id: u64,
    major: u32,
    minor: u32,
    root: Vec<u8>,
}

pub(super) fn authenticate(anchor: &ComposeObjectIdentity) -> Result<()> {
    // The caller retains the root descriptor, preventing mount-ID recycling.
    // Each proc table is released before the next allocation. Only the bounded
    // root entry survives; paths in unrelated entries need not be UTF-8.
    let current = read_root("/proc/self/mountinfo")?;
    let host = read_root("/proc/1/mountinfo")?;
    agree(anchor, &current, &host)
}

fn agree(anchor: &ComposeObjectIdentity, current: &RootMount, host: &RootMount) -> Result<()> {
    if anchor.mount_id == 0
        || current != host
        || current.id != anchor.mount_id
        || (current.major, current.minor) != (anchor.major, anchor.minor)
    {
        return Err(conflict());
    }
    Ok(())
}

fn read_root(path: &'static str) -> Result<RootMount> {
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| unavailable())?;
    require_proc(&fd)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(TABLE_BYTES)
        .map_err(|_| unavailable())?;
    let mut scratch = [0; 4096];
    loop {
        let count = match rustix::io::read(&fd, &mut scratch) {
            Ok(count) => count,
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return Err(unavailable()),
        };
        if count == 0 {
            break;
        }
        if count > TABLE_BYTES - bytes.len() {
            return Err(unavailable());
        }
        bytes.extend_from_slice(&scratch[..count]);
    }
    root_mount(&bytes)
}

fn require_proc(fd: &OwnedFd) -> Result<()> {
    // Linux PROC_SUPER_MAGIC. Ordinary files cannot supply a host-view proof.
    let fs = rustix::fs::fstatfs(fd).map_err(|_| unavailable())?;
    let stat = rustix::fs::fstat(fd).map_err(|_| unavailable())?;
    if fs.f_type != 0x9fa0
        || stat.st_mode & 0o170000 != 0o100000
        || stat.st_uid != 0
        || stat.st_nlink == 0
    {
        return Err(unavailable());
    }
    Ok(())
}

fn decimal(bytes: &[u8]) -> Result<u64> {
    if bytes.is_empty() || (bytes.len() > 1 && bytes[0] == b'0') {
        return Err(unavailable());
    }
    bytes.iter().try_fold(0u64, |value, digit| {
        if !digit.is_ascii_digit() {
            return Err(unavailable());
        }
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(digit - b'0')))
            .ok_or_else(unavailable)
    })
}

fn root_mount(bytes: &[u8]) -> Result<RootMount> {
    if bytes.is_empty() || bytes.len() > TABLE_BYTES || bytes.last() != Some(&b'\n') {
        return Err(unavailable());
    }
    let mut root = None;
    for (index, line) in bytes[..bytes.len() - 1].split(|&b| b == b'\n').enumerate() {
        if index >= ROWS || line.len() > ROW_BYTES || line.iter().any(|&b| b < b' ') {
            return Err(unavailable());
        }
        let mut fields = line.split(|&b| b == b' ');
        let mut next = || {
            fields
                .next()
                .filter(|field| !field.is_empty())
                .ok_or_else(unavailable)
        };
        let id = decimal(next()?)?;
        if id == 0 {
            return Err(unavailable());
        }
        decimal(next()?)?; // The parent may lie outside the process's root.
        let device = next()?;
        let colon = device
            .iter()
            .position(|&b| b == b':')
            .ok_or_else(unavailable)?;
        let major = decimal(&device[..colon])?
            .try_into()
            .map_err(|_| unavailable())?;
        let minor = decimal(&device[colon + 1..])?
            .try_into()
            .map_err(|_| unavailable())?;
        let mount_root = next()?;
        let mountpoint = next()?;
        if !mount_root.starts_with(b"/") || !mountpoint.starts_with(b"/") {
            return Err(unavailable());
        }
        next()?; // Per-mount options.
        while next()? != b"-" {} // Optional kernel fields, bounded by row bytes.
        next()?; // Filesystem type.
        next()?; // Source.
        next()?; // Superblock options.
        if fields.next().is_some() {
            return Err(unavailable());
        }
        if mountpoint == b"/" {
            if root.is_some() {
                return Err(unavailable());
            }
            let mut retained = Vec::new();
            retained
                .try_reserve_exact(mount_root.len())
                .map_err(|_| unavailable())?;
            retained.extend_from_slice(mount_root);
            root = Some(RootMount {
                id,
                major,
                minor,
                root: retained,
            });
        }
    }
    root.ok_or_else(unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    const ROOT: &[u8] = b"27 1 8:1 / / rw,relatime shared:1 - ext4 /dev/vda1 rw\n";
    fn anchor() -> ComposeObjectIdentity {
        ComposeObjectIdentity {
            major: 8,
            minor: 1,
            mount_id: 27,
            inode: 2,
            mode: 0o40755,
            uid: 0,
            gid: 0,
        }
    }
    #[test]
    fn pinned_root_and_both_kernel_tables_must_agree() {
        let root = root_mount(ROOT).unwrap();
        agree(&anchor(), &root, &root).unwrap();
        let cloned = RootMount {
            id: 28,
            ..root_mount(ROOT).unwrap()
        };
        assert!(agree(&anchor(), &cloned, &root).is_err());
        assert!(agree(&anchor(), &cloned, &cloned).is_err());
        for changed in [
            ComposeObjectIdentity {
                mount_id: 0,
                ..anchor()
            },
            ComposeObjectIdentity {
                mount_id: 28,
                ..anchor()
            },
            ComposeObjectIdentity {
                major: 9,
                ..anchor()
            },
            ComposeObjectIdentity {
                minor: 2,
                ..anchor()
            },
        ] {
            assert!(agree(&changed, &root, &root).is_err());
        }
    }
    #[test]
    fn changed_mount_root_or_device_refuses() {
        let root = root_mount(ROOT).unwrap();
        for changed in [
            RootMount {
                root: b"/subdirectory".to_vec(),
                ..root_mount(ROOT).unwrap()
            },
            RootMount {
                major: 9,
                ..root_mount(ROOT).unwrap()
            },
            RootMount {
                minor: 2,
                ..root_mount(ROOT).unwrap()
            },
        ] {
            assert!(agree(&anchor(), &root, &changed).is_err());
        }
    }
    #[test]
    fn missing_duplicate_and_malformed_roots_refuse() {
        for bad in [
            b"".as_slice(),
            b"27 1 8:1 / /data rw - ext4 /dev/vda1 rw\n",
            b"27 1 8:1 / / rw - ext4 /dev/vda1 rw",
            b"0 1 8:1 / / rw - ext4 /dev/vda1 rw\n",
            b"027 1 8:1 / / rw - ext4 /dev/vda1 rw\n",
            b"18446744073709551616 1 8:1 / / rw - ext4 /dev/vda1 rw\n",
            b"27 1 4294967296:1 / / rw - ext4 /dev/vda1 rw\n",
            b"27 1 8:1:2 / / rw - ext4 /dev/vda1 rw\n",
            b"27 1 8:1 / / rw ext4 /dev/vda1 rw\n",
            b"27 1 8:1 / / rw - ext4 /dev/vda1\n",
            b"27 1 8:1 / / rw - ext4 /dev/vda1 rw extra\n",
            b"27 1 8:1 / / rw - ext4 /dev/vda1 rw\n\n",
        ] {
            assert!(root_mount(bad).is_err(), "{bad:?}");
        }
        let duplicate = [ROOT, ROOT].concat();
        assert!(root_mount(&duplicate).is_err());
    }
    #[test]
    fn byte_row_and_count_bounds_refuse() {
        assert!(root_mount(&vec![b'x'; TABLE_BYTES + 1]).is_err());
        let mut long = ROOT[..ROOT.len() - 1].to_vec();
        long.extend(vec![b'x'; ROW_BYTES]);
        long.push(b'\n');
        assert!(root_mount(&long).is_err());
        let mut many = ROOT.to_vec();
        for _ in 0..ROWS {
            many.extend_from_slice(b"28 27 0:1 / /other rw - tmpfs tmpfs rw\n");
        }
        assert!(root_mount(&many).is_err());
    }
    #[test]
    fn unrelated_non_utf8_paths_do_not_hide_the_root() {
        let bytes = [ROOT, b"28 27 0:1 / /other\xff rw - tmpfs tmpfs rw\n"].concat();
        assert_eq!(root_mount(&bytes).unwrap(), root_mount(ROOT).unwrap());
    }
    #[test]
    fn ordinary_descriptors_cannot_supply_proc_evidence() {
        let fd = rustix::fs::open("/", OFlags::PATH | OFlags::CLOEXEC, Mode::empty()).unwrap();
        assert!(require_proc(&fd).is_err());
    }
    #[test]
    fn actual_pid1_proc_table_is_bounded_and_has_a_root() {
        let root = read_root("/proc/1/mountinfo").unwrap();
        assert!(root.id > 0);
    }
}
