use limeos_contracts::{HostMetrics, Resource, ResourceKind, Source};
use limeos_domain::{Error, ErrorCode, Result};
use serde_json::Value;
use std::{io::Read, path::Path, time::Duration};
use tokio::io::AsyncReadExt;

pub(crate) fn text(value: &Value, limit: usize) -> String {
    value
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}
pub(crate) fn read(path: &Path, limit: u64) -> Result<String> {
    let mut value = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(limit + 1).read_to_string(&mut value))
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    if value.len() as u64 > limit {
        return Err(Error(ErrorCode::Unavailable));
    }
    Ok(value)
}
pub(crate) fn cpu_counters(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|s| s.starts_with("cpu "))?;
    let values: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    if values.len() != 8 {
        return None;
    }
    Some((
        values.iter().try_fold(0u64, |sum, v| sum.checked_add(*v))?,
        values[3].checked_add(values[4])?,
    ))
}
pub(crate) fn cpu_delta(previous: Option<(u64, u64)>, current: Option<(u64, u64)>) -> Option<f64> {
    let (p, c) = (previous?, current?);
    let total = c.0.checked_sub(p.0)?;
    let idle = c.1.checked_sub(p.1)?;
    if total == 0 || idle > total {
        return None;
    }
    Some(100.0 * (total - idle) as f64 / total as f64)
}
fn memory(mem: &str) -> Option<f64> {
    let field = |key: &str| {
        mem.lines().find_map(|line| {
            line.strip_prefix(key)?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
    };
    let (total, available) = (field("MemTotal:")?, field("MemAvailable:")?);
    if total == 0 || available > total {
        return None;
    }
    Some(100.0 * (total - available) as f64 / total as f64)
}
pub(crate) fn host(previous: &mut Option<(u64, u64)>) -> (HostMetrics, Vec<String>) {
    let mut warnings = Vec::new();
    let current = read(Path::new("/proc/stat"), 65536)
        .ok()
        .and_then(|s| cpu_counters(&s));
    let cpu_percent = cpu_delta(*previous, current);
    *previous = current;
    if cpu_percent.is_none() {
        warnings.push("CPU baseline unavailable; waiting for a second sample".into());
    }
    let memory_percent = read(Path::new("/proc/meminfo"), 65536)
        .ok()
        .and_then(|s| memory(&s));
    if memory_percent.is_none() {
        warnings.push("Memory accounting unavailable".into());
    }
    let temperature_celsius = read(Path::new("/sys/class/thermal/thermal_zone0/temp"), 128)
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|v| v / 1000.0)
        .filter(|v| v.is_finite() && (-40.0..=150.0).contains(v));
    if temperature_celsius.is_none() {
        warnings.push("Temperature sensor unavailable".into());
    }
    let load_one = read(Path::new("/proc/loadavg"), 1024)
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .filter(|v| v.is_finite());
    let uptime_seconds = read(Path::new("/proc/uptime"), 1024)
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .filter(|v| v.is_finite());
    let disk_percent = fs2::total_space("/")
        .ok()
        .zip(fs2::free_space("/").ok())
        .and_then(|(total, free)| {
            (total > 0 && free <= total).then(|| 100.0 * (total - free) as f64 / total as f64)
        });
    if disk_percent.is_none() {
        warnings.push("Root filesystem usage unavailable".into());
    }
    (
        HostMetrics {
            cpu_percent,
            memory_percent,
            temperature_celsius,
            disk_percent,
            load_one,
            uptime_seconds,
        },
        warnings,
    )
}
// No caller-controlled executable, arguments, environment or output paths.
pub(crate) async fn block_devices() -> Result<(Vec<Resource>, Vec<String>)> {
    let mut child = tokio::process::Command::new("/usr/bin/lsblk")
        .args([
            "--bytes",
            "--json",
            "--output",
            "NAME,PATH,TYPE,SIZE,MODEL,SERIAL,TRAN,HOTPLUG,FSTYPE,UUID,LABEL,MOUNTPOINTS,MAJ:MIN",
        ])
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or(Error(ErrorCode::Unavailable))?
            .take(131073)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        if bytes.len() > 131072 {
            return Err(Error(ErrorCode::Unavailable));
        }
        if !child
            .wait()
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?
            .success()
        {
            return Err(Error(ErrorCode::Unavailable));
        }
        disks(&serde_json::from_slice::<Value>(&bytes).map_err(|_| Error(ErrorCode::Unavailable))?)
    })
    .await
    .map_err(|_| Error(ErrorCode::Unavailable))?
}
/// Devices kept in one observation. Loop and rom devices are skipped first and
/// never count against it.
const MAX_DEVICES: usize = 64;
const MAX_DEPTH: usize = 8;
/// Per-kind cap on the source warnings that name individual devices.
const MAX_NAMED: usize = 8;
struct Found {
    kind: String,
    name: String,
    path: String,
    identity: String,
    mounts: Vec<String>,
    swap: bool,
    filesystem: Option<String>,
    size: Option<u64>,
    parent: Option<usize>,
}
fn visit(
    dev: &Value,
    parent: Option<usize>,
    depth: usize,
    found: &mut Vec<Found>,
    omitted: &mut bool,
) {
    let kind = text(&dev["type"], 32);
    if matches!(kind.as_str(), "loop" | "rom") {
        return;
    }
    if depth > MAX_DEPTH || found.len() >= MAX_DEVICES {
        *omitted = true;
        return;
    }
    let name = text(&dev["name"], 64);
    let uuid = text(&dev["uuid"], 128);
    let serial = text(&dev["serial"], 128);
    let path = text(&dev["path"], 128);
    let identity = if !uuid.is_empty() {
        format!("uuid:{uuid}")
    } else if !serial.is_empty() {
        format!("serial:{serial}")
    } else {
        format!("device:{path}")
    };
    let filesystem = Some(text(&dev["fstype"], 64)).filter(|s| !s.is_empty());
    let mut mounts: Vec<String> = dev["mountpoints"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter(|v| v.is_string())
                .take(32)
                .map(|v| text(v, 256))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    // An active swap area is listed as `[SWAP]`; it is not a path.
    let swap = mounts.iter().any(|m| m == "[SWAP]");
    mounts.retain(|m| m != "[SWAP]");
    // Several mountpoints (bind mounts, subvolumes): shortest path first.
    mounts.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    mounts.dedup();
    let index = found.len();
    found.push(Found {
        kind,
        name,
        path,
        identity,
        mounts,
        swap,
        filesystem,
        size: dev["size"]
            .as_u64()
            .or_else(|| dev["size"].as_str()?.parse().ok()),
        parent,
    });
    if let Some(children) = dev["children"].as_array() {
        for c in children {
            visit(c, Some(index), depth + 1, found, omitted);
        }
    }
}
fn named(warnings: &mut Vec<String>, lines: Vec<String>, more: impl Fn(usize) -> String) {
    let extra = lines.len().saturating_sub(MAX_NAMED);
    warnings.extend(lines.into_iter().take(MAX_NAMED));
    if extra > 0 {
        warnings.push(more(extra));
    }
}
/// One odd device never blanks the inventory. A duplicated UUID, serial or
/// path makes exactly those devices `ambiguous`: listed, with a reason, under
/// an `ambiguous:` id and without an identity, so nothing can address them.
pub(crate) fn disks(value: &Value) -> Result<(Vec<Resource>, Vec<String>)> {
    let devices = value["blockdevices"]
        .as_array()
        .ok_or(Error(ErrorCode::Unavailable))?;
    let mut found = Vec::new();
    let mut omitted = false;
    for dev in devices {
        visit(dev, None, 0, &mut found, &mut omitted);
    }
    let mut uses = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    for f in &found {
        uses.entry(&f.identity).or_default().push(&f.name);
    }
    let mut warnings = Vec::new();
    named(
        &mut warnings,
        uses.iter()
            .filter(|(_, names)| names.len() > 1)
            .map(|(identity, names)| {
                format!(
                    "Devices {} share {identity}; they are shown as ambiguous and cannot be used as identities",
                    names.join(", ")
                )
            })
            .collect(),
        |n| format!("{n} more identifiers are shared by several devices"),
    );
    let ids: Vec<String> = found
        .iter()
        .map(|f| {
            if uses
                .get(f.identity.as_str())
                .is_some_and(|names| names.len() > 1)
            {
                format!(
                    "ambiguous:{}",
                    if f.path.is_empty() { &f.name } else { &f.path }
                )
            } else {
                f.identity.clone()
            }
        })
        .collect();
    let mut several = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for (f, id) in found.iter().zip(&ids) {
        // Malformed input naming one path twice: keep the first only.
        if !seen.insert(id.as_str()) {
            omitted = true;
            continue;
        }
        let ambiguous = id.starts_with("ambiguous:");
        let status = if ambiguous {
            "ambiguous"
        } else if f.swap {
            "swap"
        } else if f.mounts.is_empty() {
            "unmounted"
        } else {
            "mounted"
        };
        let mut r = Resource::new(
            id.clone(),
            if f.kind == "disk" {
                ResourceKind::Disk
            } else {
                ResourceKind::Partition
            },
            f.name.clone(),
            status.into(),
            Source::Storage,
        );
        r.identity = (!ambiguous).then(|| f.identity.clone());
        r.parent = f.parent.and_then(|p| ids.get(p)).cloned();
        r.mountpoint = f.mounts.first().cloned();
        if f.mounts.len() > 1 {
            several.push(format!(
                "{} is mounted at {} paths; the shortest is shown",
                f.name,
                f.mounts.len()
            ));
        }
        r.filesystem = f.filesystem.clone();
        r.size_bytes = f.size;
        result.push(r);
    }
    named(&mut warnings, several, |n| {
        format!("{n} more devices are mounted at several paths")
    });
    if omitted {
        warnings.push(format!(
            "Storage inventory exceeds {MAX_DEVICES} devices or nesting depth {MAX_DEPTH}; some devices are omitted"
        ));
    }
    Ok((result, warnings))
}
/// Undo the kernel's octal escaping of a mountinfo field (`\040`, `\011`,
/// `\012`, `\134` and any other `\ooo`). A malformed escape or non-UTF-8
/// result is refused.
fn unescape(field: &str) -> Option<String> {
    let mut out = Vec::with_capacity(field.len());
    let mut bytes = field.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let mut value = 0u32;
        for _ in 0..3 {
            let digit = bytes.next().filter(|d| (b'0'..=b'7').contains(d))?;
            value = value * 8 + u32::from(digit - b'0');
        }
        out.push(u8::try_from(value).ok()?);
    }
    String::from_utf8(out).ok()
}
/// Mergerfs pools by mountpoint. Stacked mounts at one mountpoint are one pool:
/// the last listed, which is the one visible there.
pub(crate) fn pools(mounts: &str) -> Vec<Resource> {
    let mut found = std::collections::BTreeMap::new();
    for line in mounts.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let Some(fs) = right.split_whitespace().next() else {
            continue;
        };
        if !matches!(fs, "fuse.mergerfs" | "mergerfs") {
            continue;
        }
        let Some(mount) = left.split_whitespace().nth(4).and_then(unescape) else {
            continue;
        };
        if mount.len() > 256 {
            continue;
        }
        if found.len() >= 16 && !found.contains_key(&mount) {
            continue;
        }
        let mut r = Resource::new(
            format!("pool:{mount}"),
            ResourceKind::Pool,
            mount.clone(),
            "mounted".into(),
            Source::Pools,
        );
        r.mountpoint = Some(mount.clone());
        r.filesystem = Some(fs.into());
        found.insert(mount, r);
    }
    found.into_values().collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counters_reset_missing_fields_and_no_cpu_baseline_are_unknown() {
        assert_eq!(cpu_delta(None, Some((100, 20))), None);
        assert_eq!(cpu_delta(Some((100, 20)), Some((90, 30))), None);
        assert_eq!(cpu_delta(Some((100, 20)), Some((200, 50))), Some(70.0));
        assert_eq!(memory("MemTotal: 100 kB\nMemAvailable: 20 kB"), Some(80.0));
        assert_eq!(memory("MemTotal: 100 kB"), None);
        assert_eq!(cpu_counters("cpu 1 2"), None);
    }
    #[test]
    fn disk_identity_is_uuid_not_device_name_and_a_duplicate_is_ambiguous_not_fatal() {
        let mut v = serde_json::json!({"blockdevices":[{"name":"sda","path":"/dev/sda","type":"disk","size":100,"serial":"drive-1","children":[{"name":"sda1","path":"/dev/sda1","type":"part","uuid":"data-1","fstype":"ext4","mountpoints":["/mnt/data"]}]}]});
        let (a, warnings) = disks(&v).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(a[1].id, "uuid:data-1");
        assert_eq!(a[1].status, "mounted");
        v["blockdevices"][0]["children"][0]["name"] = "sdb1".into();
        assert_eq!(disks(&v).unwrap().0[1].id, a[1].id);
        let mut child = v["blockdevices"][0]["children"][0].clone();
        child["path"] = "/dev/sdb1".into();
        v["blockdevices"][0]["children"]
            .as_array_mut()
            .unwrap()
            .push(child);
        let (r, warnings) = disks(&v).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].id, "serial:drive-1");
        for (resource, path) in r[1..].iter().zip(["/dev/sda1", "/dev/sdb1"]) {
            assert_eq!(resource.id, format!("ambiguous:{path}"));
            assert_eq!(resource.status, "ambiguous");
            assert_eq!(resource.identity, None);
            assert_eq!(resource.parent.as_deref(), Some("serial:drive-1"));
        }
        assert!(warnings[0].contains("share uuid:data-1"));
        assert_eq!(
            pools("1 2 0:1 / /mnt/media rw - fuse.mergerfs media rw")[0].id,
            "pool:/mnt/media"
        );
    }
    fn part(name: &str, uuid: &str, fstype: &str, mounts: serde_json::Value) -> Value {
        serde_json::json!({"name":name,"path":format!("/dev/{name}"),"type":"part","size":10,"uuid":uuid,"fstype":fstype,"mountpoints":mounts})
    }
    #[test]
    fn btrfs_members_cloned_serials_swap_and_loops_keep_the_rest_of_the_inventory() {
        let mut devices: Vec<Value> = (0..100)
            .map(|i| serde_json::json!({"name":format!("loop{i}"),"path":format!("/dev/loop{i}"),"type":"loop","size":1,"mountpoints":[format!("/snap/core/{i}")]}))
            .collect();
        devices
            .push(serde_json::json!({"name":"sr0","path":"/dev/sr0","type":"rom","serial":"cd"}));
        // Two disks with the serial a USB bridge reports for both.
        devices.push(serde_json::json!({"name":"sda","path":"/dev/sda","type":"disk","serial":"bridge","children":[
            part("sda1", "pool-fs", "btrfs", serde_json::json!(["/srv/pool", "/srv/pool/@media", null])),
        ]}));
        devices.push(serde_json::json!({"name":"sdb","path":"/dev/sdb","type":"disk","serial":"bridge","children":[
            part("sdb1", "pool-fs", "btrfs", serde_json::json!([null])),
        ]}));
        devices.push(serde_json::json!({"name":"mmcblk0","path":"/dev/mmcblk0","type":"disk","serial":"0xboot","children":[
            part("mmcblk0p1", "boot-1", "vfat", serde_json::json!(["/boot/firmware"])),
            part("mmcblk0p2", "root-1", "ext4", serde_json::json!(["/"])),
            part("mmcblk0p3", "swap-1", "swap", serde_json::json!(["[SWAP]"])),
        ]}));
        let (r, warnings) = disks(&serde_json::json!({ "blockdevices": devices })).unwrap();
        // Loop and rom devices never count against the bound.
        assert_eq!(r.len(), 8, "{r:?}");
        let get = |name: &str| r.iter().find(|d| d.name == name).unwrap();
        for name in ["sda", "sdb", "sda1", "sdb1"] {
            assert_eq!(get(name).status, "ambiguous", "{name}");
            assert_eq!(get(name).identity, None);
            assert!(get(name).id.starts_with("ambiguous:/dev/"));
        }
        assert_eq!(get("sda1").parent.as_deref(), Some("ambiguous:/dev/sda"));
        // Visible: the chosen mountpoint is deterministic and the rest is noted.
        assert_eq!(get("sda1").mountpoint.as_deref(), Some("/srv/pool"));
        assert!(
            warnings
                .iter()
                .any(|w| w == "sda1 is mounted at 2 paths; the shortest is shown")
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("sda, sdb share serial:bridge"))
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("sda1, sdb1 share uuid:pool-fs"))
        );
        assert_eq!(get("mmcblk0p2").id, "uuid:root-1");
        assert_eq!(get("mmcblk0p2").status, "mounted");
        let swap = get("mmcblk0p3");
        assert_eq!(
            (swap.status.as_str(), swap.mountpoint.as_deref()),
            ("swap", None)
        );
        assert_eq!(swap.filesystem.as_deref(), Some("swap"));
        assert_eq!(swap.id, "uuid:swap-1");
    }
    #[test]
    fn more_devices_than_the_bound_are_truncated_with_a_warning() {
        let devices: Vec<Value> = (0..80)
            .map(|i| serde_json::json!({"name":format!("sd{i}"),"path":format!("/dev/sd{i}"),"type":"disk","serial":format!("s{i}")}))
            .collect();
        let (r, warnings) = disks(&serde_json::json!({ "blockdevices": devices })).unwrap();
        assert_eq!(r.len(), MAX_DEVICES);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("some devices are omitted"))
        );
        assert!(disks(&serde_json::json!({"blockdevices": "no"})).is_err());
    }
    #[test]
    fn mountinfo_octal_escapes_are_decoded_and_stacked_mounts_are_one_pool() {
        let mounts = "\
36 1 0:30 / /mnt/a\\012b rw - fuse.mergerfs a rw
37 1 0:31 / /mnt/media\\040pool rw - fuse.mergerfs one rw
38 1 0:32 / /mnt/media\\040pool rw - mergerfs two rw
39 1 0:33 / /mnt/x\\043y rw - fuse.mergerfs x rw
40 1 0:34 / /mnt/bad\\9 rw - fuse.mergerfs bad rw
41 1 0:35 / /mnt/data rw - ext4 /dev/sda1 rw";
        let p = pools(mounts);
        let ids: Vec<_> = p.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            ["pool:/mnt/a\nb", "pool:/mnt/media pool", "pool:/mnt/x#y"]
        );
        // The later, visible mount at one mountpoint wins.
        assert_eq!(p[1].filesystem.as_deref(), Some("mergerfs"));
        assert_eq!(unescape("/a\\134b"), Some("/a\\b".into()));
        assert_eq!(unescape("/a\\400"), None);
        assert_eq!(unescape("/a\\01"), None);
    }
    #[test]
    fn frozen_reference_disk_fixture_agrees_on_identity_mount_and_status() {
        let v: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/read-semantics.json"))
                .unwrap();
        let (r, warnings) = disks(&v["lsblk"]).unwrap();
        assert!(warnings.is_empty());
        let expected = &v["expected"]["disk"];
        assert_eq!(
            r[0].id,
            format!("serial:{}", expected["serial"].as_str().unwrap())
        );
        assert_eq!(
            r[1].id,
            format!("uuid:{}", expected["partition_uuid"].as_str().unwrap())
        );
        assert_eq!(
            r[1].mountpoint.as_deref(),
            expected["partition_mountpoint"].as_str()
        );
        assert_eq!(
            r[1].status == "mounted",
            expected["partition_mounted"].as_bool().unwrap()
        );
    }
}
