//! Protected fstab inspection and pure replacement preview. Never writes.
use crate::*;
use limeos_domain::{
    STORAGE_FSTAB_BEGIN, STORAGE_FSTAB_END, StorageContract, StorageFstabEntry, StorageInventory,
};
use rustix::fd::OwnedFd;

pub(crate) fn read_with_profile(profile: StorageReadProfile) -> Result<(OwnedFd, String)> {
    let fd = path::open(Path::new("/etc/fstab"), false, true)?;
    let s = path::stat(&fd)?;
    if s.stx_mode & 0o170000 != 0o100000 || s.stx_size > 65536 || s.stx_nlink != 1 {
        return Err(Failure::UnsafePath);
    }
    let duplicate = rustix::io::dup(&fd).map_err(|_| Failure::Unavailable)?;
    if profile == StorageReadProfile::CombinedV1 {
        let mut file = std::fs::File::from(duplicate);
        let text = String::from_utf8(processes::read_bounded(&mut file, 65536, &|| Ok(()))?)
            .map_err(|_| Failure::Unavailable)?;
        return Ok((fd, text));
    }
    let mut text = String::new();
    std::fs::File::from(duplicate)
        .take(65537)
        .read_to_string(&mut text)
        .map_err(|_| Failure::Unavailable)?;
    if text.len() > 65536 {
        return Err(Failure::Unavailable);
    }
    Ok((fd, text))
}
fn span(text: &str) -> Result<Option<(usize, usize)>> {
    if text.len() > 65536
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(Failure::InvalidPlan);
    }
    let (mut begin, mut end, mut offset) = (None, None, 0);
    for line in text.split_inclusive('\n') {
        let raw = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = raw.trim();
        if trimmed.starts_with("# BEGIN LIMEOS STORAGE")
            || trimmed.starts_with("# END LIMEOS STORAGE")
        {
            if raw == STORAGE_FSTAB_BEGIN && begin.is_none() && end.is_none() {
                begin = Some(offset);
            } else if raw == STORAGE_FSTAB_END && begin.is_some() && end.is_none() {
                end = Some(offset + line.len());
            } else {
                return Err(Failure::InvalidPlan);
            }
        }
        offset += line.len();
    }
    match (begin, end) {
        (None, None) => Ok(None),
        (Some(b), Some(e)) => Ok(Some((b, e))),
        _ => Err(Failure::InvalidPlan),
    }
}
#[cfg(test)]
pub(crate) fn entries(text: &str, topology: &topology::Topology) -> Result<Vec<StorageFstabEntry>> {
    entries_with_profile(text, topology, StorageReadProfile::Standard)
}
pub(crate) fn entries_with_profile(
    text: &str,
    topology: &topology::Topology,
    profile: StorageReadProfile,
) -> Result<Vec<StorageFstabEntry>> {
    let managed = span(text)?;
    let mut entries = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let owns = managed.is_some_and(|(b, e)| (b..e).contains(&offset));
        offset += line.len();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if entries.len() >= 128 {
            return Err(Failure::Unavailable);
        }
        // Inline comments after the six standard fields are preserved privately.
        let mut words = trimmed
            .split_whitespace()
            .take_while(|s| !s.starts_with('#'));
        let mut fields = [""; 6];
        for field in &mut fields {
            *field = words.next().ok_or(Failure::InvalidPlan)?;
        }
        if words.next().is_some()
            || !fields[4].bytes().all(|c| c.is_ascii_digit())
            || !fields[5].bytes().all(|c| c.is_ascii_digit())
        {
            return Err(Failure::InvalidPlan);
        }
        let decode = |value: &str| {
            if profile == StorageReadProfile::CombinedV1 {
                read_admission::decode_path(value, 512)
            } else {
                mounts::unescape(value)
            }
        };
        if profile == StorageReadProfile::CombinedV1 && fields[2].len() > 64 {
            return Err(Failure::Unavailable);
        }
        let source = decode(fields[0])?;
        let mountpoint = if fields[2] == "swap" && fields[1] == "none" {
            "/".into()
        } else {
            decode(fields[1])?
        };
        let uuid = source
            .strip_prefix("UUID=")
            .or_else(|| source.strip_prefix("/dev/disk/by-uuid/"))
            .map(str::to_string);
        let number = topology.0.iter().find_map(|(n, node)| {
            (source == format!("/dev/{}", node.name)).then_some(inventory::wire(*n))
        });
        // Unresolved local aliases need PARTUUID/label/mapping proof before an
        // assignment can coexist with them. Boot/root records cannot select a
        // candidate because the full connected boot graph is already excluded.
        let protected = mountpoint == "/"
            || mountpoint == "/boot"
            || mountpoint.starts_with("/boot/")
            || mountpoint == "/efi"
            || mountpoint.starts_with("/efi/");
        let local = matches!(
            fields[2],
            "auto"
                | "ext2"
                | "ext3"
                | "ext4"
                | "xfs"
                | "btrfs"
                | "vfat"
                | "exfat"
                | "ntfs"
                | "ntfs3"
                | "swap"
        );
        if local && !protected && uuid.is_none() && number.is_none() {
            return Err(Failure::Unavailable);
        }
        entries.push(StorageFstabEntry {
            filesystem_uuid: uuid,
            device: number,
            mountpoint,
            managed: owns,
        });
    }
    Ok(entries)
}
/// Exact unmanaged bytes survive; a stale source digest or malformed ownership
/// marker is refused. A future effect must re-read/revalidate before atomic write.
pub fn planned_fstab(
    original: &str,
    contract: &StorageContract,
    timeout: u16,
    expected: &StorageInventory,
) -> Result<String> {
    if limeos_identity::digest(original) != expected.fstab_digest {
        return Err(Failure::IdentityMismatch);
    }
    let section = limeos_domain::storage_managed_fstab(contract, timeout, expected)
        .map_err(|_| Failure::InvalidPlan)?;
    let result = match span(original)? {
        Some((b, e)) => format!("{}{}{}", &original[..b], section, &original[e..]),
        None if original.is_empty() || original.ends_with('\n') => format!("{original}{section}"),
        None => return Err(Failure::InvalidPlan),
    };
    if result.len() > 65536 {
        return Err(Failure::InvalidPlan);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combined_fstab_fields_and_whole_row_bounds_preserve_private_options() {
        let topology = topology::Topology(BTreeMap::new());
        let raw: String = (0..128)
            .map(|i| format!("server:/data /mnt/{i} nfs secret=hidden 0 0 # operator\n"))
            .collect();
        let value = entries_with_profile(&raw, &topology, StorageReadProfile::CombinedV1).unwrap();
        assert_eq!(value.len(), 128);
        assert!(!serde_json::to_string(&value).unwrap().contains("hidden"));
        assert!(
            entries_with_profile(
                &(raw + "server:/data /extra nfs defaults 0 0\n"),
                &topology,
                StorageReadProfile::CombinedV1
            )
            .is_err()
        );
        assert!(
            entries_with_profile(
                &format!("server:/data /{} nfs defaults 0 0\n", "x".repeat(512)),
                &topology,
                StorageReadProfile::CombinedV1
            )
            .is_err()
        );
        assert_eq!(
            entries_with_profile(
                &"a ".repeat(20000),
                &topology,
                StorageReadProfile::CombinedV1
            ),
            Err(Failure::InvalidPlan)
        );
    }
    #[test]
    fn parser_bounds_markers_and_keeps_remote_credentials_private() {
        let topology = topology::Topology(BTreeMap::new());
        let raw = "UUID=root / ext4 defaults 0 1\n//server/share /mnt/remote cifs username=hidden,password=secret 0 0\n";
        let entries = entries(raw, &topology).unwrap();
        let serialized = serde_json::to_string(&entries).unwrap();
        assert!(
            !serialized.contains("hidden")
                && !serialized.contains("secret")
                && !serialized.contains("server")
        );
        for invalid in [
            format!("{STORAGE_FSTAB_BEGIN}\n"),
            format!("{STORAGE_FSTAB_END}\n"),
            format!(" {STORAGE_FSTAB_BEGIN}\n{STORAGE_FSTAB_END}\n"),
            format!(
                "{STORAGE_FSTAB_BEGIN}\n{STORAGE_FSTAB_END}\n{STORAGE_FSTAB_BEGIN}\n{STORAGE_FSTAB_END}\n"
            ),
            "x".repeat(65537),
            "UUID=a /mnt/data ext4 defaults\n".into(),
            "LABEL=data /mnt/data ext4 defaults 0 2\n".into(),
        ] {
            assert!(self::entries(&invalid, &topology).is_err());
        }
    }
    #[test]
    fn section_replacement_preserves_operator_bytes_and_refuses_stale_source() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/storage-contracts.json"
        ))
        .unwrap();
        let contract: StorageContract =
            serde_json::from_value(fixtures["cases"][0]["contract"].clone()).unwrap();
        let device = &contract.devices[0];
        let raw = format!(
            "# operator prefix\nUUID=root / ext4 defaults 0 1\n{STORAGE_FSTAB_BEGIN}\n# old managed data\n{STORAGE_FSTAB_END}\n# operator suffix\n"
        );
        let mut inventory = StorageInventory {
            host_root_mount_id: 1,
            topology_digest: "a".repeat(64),
            mounts_digest: "b".repeat(64),
            fstab_digest: limeos_identity::digest(&raw),
            fstab_entries: vec![],
            devices: vec![limeos_domain::StorageObservedDevice {
                device: limeos_domain::StorageBlockDevice { major: 8, minor: 1 },
                filesystem_uuid: device.filesystem_uuid.clone(),
                filesystem: "ext4".into(),
                serial: device.serial.clone(),
                boot_backing: false,
                in_use_as_swap: false,
                mounts: vec![],
            }],
        };
        let rendered = planned_fstab(&raw, &contract, 10, &inventory).unwrap();
        assert!(rendered.starts_with("# operator prefix\nUUID=root / ext4 defaults 0 1\n"));
        assert!(rendered.ends_with("# operator suffix\n"));
        assert!(!rendered.contains("old managed data"));
        assert_eq!(
            planned_fstab(&rendered, &contract, 10, &{
                inventory.fstab_digest = limeos_identity::digest(&rendered);
                inventory.clone()
            })
            .unwrap(),
            rendered
        );
        assert!(planned_fstab(&(raw + "# edit\n"), &contract, 10, &inventory).is_err());
    }
}
