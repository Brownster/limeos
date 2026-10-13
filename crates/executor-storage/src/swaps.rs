//! Fresh swap identities; utilization is not identity and never enters the digest.
use crate::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Snapshot {
    entries: Vec<Entry>,
    pub devices: BTreeSet<DeviceNumber>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Entry {
    path: String,
    kind: String,
    size: u64,
    priority: i32,
    device: DeviceNumber,
    inode: u64,
}
#[cfg(test)]
fn rows(text: &str) -> Result<Vec<(String, String, u64, i32)>> {
    rows_with_profile(text, StorageReadProfile::Standard)
}
fn rows_with_profile(
    text: &str,
    profile: StorageReadProfile,
) -> Result<Vec<(String, String, u64, i32)>> {
    if text.len() > 65536 {
        return Err(Failure::Unavailable);
    }
    let mut lines = text.lines();
    if lines
        .next()
        .and_then(|s| mounts::fixed_fields::<5>(s, false).ok())
        != Some(["Filename", "Type", "Size", "Used", "Priority"])
    {
        return Err(Failure::Unavailable);
    }
    let mut result = Vec::new();
    let mut names = BTreeSet::new();
    for line in lines {
        let fields = mounts::fixed_fields::<5>(line, false)?;
        if result.len()
            >= if profile == StorageReadProfile::CombinedV1 {
                8
            } else {
                128
            }
            || !matches!(fields[1], "file" | "partition")
        {
            return Err(Failure::Unavailable);
        }
        let name = if profile == StorageReadProfile::CombinedV1 {
            read_admission::decode_path(fields[0], 512)?
        } else {
            mounts::unescape(fields[0])?
        };
        if !names.insert(name.clone()) || fields[3].parse::<u64>().is_err() {
            return Err(Failure::Unavailable);
        }
        result.push((
            name,
            fields[1].into(),
            fields[2].parse().map_err(|_| Failure::Unavailable)?,
            fields[4].parse().map_err(|_| Failure::Unavailable)?,
        ));
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}
impl Snapshot {
    pub fn collect() -> Result<Self> {
        Self::collect_with_profile(StorageReadProfile::Standard)
    }
    pub(crate) fn collect_with_profile(profile: StorageReadProfile) -> Result<Self> {
        let mut entries = Vec::new();
        let mut devices = BTreeSet::new();
        for (name, kind, size, priority) in
            rows_with_profile(&profile.read(Path::new("/proc/swaps"), 65536)?, profile)?
        {
            let fd = path::open(Path::new(&name), false, false)?;
            let s = path::stat(&fd)?;
            let device = match (kind.as_str(), s.stx_mode & 0o170000) {
                ("partition", 0o060000) => DeviceNumber {
                    major: s.stx_rdev_major,
                    minor: s.stx_rdev_minor,
                },
                ("file", 0o100000) if s.stx_nlink == 1 => DeviceNumber {
                    major: s.stx_dev_major,
                    minor: s.stx_dev_minor,
                },
                _ => return Err(Failure::Unavailable),
            };
            devices.insert(device);
            entries.push(Entry {
                path: name,
                kind,
                size,
                priority,
                device,
                inode: s.stx_ino,
            });
        }
        Ok(Self { entries, devices })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combined_swap_admission_bounds_complete_rows_and_decoded_names() {
        let header = "Filename Type Size Used Priority\n";
        let rows: String = (0..8)
            .map(|i| format!("/dev/sd{i} partition 10 0 -2\n"))
            .collect();
        assert_eq!(
            rows_with_profile(&format!("{header}{rows}"), StorageReadProfile::CombinedV1)
                .unwrap()
                .len(),
            8
        );
        let extra = format!("{header}{rows}/dev/extra partition 10 0 -2\n");
        assert_eq!(
            rows_with_profile(&extra, StorageReadProfile::CombinedV1),
            Err(Failure::Unavailable)
        );
        assert_eq!(
            rows_with_profile(&extra, StorageReadProfile::Standard)
                .unwrap()
                .len(),
            9
        );
        let oversized = format!("{header}/{} file 10 0 -2\n", "x".repeat(512));
        assert_eq!(
            rows_with_profile(&oversized, StorageReadProfile::CombinedV1),
            Err(Failure::Unavailable)
        );
        assert!(
            rows_with_profile(
                &format!("{header}{}", "a ".repeat(20000)),
                StorageReadProfile::CombinedV1
            )
            .is_err()
        );
    }
    #[test]
    fn swap_identity_ignores_utilization_but_bounds_and_validates_kernel_rows() {
        let header = "Filename\tType\tSize\tUsed\tPriority\n";
        assert_eq!(
            rows(&format!("{header}/dev/vdc1 partition 1024 0 -2\n")).unwrap(),
            rows(&format!("{header}/dev/vdc1 partition 1024 512 -2\n")).unwrap()
        );
        assert_eq!(
            rows(&format!("{header}/swap\\040file file 1024 0 -2\n")).unwrap()[0].0,
            "/swap file"
        );
        for bad in [
            "".into(),
            format!("{header}/dev/vdc1 unknown 1 0 -2\n"),
            format!("{header}/dev/vdc1 partition 1 nope -2\n"),
            format!("{header}/dev/vdc1 partition 1 0 -2\n/dev/vdc1 partition 1 0 -2\n"),
        ] {
            assert!(rows(&bad).is_err());
        }
    }
}
