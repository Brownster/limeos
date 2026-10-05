use crate::{Failure, Result, bounded_read};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct DeviceNumber {
    pub major: u32,
    pub minor: u32,
}
impl DeviceNumber {
    pub(crate) fn parse(s: &str) -> Result<Self> {
        let (a, b) = s.split_once(':').ok_or(Failure::Unavailable)?;
        Ok(Self {
            major: a.parse().map_err(|_| Failure::Unavailable)?,
            minor: b.parse().map_err(|_| Failure::Unavailable)?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Node {
    pub name: String,
    pub parents: BTreeSet<DeviceNumber>,
    pub serial: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Topology(pub BTreeMap<DeviceNumber, Node>);
impl Topology {
    pub fn collect() -> Result<Self> {
        let mut nodes = BTreeMap::new();
        for entry in std::fs::read_dir("/sys/dev/block").map_err(|_| Failure::Unavailable)? {
            let entry = entry.map_err(|_| Failure::Unavailable)?;
            if nodes.len() >= 128 {
                return Err(Failure::Unavailable);
            }
            let number =
                DeviceNumber::parse(entry.file_name().to_str().ok_or(Failure::Unavailable)?)?;
            let path = std::fs::canonicalize(entry.path()).map_err(|_| Failure::Unavailable)?;
            if !path.starts_with("/sys/devices") {
                return Err(Failure::Unavailable);
            }
            let name = bounded_read(&path.join("uevent"), 4096)?
                .lines()
                .find_map(|l| l.strip_prefix("DEVNAME="))
                .ok_or(Failure::Unavailable)?
                .to_string();
            if name.is_empty()
                || name.len() > 128
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-!".contains(&c))
            {
                return Err(Failure::Unavailable);
            }
            let mut parents = BTreeSet::new();
            let partition = path.join("partition").exists();
            if partition {
                parents.insert(DeviceNumber::parse(
                    bounded_read(&path.parent().ok_or(Failure::Unavailable)?.join("dev"), 64)?
                        .trim(),
                )?);
            }
            if !partition {
                for slave in
                    std::fs::read_dir(path.join("slaves")).map_err(|_| Failure::Unavailable)?
                {
                    parents.insert(DeviceNumber::parse(
                        bounded_read(
                            &slave.map_err(|_| Failure::Unavailable)?.path().join("dev"),
                            64,
                        )?
                        .trim(),
                    )?);
                }
            }
            let serial = [path.join("serial"), path.join("device/serial")]
                .iter()
                .find_map(|p| bounded_read(p, 256).ok().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control));
            nodes.insert(
                number,
                Node {
                    name,
                    parents,
                    serial,
                },
            );
        }
        if nodes.is_empty()
            || nodes.iter().any(|(n, v)| {
                v.parents.contains(n) || v.parents.iter().any(|p| !nodes.contains_key(p))
            })
        {
            return Err(Failure::Unavailable);
        }
        let topology = Self(nodes);
        // Cycles and excessive depth are unknown authority, never a usable disk.
        for n in topology.0.keys() {
            topology.ancestors(*n, &mut BTreeSet::new(), 0)?;
        }
        Ok(topology)
    }
    pub(crate) fn ancestors(
        &self,
        n: DeviceNumber,
        seen: &mut BTreeSet<DeviceNumber>,
        depth: usize,
    ) -> Result<()> {
        if depth > 16 || !seen.insert(n) {
            return Err(Failure::Unavailable);
        }
        for p in &self.0.get(&n).ok_or(Failure::Unavailable)?.parents {
            self.ancestors(*p, &mut seen.clone(), depth + 1)?;
        }
        Ok(())
    }
    pub fn connected(&self, seeds: &BTreeSet<DeviceNumber>) -> Result<BTreeSet<DeviceNumber>> {
        if seeds.iter().any(|n| !self.0.contains_key(n)) {
            return Err(Failure::Unavailable);
        }
        let mut all = seeds.clone();
        loop {
            let before = all.len();
            for (n, node) in &self.0 {
                if all.contains(n) || node.parents.iter().any(|p| all.contains(p)) {
                    all.insert(*n);
                    all.extend(&node.parents);
                }
            }
            if all.len() == before {
                return Ok(all);
            }
        }
    }
    pub fn serial_matches(&self, n: DeviceNumber, serial: &str) -> bool {
        // Only ancestry, never a sibling's serial, can verify an assignment.
        let mut next = vec![n];
        let mut seen = BTreeSet::new();
        let mut found = BTreeSet::new();
        while let Some(n) = next.pop() {
            if !seen.insert(n) {
                continue;
            }
            let Some(node) = self.0.get(&n) else {
                return false;
            };
            if let Some(s) = &node.serial {
                found.insert(s.as_str());
            }
            next.extend(&node.parents);
        }
        found.len() == 1 && found.contains(serial)
    }
    pub fn path(&self, n: DeviceNumber) -> Result<PathBuf> {
        Ok(PathBuf::from("/dev").join(&self.0.get(&n).ok_or(Failure::Unavailable)?.name))
    }
}
