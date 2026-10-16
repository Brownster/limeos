//! Fixed invocation built only from a genuine retained physical target owner.
//! Consumer authority and gated process/termination ownership remain in the
//! installed executor composition root, rather than in this launch description.
use crate::{Failure, Result};
use rustix::fs::{AtFlags, StatxFlags};
use std::{
    os::fd::{AsRawFd, BorrowedFd},
    time::Duration,
};

pub const MOUNT_HELPER: &str = "/usr/bin/mount";
pub const MOUNT_OPTIONS: &str = "rw,nosuid,nodev";
pub const MOUNT_HELPER_DEADLINE: Duration = Duration::from_secs(3);
pub const MOUNT_HELPER_OUTPUT_BYTES: usize = 8192;

/// Borrowing keeps all three parent descriptors alive through gated launch and
/// confirmed helper termination. CLOEXEC is intentional: the helper resolves
/// `/proc/<original-parent>/fd/<n>`, not its own descriptor table. It runs in the
/// parent's validated host mount namespace. A child/foreign namespace is refused
/// by the genuine admission owner before this object can be released for use.
///
/// This is not serializable, constructible from plan JSON, or a consumer proof.
/// The trusted runner must consume genuine admission and supervisor owners,
/// persist Prepared/actual helper identity before releasing its gate, clear the
/// environment, use null stdin and bound output/time. No caller chooses options,
/// helper image, source/target pathname or an environment variable.
pub struct MountInvocation<'a> {
    source: BorrowedFd<'a>,
    target: BorrowedFd<'a>,
    image: BorrowedFd<'a>,
    parent: u32,
    program_path: String,
    source_path: String,
    target_path: String,
}
impl<'a> MountInvocation<'a> {
    /// Crate-private: only the genuine MountTarget producer supplies retained FDs.
    pub(crate) fn from_retained(target: &'a crate::mount_admission::MountTarget) -> Result<Self> {
        Self::checked_descriptors(target.source_fd(), target.target_fd(), target.program_fd())
    }
    fn checked_descriptors(
        source: BorrowedFd<'a>,
        target: BorrowedFd<'a>,
        image: BorrowedFd<'a>,
    ) -> Result<Self> {
        let source_stat = descriptor_stat(source)?;
        let target_stat = descriptor_stat(target)?;
        let image_stat = descriptor_stat(image)?;
        if source_stat.stx_mode & 0o170000 != 0o060000
            || target_stat.stx_mode & 0o170000 != 0o040000
            || target_stat.stx_uid != 0
            || target_stat.stx_mode & 0o022 != 0
            || image_stat.stx_mode & 0o170000 != 0o100000
            || image_stat.stx_uid != 0
            || image_stat.stx_mode & 0o022 != 0
            || image_stat.stx_mode & 0o111 == 0
            || image_stat.stx_nlink != 1
            || image_stat.stx_size == 0
            || image_stat.stx_size > 8 * 1024 * 1024
        {
            return Err(Failure::UnsafePath);
        }
        let parent = std::process::id();
        Ok(Self {
            source,
            target,
            image,
            parent,
            program_path: descriptor_name(parent, image),
            source_path: descriptor_name(parent, source),
            target_path: descriptor_name(parent, target),
        })
    }
    pub fn parent_pid(&self) -> u32 {
        self.parent
    }
    pub fn program_path(&self) -> &str {
        &self.program_path
    }
    pub fn argument_zero(&self) -> &'static str {
        MOUNT_HELPER
    }
    pub fn arguments(&self) -> [&str; 11] {
        [
            "--no-canonicalize",
            "--no-mtab",
            "--internal-only",
            "--types",
            "ext4",
            "--options",
            MOUNT_OPTIONS,
            "--source",
            &self.source_path,
            "--target",
            &self.target_path,
        ]
    }
    /// Descriptor identities are borrowed for independent launch verification,
    /// never adopted from a wire report or resolved through a mutable /dev alias.
    pub fn source_descriptor(&self) -> BorrowedFd<'_> {
        self.source
    }
    pub fn target_descriptor(&self) -> BorrowedFd<'_> {
        self.target
    }
    pub fn image_descriptor(&self) -> BorrowedFd<'_> {
        self.image
    }
    pub fn environment(&self) -> [(&'static str, &'static str); 1] {
        [("LC_ALL", "C")]
    }
}
fn descriptor_name(parent: u32, descriptor: BorrowedFd<'_>) -> String {
    format!("/proc/{parent}/fd/{}", descriptor.as_raw_fd())
}
fn descriptor_stat(descriptor: BorrowedFd<'_>) -> Result<rustix::fs::Statx> {
    let mask = StatxFlags::BASIC_STATS | StatxFlags::MNT_ID;
    let stat = rustix::fs::statx(descriptor, "", AtFlags::EMPTY_PATH, mask)
        .map_err(|_| Failure::Unavailable)?;
    if stat.stx_mask & mask.bits() != mask.bits() {
        return Err(Failure::Unavailable);
    }
    Ok(stat)
}

#[cfg(test)]
mod tests;
