//! Read-only import/parsing ports. No filesystem writes or process execution.
mod imports;
mod reads;
pub use imports::*;
use limeos_domain::{
    ProtectionSourceEvidence, SnapraidConfig, SnapraidPreview, StorageReadFailure,
};
pub use reads::*;

/// The integrator supplies a protected RW-040 reader. Its implementation must
/// retain descriptors, exclude boot/swap, resolve fresh UUIDs, and use the host
/// namespace. Dashboard caches and /dev/disk/by-uuid alone cannot satisfy this port.
pub trait PoolProtectionReader {
    fn pool_mounts(
        &mut self,
        config: &limeos_domain::PoolsConfig,
    ) -> Result<Vec<PoolMountObservation>, StorageReadFailure>;
    fn sources(
        &mut self,
        preview: &SnapraidPreview,
    ) -> Result<ProtectionSourceEvidence, StorageReadFailure>;
    /// Fixed read queries only. Configuration is resolved from protected state;
    /// never execute a writable file or accept executable/shell text from callers.
    fn snapraid_read(
        &mut self,
        config: &SnapraidConfig,
        sources: &ProtectionSourceEvidence,
        query: SnapraidReadQuery,
    ) -> CapturedStorageRead;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapraidReadQuery {
    Status,
    Diff,
}

#[cfg(test)]
mod tests;
