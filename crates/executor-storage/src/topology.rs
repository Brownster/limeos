use crate::{Failure, Result, StorageReadProfile};
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
        Self::collect_with_profile(StorageReadProfile::Standard)
    }
    pub(crate) fn collect_with_profile(profile: StorageReadProfile) -> Result<Self> {
        let mut nodes = BTreeMap::new();
        let mut edges = 0usize;
        for entry in std::fs::read_dir("/sys/dev/block").map_err(|_| Failure::Unavailable)? {
            let entry = entry.map_err(|_| Failure::Unavailable)?;
            if nodes.len() >= profile.nodes() {
                return Err(Failure::Unavailable);
            }
            let number =
                DeviceNumber::parse(entry.file_name().to_str().ok_or(Failure::Unavailable)?)?;
            let path = std::fs::canonicalize(entry.path()).map_err(|_| Failure::Unavailable)?;
            if !path.starts_with("/sys/devices") {
                return Err(Failure::Unavailable);
            }
            let uevent = profile.read(&path.join("uevent"), 4096)?;
            let name = uevent
                .lines()
                .find_map(|l| l.strip_prefix("DEVNAME="))
                .ok_or(Failure::Unavailable)?;
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
                admit_parent(profile, &mut parents, edges, || {
                    DeviceNumber::parse(
                        profile
                            .read(&path.parent().ok_or(Failure::Unavailable)?.join("dev"), 64)?
                            .trim(),
                    )
                })?;
            }
            if !partition {
                for slave in
                    std::fs::read_dir(path.join("slaves")).map_err(|_| Failure::Unavailable)?
                {
                    admit_parent(profile, &mut parents, edges, || {
                        DeviceNumber::parse(
                            profile
                                .read(
                                    &slave.map_err(|_| Failure::Unavailable)?.path().join("dev"),
                                    64,
                                )?
                                .trim(),
                        )
                    })?;
                }
            }
            edges += parents.len();
            if profile == StorageReadProfile::CombinedV1 && edges > 128 {
                return Err(Failure::Unavailable);
            }
            let serial = [path.join("serial"), path.join("device/serial")]
                .iter()
                .find_map(|p| profile.read(p, 256).ok().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control));
            nodes.insert(
                number,
                Node {
                    name: name.into(),
                    parents,
                    serial,
                },
            );
        }
        Self(nodes).validate_with_profile(profile)
    }
    pub(crate) fn validate_with_profile(self, profile: StorageReadProfile) -> Result<Self> {
        let nodes = &self.0;
        if nodes.is_empty()
            || nodes.len() > profile.nodes()
            || nodes.iter().any(|(n, v)| {
                v.parents.contains(n) || v.parents.iter().any(|p| !nodes.contains_key(p))
            })
        {
            return Err(Failure::Unavailable);
        }
        if profile == StorageReadProfile::CombinedV1 {
            if nodes.values().any(|node| node.parents.len() > 8)
                || nodes.values().map(|node| node.parents.len()).sum::<usize>() > 128
            {
                return Err(Failure::Unavailable);
            }
            let mut heights = BTreeMap::new();
            for n in nodes.keys() {
                self.height(*n, &mut BTreeSet::new(), &mut heights)?;
            }
            return Ok(self);
        }
        // Cycles and excessive depth are unknown authority, never a usable disk.
        for n in nodes.keys() {
            self.ancestors(*n, &mut BTreeSet::new(), 0)?;
        }
        Ok(self)
    }
    fn height(
        &self,
        n: DeviceNumber,
        active: &mut BTreeSet<DeviceNumber>,
        heights: &mut BTreeMap<DeviceNumber, usize>,
    ) -> Result<usize> {
        if let Some(height) = heights.get(&n) {
            return Ok(*height);
        }
        if !active.insert(n) || active.len() > 17 {
            return Err(Failure::Unavailable);
        }
        let mut height = 0;
        for parent in &self.0.get(&n).ok_or(Failure::Unavailable)?.parents {
            height = height.max(1 + self.height(*parent, active, heights)?);
            if height > 16 {
                return Err(Failure::Unavailable);
            }
        }
        active.remove(&n);
        heights.insert(n, height);
        Ok(height)
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

fn admit_parent(
    profile: StorageReadProfile,
    parents: &mut BTreeSet<DeviceNumber>,
    edges: usize,
    read: impl FnOnce() -> Result<DeviceNumber>,
) -> Result<()> {
    if profile == StorageReadProfile::CombinedV1
        && (parents.len() >= 8 || edges + parents.len() >= 128)
    {
        return Err(Failure::Unavailable);
    }
    let inserted = parents.insert(read()?);
    if profile == StorageReadProfile::CombinedV1 && !inserted {
        return Err(Failure::Unavailable);
    }
    Ok(())
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use std::cell::Cell;
    const PROFILE: StorageReadProfile = StorageReadProfile::CombinedV1;
    fn n(i: u32) -> DeviceNumber {
        DeviceNumber { major: 8, minor: i }
    }
    fn graph(count: u32, parents: impl Fn(u32) -> BTreeSet<DeviceNumber>) -> Topology {
        Topology(
            (0..count)
                .map(|i| {
                    (
                        n(i),
                        Node {
                            name: format!("sd{i}"),
                            parents: parents(i),
                            serial: None,
                        },
                    )
                })
                .collect(),
        )
    }
    #[test]
    fn parent_admission_precedes_next_metadata_read_and_rejects_duplicates() {
        let calls = Cell::new(0);
        let mut parents = BTreeSet::new();
        for i in 0..8 {
            admit_parent(PROFILE, &mut parents, 0, || {
                calls.set(calls.get() + 1);
                Ok(n(i))
            })
            .unwrap();
        }
        assert_eq!(
            admit_parent(PROFILE, &mut parents, 0, || {
                calls.set(calls.get() + 1);
                Ok(n(9))
            }),
            Err(Failure::Unavailable)
        );
        assert_eq!(calls.get(), 8);
        assert_eq!(
            admit_parent(PROFILE, &mut BTreeSet::new(), 128, || {
                calls.set(calls.get() + 1);
                Ok(n(9))
            }),
            Err(Failure::Unavailable)
        );
        assert_eq!(calls.get(), 8);
        let mut parents = BTreeSet::from([n(1)]);
        assert_eq!(
            admit_parent(PROFILE, &mut parents, 0, || Ok(n(1))),
            Err(Failure::Unavailable)
        );
    }
    #[test]
    fn topology_node_parent_and_aggregate_limits_refuse_whole_graphs() {
        assert!(
            graph(48, |_| BTreeSet::new())
                .validate_with_profile(PROFILE)
                .is_ok()
        );
        assert!(
            graph(49, |_| BTreeSet::new())
                .validate_with_profile(PROFILE)
                .is_err()
        );
        assert!(
            graph(10, |i| if i == 9 {
                (0..8).map(n).collect()
            } else {
                BTreeSet::new()
            })
            .validate_with_profile(PROFILE)
            .is_ok()
        );
        assert!(
            graph(10, |i| if i == 9 {
                (0..9).map(n).collect()
            } else {
                BTreeSet::new()
            })
            .validate_with_profile(PROFILE)
            .is_err()
        );
        assert!(
            graph(24, |i| if i >= 8 {
                (0..8).map(n).collect()
            } else {
                BTreeSet::new()
            })
            .validate_with_profile(PROFILE)
            .is_ok()
        );
        assert!(
            graph(25, |i| if i >= 8 {
                (0..8).map(n).collect()
            } else {
                BTreeSet::new()
            })
            .validate_with_profile(PROFILE)
            .is_err()
        );
    }
    #[test]
    fn dag_height_bounds_cycles_missing_parents_and_shared_ancestry() {
        assert!(
            graph(17, |i| if i == 0 {
                BTreeSet::new()
            } else {
                BTreeSet::from([n(i - 1)])
            })
            .validate_with_profile(PROFILE)
            .is_ok()
        );
        assert!(
            graph(18, |i| if i == 0 {
                BTreeSet::new()
            } else {
                BTreeSet::from([n(i - 1)])
            })
            .validate_with_profile(PROFILE)
            .is_err()
        );
        assert!(
            graph(2, |i| BTreeSet::from([n(1 - i)]))
                .validate_with_profile(PROFILE)
                .is_err()
        );
        assert!(
            graph(1, |_| BTreeSet::from([n(9)]))
                .validate_with_profile(PROFILE)
                .is_err()
        );
        let dag = graph(32, |i| {
            if i < 2 {
                BTreeSet::new()
            } else {
                BTreeSet::from([n((i / 2 - 1) * 2), n((i / 2 - 1) * 2 + 1)])
            }
        });
        let mut heights = BTreeMap::new();
        assert_eq!(
            dag.height(n(31), &mut BTreeSet::new(), &mut heights),
            Ok(15)
        );
        assert_eq!(heights.len(), 31); // Complete shared ancestors are memoized once.
        assert!(dag.validate_with_profile(PROFILE).is_ok());
    }
}
