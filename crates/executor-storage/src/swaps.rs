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
fn rows(text: &str) -> Result<Vec<(String, String, u64, i32)>> {
    let mut lines = text.lines();
    if lines
        .next()
        .map(|s| s.split_whitespace().collect::<Vec<_>>())
        != Some(vec!["Filename", "Type", "Size", "Used", "Priority"])
    {
        return Err(Failure::Unavailable);
    }
    let mut result = Vec::new();
    let mut names = BTreeSet::new();
    for line in lines {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 5 || result.len() >= 128 || !matches!(fields[1], "file" | "partition") {
            return Err(Failure::Unavailable);
        }
        let name = mounts::unescape(fields[0])?;
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
        let mut entries = Vec::new();
        let mut devices = BTreeSet::new();
        for (name, kind, size, priority) in rows(&bounded_read(Path::new("/proc/swaps"), 65536)?)? {
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
