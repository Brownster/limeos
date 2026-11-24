//! Who may answer core on an unprivileged executor's socket.
//!
//! The container executor and the storage reader each run as their own
//! account, neither root nor core. Core trusts an answer on such a socket
//! when the socket file is owned by an account that is neither root nor core
//! and the kernel names that same account as the peer (`SO_PEERCRED`). Only
//! that account or root can create a file in the executor's runtime
//! directory (systemd creates it owned by the account, mode 0750, and `/run`
//! itself is root's), so no other unprivileged account can stand in for the
//! executor, and core cannot answer itself. One rule for every such socket:
//! [`ExecutorOwner`].
use limeos_domain::{Error, ErrorCode, Result};
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::Path,
};
use tokio::net::UnixStream;

/// The rule, for a core process running as `core_uid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutorOwner {
    core_uid: u32,
}
impl ExecutorOwner {
    /// For a core running as `core_uid` (its effective UID). Tests pass a
    /// UID other than their own to stand in for a separate executor account.
    pub const fn new(core_uid: u32) -> Self {
        Self { core_uid }
    }
    /// The account that may answer on a socket with this metadata (from
    /// `lstat` or `fstat`): its owner, when it is a socket owned by neither
    /// root nor core. `Forbidden` otherwise.
    pub fn socket_owner(&self, metadata: &std::fs::Metadata) -> Result<u32> {
        self.admit(metadata.file_type().is_socket(), metadata.uid())
    }
    fn admit(&self, socket: bool, owner: u32) -> Result<u32> {
        if !socket || owner == 0 || owner == self.core_uid {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(owner)
    }
    /// The kernel's peer UID must be the socket's owner. `Forbidden`
    /// otherwise.
    pub fn check_peer(&self, owner: u32, peer: u32) -> Result<()> {
        if peer != owner || self.admit(true, peer).is_err() {
            return Err(Error(ErrorCode::Forbidden));
        }
        Ok(())
    }
    /// Connects to the executor at `path` under the rule: the socket is
    /// checked before connecting (nothing is sent to a refused one) and the
    /// peer after. A socket that is missing or does not accept is
    /// `Unavailable`; one the rule refuses is `Forbidden`.
    pub async fn connect(&self, path: &Path) -> Result<UnixStream> {
        let metadata =
            std::fs::symlink_metadata(path).map_err(|_| Error(ErrorCode::Unavailable))?;
        let owner = self.socket_owner(&metadata)?;
        let stream = UnixStream::connect(path)
            .await
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        let peer = stream
            .peer_cred()
            .map_err(|_| Error(ErrorCode::Unavailable))?
            .uid();
        self.check_peer(owner, peer)?;
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    fn own_uid() -> u32 {
        std::fs::metadata("/proc/self").unwrap().uid()
    }

    #[test]
    fn only_a_socket_owned_by_neither_root_nor_core_is_admitted() {
        let rule = ExecutorOwner::new(150);
        assert_eq!(rule.admit(true, 151), Ok(151));
        for (socket, owner) in [(true, 0), (true, 150), (false, 151)] {
            assert_eq!(
                rule.admit(socket, owner),
                Err(Error(ErrorCode::Forbidden)),
                "{socket} {owner}"
            );
        }
    }

    #[test]
    fn the_peer_must_be_the_owner_and_never_root_or_core() {
        let rule = ExecutorOwner::new(150);
        assert_eq!(rule.check_peer(151, 151), Ok(()));
        for (owner, peer) in [(151, 152), (151, 0), (151, 150), (0, 0), (150, 150)] {
            assert_eq!(
                rule.check_peer(owner, peer),
                Err(Error(ErrorCode::Forbidden)),
                "{owner} {peer}"
            );
        }
    }

    #[tokio::test]
    async fn connect_admits_another_accounts_socket_and_refuses_cores_own_before_connecting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("executor.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let uid = own_uid();
        // As a core running under some other UID, this test's socket is an
        // executor's: admitted, and the kernel's peer is its owner.
        let stream = ExecutorOwner::new(uid.wrapping_add(1))
            .connect(&path)
            .await
            .unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        assert_eq!(stream.peer_cred().unwrap().uid(), uid);
        drop((stream, accepted));
        // As core itself: refused before any connection is made.
        assert_eq!(
            ExecutorOwner::new(uid).connect(&path).await.err(),
            Some(Error(ErrorCode::Forbidden))
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
        // Not a socket, and missing.
        let file = dir.path().join("file");
        std::fs::write(&file, b"").unwrap();
        let other = ExecutorOwner::new(uid.wrapping_add(1));
        assert_eq!(
            other.connect(&file).await.err(),
            Some(Error(ErrorCode::Forbidden))
        );
        let link = dir.path().join("link.sock");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(
            other.connect(&link).await.err(),
            Some(Error(ErrorCode::Forbidden))
        );
        assert_eq!(
            other.connect(&dir.path().join("missing")).await.err(),
            Some(Error(ErrorCode::Unavailable))
        );
    }
}
