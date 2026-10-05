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
pub(crate) async fn block_devices() -> Result<Vec<Resource>> {
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
pub(crate) fn disks(value: &Value) -> Result<Vec<Resource>> {
    fn visit(
        dev: &Value,
        parent: Option<String>,
        depth: usize,
        result: &mut Vec<Resource>,
    ) -> Result<()> {
        if depth > 8 || result.len() >= 64 {
            return Err(Error(ErrorCode::Unavailable));
        }
        let kind = text(&dev["type"], 32);
        if matches!(kind.as_str(), "loop" | "rom") {
            return Ok(());
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
        let id = identity.clone();
        let mounts = dev["mountpoints"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v.is_string()))
            .map(|v| text(v, 256))
            .filter(|s| !s.is_empty());
        let mut r = Resource::new(
            id.clone(),
            if kind == "disk" {
                ResourceKind::Disk
            } else {
                ResourceKind::Partition
            },
            name,
            if mounts.is_some() {
                "mounted"
            } else {
                "unmounted"
            }
            .into(),
            Source::Storage,
        );
        r.identity = Some(identity);
        r.parent = parent;
        r.mountpoint = mounts;
        r.filesystem = Some(text(&dev["fstype"], 64)).filter(|s| !s.is_empty());
        r.size_bytes = dev["size"]
            .as_u64()
            .or_else(|| dev["size"].as_str()?.parse().ok());
        result.push(r);
        if let Some(children) = dev["children"].as_array() {
            for c in children {
                visit(c, Some(id.clone()), depth + 1, result)?;
            }
        }
        Ok(())
    }
    let devices = value["blockdevices"]
        .as_array()
        .ok_or(Error(ErrorCode::Unavailable))?;
    let mut result = Vec::new();
    for dev in devices {
        visit(dev, None, 0, &mut result)?;
    }
    // Duplicate UUIDs are ambiguous, not usable resource identities.
    let mut ids = std::collections::HashSet::new();
    if result.iter().any(|r| !ids.insert(r.id.clone())) {
        return Err(Error(ErrorCode::Unavailable));
    }
    Ok(result)
}
pub(crate) fn pools(mounts: &str) -> Vec<Resource> {
    mounts
        .lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let fs = right.split_whitespace().next()?;
            if !matches!(fs, "fuse.mergerfs" | "mergerfs") {
                return None;
            }
            let mount = left
                .split_whitespace()
                .nth(4)?
                .replace("\\040", " ")
                .replace("\\011", "\t")
                .replace("\\134", "\\");
            if mount.len() > 256 {
                return None;
            }
            let mut r = Resource::new(
                format!("pool:{mount}"),
                ResourceKind::Pool,
                mount.clone(),
                "mounted".into(),
                Source::Pools,
            );
            r.mountpoint = Some(mount);
            r.filesystem = Some(fs.into());
            Some(r)
        })
        .take(16)
        .collect()
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
    fn disk_identity_is_uuid_not_device_name_and_duplicates_fail_closed() {
        let mut v = serde_json::json!({"blockdevices":[{"name":"sda","path":"/dev/sda","type":"disk","size":100,"serial":"drive-1","children":[{"name":"sda1","path":"/dev/sda1","type":"part","uuid":"data-1","fstype":"ext4","mountpoints":["/mnt/data"]}]}]});
        let a = disks(&v).unwrap();
        assert_eq!(a[1].id, "uuid:data-1");
        assert_eq!(a[1].status, "mounted");
        v["blockdevices"][0]["children"][0]["name"] = "sdb1".into();
        assert_eq!(disks(&v).unwrap()[1].id, a[1].id);
        let child = v["blockdevices"][0]["children"][0].clone();
        v["blockdevices"][0]["children"]
            .as_array_mut()
            .unwrap()
            .push(child);
        assert!(disks(&v).is_err());
        assert_eq!(
            pools("1 2 0:1 / /mnt/media rw - fuse.mergerfs media rw")[0].id,
            "pool:/mnt/media"
        );
    }
    #[test]
    fn frozen_reference_disk_fixture_agrees_on_identity_mount_and_status() {
        let v: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/read-semantics.json"))
                .unwrap();
        let r = disks(&v["lsblk"]).unwrap();
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
