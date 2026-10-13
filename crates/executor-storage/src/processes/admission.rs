use super::*;

/// Closed process admission for trusted library callers. No wire overrides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessReadProfile {
    Standard,
    CombinedV1,
}

impl ProcessReadProfile {
    pub(super) fn admit(
        self,
        declarations: &ContainerStorageInventory,
        bindings: usize,
    ) -> Result<()> {
        if self == Self::Standard {
            return Ok(());
        }
        if declarations.containers.len() > 16 || bindings > 8 {
            return Err(Failure::Unavailable);
        }
        let running = declarations
            .containers
            .iter()
            .filter(|c| c.container.running)
            .count();
        let mounts = declarations
            .containers
            .iter()
            .try_fold(0usize, |n, c| n.checked_add(c.mounts.len()))
            .ok_or(Failure::Unavailable)?;
        if running > 8 || mounts > 32 {
            return Err(Failure::Unavailable);
        }
        Ok(())
    }
    pub(super) fn table_bytes(self) -> usize {
        match self {
            Self::Standard => TABLE_BYTES,
            Self::CombinedV1 => 64 * 1024,
        }
    }
    pub(super) fn table_rows(self) -> usize {
        match self {
            Self::Standard => TABLE_ROWS,
            Self::CombinedV1 => 512,
        }
    }
    pub(super) fn aggregate_bytes(self) -> usize {
        match self {
            Self::Standard => AGGREGATE_TABLE_BYTES,
            Self::CombinedV1 => 256 * 1024,
        }
    }
}

// Counts retained vector and string capacities. Allocator metadata, BTreeSet
// nodes, raw buffers, reports and runtime memory are separate bounded inputs;
// this payload allowance is neither total heap nor an RSS guarantee.
pub(super) struct MountPayload {
    used: usize,
    limit: usize,
}
impl MountPayload {
    pub(super) fn new(profile: ProcessReadProfile) -> Self {
        Self {
            used: 0,
            limit: match profile {
                ProcessReadProfile::Standard => usize::MAX,
                ProcessReadProfile::CombinedV1 => 512 * 1024,
            },
        }
    }
    fn check(&self, bytes: usize) -> Result<()> {
        self.used
            .checked_add(bytes)
            .filter(|n| *n <= self.limit)
            .map(|_| ())
            .ok_or(Failure::Unavailable)
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.check(bytes)?;
        self.used += bytes;
        Ok(())
    }
    pub(super) fn vector(&mut self, count: usize) -> Result<Vec<ContainerProcessMount>> {
        self.check(
            count
                .checked_mul(size_of::<ContainerProcessMount>())
                .ok_or(Failure::Unavailable)?,
        )?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| Failure::Unavailable)?;
        self.charge(
            values
                .capacity()
                .checked_mul(size_of::<ContainerProcessMount>())
                .ok_or(Failure::Unavailable)?,
        )?;
        Ok(values)
    }
    pub(super) fn filesystem(
        &mut self,
        value: &str,
        profile: ProcessReadProfile,
    ) -> Result<String> {
        if profile == ProcessReadProfile::Standard {
            return Ok(value.into());
        }
        if value.len() > 64 {
            return Err(Failure::Unavailable);
        }
        self.check(value.len())?;
        let mut output = String::new();
        output
            .try_reserve_exact(value.len())
            .map_err(|_| Failure::Unavailable)?;
        self.charge(output.capacity())?;
        output.push_str(value);
        Ok(output)
    }
    pub(super) fn path(&mut self, value: &str, profile: ProcessReadProfile) -> Result<String> {
        if profile == ProcessReadProfile::Standard {
            return mounts::unescape(value);
        }
        let mut length = 0usize;
        decoded_path(value, |_| {
            length += 1;
            if length > 512 {
                Err(Failure::Unavailable)
            } else {
                Ok(())
            }
        })?;
        self.check(length)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(length)
            .map_err(|_| Failure::Unavailable)?;
        self.charge(output.capacity())?;
        decoded_path(value, |byte| {
            output.push(byte);
            Ok(())
        })?;
        String::from_utf8(output).map_err(|_| Failure::Unavailable)
    }
}

fn decoded_path(value: &str, mut emit: impl FnMut(u8) -> Result<()>) -> Result<()> {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let byte = if bytes[i] == b'\\' {
            let escape = bytes.get(i..i + 4).ok_or(Failure::Unavailable)?;
            i += 4;
            match escape {
                b"\\040" => b' ',
                b"\\011" => b'\t',
                b"\\012" => b'\n',
                b"\\134" => b'\\',
                _ => return Err(Failure::Unavailable),
            }
        } else {
            let byte = bytes[i];
            i += 1;
            byte
        };
        emit(byte)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_capacity_allowance_accepts_exact_then_refuses_before_allocation() {
        let mut payload = MountPayload::new(ProcessReadProfile::CombinedV1);
        let count = payload.limit / size_of::<ContainerProcessMount>();
        let retained = payload.vector(count).unwrap();
        let remaining = payload.limit - payload.used;
        let strings: Vec<_> = (0..remaining / 64)
            .map(|_| {
                payload
                    .filesystem(&"x".repeat(64), ProcessReadProfile::CombinedV1)
                    .unwrap()
            })
            .collect();
        let tail = payload
            .filesystem(&"x".repeat(remaining % 64), ProcessReadProfile::CombinedV1)
            .unwrap();
        assert_eq!(payload.used, payload.limit);
        assert_eq!(
            payload.filesystem("x", ProcessReadProfile::CombinedV1),
            Err(Failure::Unavailable)
        );
        assert!(payload.vector(1).is_err());
        assert_eq!(payload.used, payload.limit);
        drop((retained, strings, tail));
    }

    #[test]
    fn oversized_decoded_fields_leave_payload_unspent() {
        let mut payload = MountPayload::new(ProcessReadProfile::CombinedV1);
        for value in [
            format!("/{}", "x".repeat(512)),
            format!("/{}", "\\040".repeat(512)),
            "/bad\\041escape".into(),
            "/trailing\\".into(),
        ] {
            assert_eq!(
                payload.path(&value, ProcessReadProfile::CombinedV1),
                Err(Failure::Unavailable)
            );
            assert_eq!(payload.used, 0);
        }
        assert_eq!(
            payload.filesystem(&"x".repeat(65), ProcessReadProfile::CombinedV1),
            Err(Failure::Unavailable)
        );
        assert_eq!(payload.used, 0);
        let value = payload
            .path(
                &format!("/{}", "\\040".repeat(511)),
                ProcessReadProfile::CombinedV1,
            )
            .unwrap();
        assert_eq!(value.len(), 512);
        assert_eq!(payload.used, value.capacity());
    }
}
