//! Preparation's private attempt: the shared private object tree
//! (`private_tree`) with the `preparing-` prefix, and preparation's IO seam.
use super::*;
use crate::private_tree::{self, Io, Kind, Labels};
pub(super) use crate::private_tree::{Barrier, Point, RealIo};

/// The private object tree as preparation uses it.
#[derive(Debug)]
pub(super) struct Preparation;
pub(super) type Attempt = private_tree::Attempt<Preparation>;
pub(super) type Object = private_tree::Object<Preparation>;

impl Kind for Preparation {
    type Metadata = PreparedMetadata;
    type Failure = Failure;
    type Error = PreparationError;
    const PREFIX: &'static str = "preparing-";
    const LABELS: Labels = Labels {
        recheck_root: "recheck preparation root",
        name: "generate preparation name",
        retain_root: "retain preparation root",
        create_attempt: "create preparation attempt",
        bind_attempt: "bind created preparation attempt",
        protect_attempt: "protect preparation attempt",
        stat_object: "stat prepared object",
        seal_object: "seal prepared object",
        fsync_object: "fsync prepared object",
    };
    fn error(failure: Failure, cleanup: Option<CleanupReport>) -> PreparationError {
        PreparationError { failure, cleanup }
    }
}
impl private_tree::Failure for Failure {
    fn changed() -> Self {
        Failure::ChangedObject
    }
    fn collision() -> Self {
        Failure::Collision
    }
    fn unsafe_root() -> Self {
        Failure::UnsafeRoot
    }
    fn unsupported_metadata() -> Self {
        Failure::UnsupportedMetadata
    }
    fn allocation() -> Self {
        Failure::Allocation
    }
    fn cancelled() -> Self {
        Failure::Cancelled
    }
    fn io(operation: &'static str, source: io::Error) -> Self {
        Failure::Io { operation, source }
    }
    fn into_io(self) -> Option<io::Error> {
        match self {
            Failure::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
impl private_tree::Metadata for PreparedMetadata {
    fn kind(&self) -> EntryKind {
        self.kind
    }
    fn size(&self) -> u64 {
        self.size
    }
    fn sha256(&self) -> Option<&str> {
        self.sha256.as_deref()
    }
    fn private_mode(&self) -> u32 {
        self.private_mode
    }
    fn private_owner(&self) -> (u32, u32) {
        (self.private_uid, self.private_gid)
    }
    fn set_private_owner(&mut self, uid: u32, gid: u32) {
        self.private_uid = uid;
        self.private_gid = gid;
    }
}

/// Preparation's IO; tests replace single operations to inject faults.
pub(super) trait PreparationIo {
    fn cleanup_count(&mut self, directory: &File) -> Result<usize, CleanupFailure> {
        private_tree::cleanup_count::<Failure>(directory)
    }
    fn write(&mut self, file: &mut File, input: &[u8]) -> io::Result<usize> {
        file.write(input)
    }
    fn read_at(&mut self, file: &File, out: &mut [u8], offset: u64) -> io::Result<usize> {
        file.read_at(out, offset)
    }
    fn sync(&mut self, file: &File, _: Barrier) -> io::Result<()> {
        file.sync_all()
    }
    fn stat_created(&mut self, file: &File) -> io::Result<Stat> {
        rustix::fs::fstat(file).map_err(Into::into)
    }
    fn reserve<T>(&mut self, values: &mut Vec<T>, count: usize) -> Result<(), Failure> {
        reserve(values, count)
    }
    fn checkpoint(&mut self, _: Point, _: &File) -> Result<(), Failure> {
        Ok(())
    }
}
impl PreparationIo for RealIo {}
impl<T: PreparationIo> Io<Preparation> for T {
    fn cleanup_count(&mut self, directory: &File) -> Result<usize, CleanupFailure> {
        PreparationIo::cleanup_count(self, directory)
    }
    fn write(&mut self, file: &mut File, input: &[u8]) -> io::Result<usize> {
        PreparationIo::write(self, file, input)
    }
    fn read_at(&mut self, file: &File, out: &mut [u8], offset: u64) -> io::Result<usize> {
        PreparationIo::read_at(self, file, out, offset)
    }
    fn sync(&mut self, file: &File, barrier: Barrier) -> io::Result<()> {
        PreparationIo::sync(self, file, barrier)
    }
    fn stat_created(&mut self, file: &File) -> io::Result<Stat> {
        PreparationIo::stat_created(self, file)
    }
    fn reserve<V>(&mut self, values: &mut Vec<V>, count: usize) -> Result<(), Failure> {
        PreparationIo::reserve(self, values, count)
    }
    fn checkpoint(&mut self, point: Point, directory: &File) -> Result<(), Failure> {
        PreparationIo::checkpoint(self, point, directory)
    }
}

/// Copy `input` whole into a private object, polling `cancelled`.
pub(super) fn write_all<I: PreparationIo>(
    io: &mut I,
    file: &mut File,
    input: &[u8],
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Failure> {
    private_tree::write_all::<Preparation, I>(io, file, input, cancelled)
}
