//! Who may answer core on an unprivileged executor's socket.
//!
//! The container executor and the storage reader each run as their own
//! account, neither root nor core. Core trusts an answer on such a socket
//! when all of this holds, checked through descriptors opened once, never by
//! resolving the path again:
//!
//! - the socket's directory's parent (`/run` as packaged) is reached without
//!   following a symlink, is owned by root and is writable by no one else, so
//!   only root can create or replace the directory below it;
//! - the socket's directory (the unit's `RuntimeDirectory`, which systemd
//!   creates for the unit's account) is not a symlink, is owned by the same
//!   account as the socket and is not group- or world-writable, so only that
//!   account (or root) can create or replace the socket;
//! - the socket itself is not a symlink, has one link, is not world-writable
//!   and is owned by an account that is neither root nor core;
//! - core connects to that very inode (through its descriptor), and the
//!   kernel names the socket's owner as the peer (`SO_PEERCRED`).
//!
//! So a socket path that a drop-in, another unit or a mistyped `core.json`
//! points somewhere else (a world-writable directory, a directory another
//! account owns, a symlink) is refused instead of trusted, and core cannot
//! answer itself. One rule for every such socket: [`ExecutorOwner`].
use limeos_domain::{Error, ErrorCode, Result};
use rustix::{
    fd::{AsRawFd, OwnedFd},
    fs::{FileType, Mode, OFlags, ResolveFlags, Stat},
};
use std::{
    ffi::OsStr,
    path::{Component, Path},
};
use tokio::net::UnixStream;

/// Why a socket was not trusted. `Missing` is the executor not running (or
/// not yet listening); every other kind is a socket the rule refuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketRefusal {
    /// No socket, or a directory on the way to it, is there (`Unavailable`).
    Missing,
    /// The path is not absolute, has no parent directory, or has `.`, `..`
    /// or empty components.
    InvalidPath,
    /// A symlink on the way, or the directory's parent is not a root-owned
    /// directory writable only by root.
    UnsafeAnchor,
    /// The socket's directory is not owned by the socket's owner, or is
    /// group- or world-writable.
    UnsafeDirectory,
    /// Not a socket, more than one link, or world-writable.
    NotASocket,
    /// The socket is owned by root or by core itself.
    WrongOwner,
    /// The kernel's peer is not the socket's owner.
    WrongPeer,
}
impl SocketRefusal {
    pub fn message(self) -> &'static str {
        match self {
            Self::Missing => "the socket is not there (the executor is not running)",
            Self::InvalidPath => "the socket path is not a plain absolute path with a directory",
            Self::UnsafeAnchor => {
                "a symlink on the socket's path, or the parent of its directory is not owned by root or is writable by others"
            }
            Self::UnsafeDirectory => {
                "the socket's directory is not owned by the socket's owner, or is group- or world-writable"
            }
            Self::NotASocket => "not a socket, linked more than once, or world-writable",
            Self::WrongOwner => "the socket is owned by root or by core itself",
            Self::WrongPeer => "the process answering is not the socket's owner",
        }
    }
    /// The error a caller reports: `Unavailable` for a socket that is not
    /// there, `Forbidden` for one the rule refuses.
    pub fn error(self) -> Error {
        match self {
            Self::Missing => Error(ErrorCode::Unavailable),
            _ => Error(ErrorCode::Forbidden),
        }
    }
}
impl std::fmt::Display for SocketRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// The rule, for a core process running as `core_uid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutorOwner {
    core_uid: u32,
    /// Who must own the socket directory's parent: root in production.
    anchor_uid: u32,
}

/// What the rule establishes about one socket, held by an `O_PATH`
/// descriptor of the socket inode itself.
#[derive(Debug)]
pub struct ExecutorSocket {
    socket: OwnedFd,
    /// The socket's owner, the only account that may answer.
    pub owner: u32,
    /// Device, inode and mode of the socket, to tell a replacement.
    pub identity: (u64, u64, u32),
    rule: ExecutorOwner,
}

/// The facts the rule judges, from `fstat` of each descriptor.
#[derive(Clone, Copy, Debug)]
struct Facts {
    directory: bool,
    socket: bool,
    uid: u32,
    mode: u32,
    one_link: bool,
}
impl From<&Stat> for Facts {
    fn from(stat: &Stat) -> Self {
        let kind = FileType::from_raw_mode(stat.st_mode);
        Self {
            directory: kind == FileType::Directory,
            socket: kind == FileType::Socket,
            uid: stat.st_uid,
            mode: stat.st_mode,
            one_link: stat.st_nlink == 1,
        }
    }
}

impl ExecutorOwner {
    /// For a core running as `core_uid` (its effective UID).
    pub const fn new(core_uid: u32) -> Self {
        Self {
            core_uid,
            anchor_uid: 0,
        }
    }
    /// For tests, which cannot create root-owned directories: the socket
    /// directory's parent must be owned by `anchor_uid` instead of root. The
    /// checks are otherwise the same.
    #[doc(hidden)]
    pub const fn with_anchor_owner(core_uid: u32, anchor_uid: u32) -> Self {
        Self {
            core_uid,
            anchor_uid,
        }
    }
    /// The judgement over the three descriptors' facts, in path order.
    fn judge(
        &self,
        anchor: Facts,
        directory: Facts,
        socket: Facts,
    ) -> std::result::Result<u32, SocketRefusal> {
        if !anchor.directory || anchor.uid != self.anchor_uid || anchor.mode & 0o022 != 0 {
            return Err(SocketRefusal::UnsafeAnchor);
        }
        if !socket.socket || !socket.one_link || socket.mode & 0o002 != 0 {
            return Err(SocketRefusal::NotASocket);
        }
        if socket.uid == 0 || socket.uid == self.core_uid {
            return Err(SocketRefusal::WrongOwner);
        }
        if !directory.directory || directory.uid != socket.uid || directory.mode & 0o022 != 0 {
            return Err(SocketRefusal::UnsafeDirectory);
        }
        Ok(socket.uid)
    }
    /// Opens the socket at `path` under the rule, without connecting.
    pub fn open(&self, path: &Path) -> std::result::Result<ExecutorSocket, SocketRefusal> {
        let (anchor_path, directory_name, socket_name) = split(path).map_err(|refusal| {
            // Nothing there is an executor that is not running, whatever the
            // path's shape; the shape is judged only for something that exists.
            if std::fs::symlink_metadata(path).is_err() {
                SocketRefusal::Missing
            } else {
                refusal
            }
        })?;
        let anchor = rustix::fs::openat2(
            rustix::fs::CWD,
            anchor_path,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(refusal)?;
        let directory = rustix::fs::openat(
            &anchor,
            directory_name,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(refusal)?;
        let socket = rustix::fs::openat(
            &directory,
            socket_name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(refusal)?;
        let stat = |fd: &OwnedFd| rustix::fs::fstat(fd).map_err(|_| SocketRefusal::Missing);
        let socket_stat = stat(&socket)?;
        let owner = self.judge(
            Facts::from(&stat(&anchor)?),
            Facts::from(&stat(&directory)?),
            Facts::from(&socket_stat),
        )?;
        Ok(ExecutorSocket {
            socket,
            owner,
            identity: (socket_stat.st_dev, socket_stat.st_ino, socket_stat.st_mode),
            rule: *self,
        })
    }
    /// Opens and connects under the rule, saying why a socket was refused.
    pub async fn connect_explained(
        &self,
        path: &Path,
    ) -> std::result::Result<UnixStream, SocketRefusal> {
        self.open(path)?.connect().await
    }
    /// Opens and connects under the rule: nothing is sent to a refused
    /// socket. A socket that is not there is `Unavailable`; one the rule
    /// refuses is `Forbidden`.
    pub async fn connect(&self, path: &Path) -> Result<UnixStream> {
        self.connect_explained(path)
            .await
            .map_err(SocketRefusal::error)
    }
}

impl ExecutorSocket {
    /// Connects to this socket inode (through `/proc/self/fd`, so the path
    /// is not resolved again) and checks the kernel's peer.
    pub async fn connect(&self) -> std::result::Result<UnixStream, SocketRefusal> {
        let through = format!("/proc/self/fd/{}", self.socket.as_raw_fd());
        let stream = UnixStream::connect(&through)
            .await
            .map_err(|_| SocketRefusal::Missing)?;
        let peer = stream
            .peer_cred()
            .map_err(|_| SocketRefusal::Missing)?
            .uid();
        if peer != self.owner || self.rule.judge_peer(peer).is_err() {
            return Err(SocketRefusal::WrongPeer);
        }
        Ok(stream)
    }
    /// Whether the descriptor still names an unchanged socket (its owner,
    /// mode and link count are rechecked).
    pub fn unchanged(&self) -> bool {
        rustix::fs::fstat(&self.socket).is_ok_and(|stat| {
            let facts = Facts::from(&stat);
            facts.socket
                && facts.one_link
                && facts.uid == self.owner
                && (stat.st_dev, stat.st_ino, stat.st_mode) == self.identity
        })
    }
}
impl ExecutorOwner {
    fn judge_peer(&self, peer: u32) -> std::result::Result<(), SocketRefusal> {
        if peer == 0 || peer == self.core_uid {
            return Err(SocketRefusal::WrongPeer);
        }
        Ok(())
    }
}

/// The directory's parent, the directory's name and the socket's name.
fn split(path: &Path) -> std::result::Result<(&Path, &OsStr, &OsStr), SocketRefusal> {
    if !path.is_absolute()
        || path.as_os_str().len() > 512
        || path
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
        || path.to_str().is_none_or(|text| text.contains("//"))
    {
        return Err(SocketRefusal::InvalidPath);
    }
    let socket = path.file_name().ok_or(SocketRefusal::InvalidPath)?;
    let directory_path = path.parent().ok_or(SocketRefusal::InvalidPath)?;
    let directory = directory_path
        .file_name()
        .ok_or(SocketRefusal::InvalidPath)?;
    let anchor = directory_path.parent().ok_or(SocketRefusal::InvalidPath)?;
    Ok((anchor, directory, socket))
}
fn refusal(error: rustix::io::Errno) -> SocketRefusal {
    use rustix::io::Errno;
    match error {
        Errno::NOENT => SocketRefusal::Missing,
        // A symlink where a directory or the socket should be.
        Errno::LOOP | Errno::XDEV | Errno::NOTDIR => SocketRefusal::UnsafeAnchor,
        _ => SocketRefusal::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    fn own_uid() -> u32 {
        rustix::process::geteuid().as_raw()
    }
    fn facts(directory: bool, socket: bool, uid: u32, mode: u32) -> Facts {
        Facts {
            directory,
            socket,
            uid,
            mode,
            one_link: true,
        }
    }

    /// `base/run` (0755, the anchor), `base/run/exec` (0750, the
    /// executor's directory) and a listening `base/run/exec/executor.sock`
    /// (0660), all owned by this test's account.
    struct Layout {
        base: tempfile::TempDir,
        anchor: std::path::PathBuf,
        directory: std::path::PathBuf,
        socket: std::path::PathBuf,
        listener: UnixListener,
    }
    fn layout() -> Layout {
        let base = tempfile::tempdir().unwrap();
        let anchor = base.path().join("run");
        let directory = anchor.join("exec");
        std::fs::create_dir(&anchor).unwrap();
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&anchor, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o750)).unwrap();
        let socket = directory.join("executor.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o660)).unwrap();
        Layout {
            base,
            anchor,
            directory,
            socket,
            listener,
        }
    }
    /// Core as another account; the test's own directories are the anchor.
    fn rule() -> ExecutorOwner {
        ExecutorOwner::with_anchor_owner(own_uid().wrapping_add(1), own_uid())
    }

    #[test]
    fn the_judgement_follows_the_rule_for_each_component() {
        let rule = ExecutorOwner::new(150);
        let anchor = facts(true, false, 0, 0o40755);
        let directory = facts(true, false, 151, 0o40750);
        let socket = facts(false, true, 151, 0o140660);
        assert_eq!(rule.judge(anchor, directory, socket), Ok(151));
        // The anchor: not root's, writable by group or others, not a directory.
        for bad in [
            facts(true, false, 151, 0o40755),
            facts(true, false, 0, 0o40775),
            facts(true, false, 0, 0o41777),
            facts(false, false, 0, 0o100755),
        ] {
            assert_eq!(
                rule.judge(bad, directory, socket),
                Err(SocketRefusal::UnsafeAnchor)
            );
        }
        // The directory: a third account's, writable by group or others.
        for bad in [
            facts(true, false, 152, 0o40750),
            facts(true, false, 0, 0o40750),
            facts(true, false, 151, 0o40770),
            facts(true, false, 151, 0o40757),
        ] {
            assert_eq!(
                rule.judge(anchor, bad, socket),
                Err(SocketRefusal::UnsafeDirectory)
            );
        }
        // The socket: root's or core's, not a socket, world-writable, linked twice.
        for (bad, why) in [
            (facts(false, true, 0, 0o140660), SocketRefusal::WrongOwner),
            (facts(false, true, 150, 0o140660), SocketRefusal::WrongOwner),
            (
                facts(false, false, 151, 0o100660),
                SocketRefusal::NotASocket,
            ),
            (facts(false, true, 151, 0o140666), SocketRefusal::NotASocket),
            (
                Facts {
                    one_link: false,
                    ..socket
                },
                SocketRefusal::NotASocket,
            ),
        ] {
            assert_eq!(rule.judge(anchor, directory, bad), Err(why));
        }
    }

    #[test]
    fn a_peer_that_is_root_or_core_is_never_the_executor() {
        let rule = ExecutorOwner::new(150);
        assert_eq!(rule.judge_peer(151), Ok(()));
        assert_eq!(rule.judge_peer(0), Err(SocketRefusal::WrongPeer));
        assert_eq!(rule.judge_peer(150), Err(SocketRefusal::WrongPeer));
    }

    #[tokio::test]
    async fn an_executor_socket_in_its_own_directory_is_admitted_and_its_owner_answers() {
        let l = layout();
        let stream = rule().connect(&l.socket).await.unwrap();
        let (_accepted, _) = l.listener.accept().await.unwrap();
        assert_eq!(stream.peer_cred().unwrap().uid(), own_uid());
        let opened = rule().open(&l.socket).unwrap();
        assert!(opened.unchanged());
        // Replaced at the same name: the held descriptor tells.
        std::fs::remove_file(&l.socket).unwrap();
        let _replacement = UnixListener::bind(&l.socket).unwrap();
        assert!(!opened.unchanged());
        assert_ne!(rule().open(&l.socket).unwrap().identity, opened.identity);
    }

    #[tokio::test]
    async fn cores_own_socket_is_refused_before_connecting() {
        let l = layout();
        let as_core = ExecutorOwner::with_anchor_owner(own_uid(), own_uid());
        assert_eq!(
            as_core.connect_explained(&l.socket).await.err(),
            Some(SocketRefusal::WrongOwner)
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), l.listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_socket_in_a_world_or_group_writable_directory_is_refused() {
        for mode in [0o777, 0o1777, 0o770] {
            let l = layout();
            std::fs::set_permissions(&l.directory, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                rule().connect_explained(&l.socket).await.err(),
                Some(SocketRefusal::UnsafeDirectory),
                "{mode:o}"
            );
            assert_eq!(
                rule().connect(&l.socket).await.err(),
                Some(Error(ErrorCode::Forbidden))
            );
        }
        // And an anchor anyone can write to, such as /tmp.
        let l = layout();
        std::fs::set_permissions(&l.anchor, std::fs::Permissions::from_mode(0o1777)).unwrap();
        assert_eq!(
            rule().connect_explained(&l.socket).await.err(),
            Some(SocketRefusal::UnsafeAnchor)
        );
    }

    #[tokio::test]
    async fn an_anchor_owned_by_another_account_is_refused() {
        // The test cannot chown, so the directory's parent is judged against
        // an owner it does not have (root, in production).
        let l = layout();
        let elsewhere = ExecutorOwner::with_anchor_owner(own_uid().wrapping_add(1), 0);
        assert_eq!(
            elsewhere.connect_explained(&l.socket).await.err(),
            Some(SocketRefusal::UnsafeAnchor)
        );
    }

    #[tokio::test]
    async fn a_symlinked_parent_directory_or_socket_is_refused() {
        let l = layout();
        // The executor's directory reached through a symlink.
        let linked = l.anchor.join("linked");
        std::os::unix::fs::symlink(&l.directory, &linked).unwrap();
        assert_eq!(
            rule()
                .connect_explained(&linked.join("executor.sock"))
                .await
                .err(),
            Some(SocketRefusal::UnsafeAnchor)
        );
        // A symlink further up, in the anchor's path.
        let up = l.base.path().join("up");
        std::os::unix::fs::symlink(&l.anchor, &up).unwrap();
        assert_eq!(
            rule()
                .connect_explained(&up.join("exec/executor.sock"))
                .await
                .err(),
            Some(SocketRefusal::UnsafeAnchor)
        );
        // The socket itself as a symlink: the link, not its target, is judged.
        let alias = l.directory.join("alias.sock");
        std::os::unix::fs::symlink(&l.socket, &alias).unwrap();
        assert_eq!(
            rule().connect_explained(&alias).await.err(),
            Some(SocketRefusal::NotASocket)
        );
    }

    #[tokio::test]
    async fn missing_sockets_and_odd_paths_are_told_apart() {
        let l = layout();
        assert_eq!(
            rule()
                .connect_explained(&l.directory.join("missing.sock"))
                .await
                .err(),
            Some(SocketRefusal::Missing)
        );
        assert_eq!(
            rule()
                .connect(&l.directory.join("missing.sock"))
                .await
                .err(),
            Some(Error(ErrorCode::Unavailable))
        );
        // Paths of the wrong shape: refused when something is there, and
        // missing (the executor is not running) when nothing is.
        for odd in ["/proc", "/run/../run", "/run//", "/"] {
            assert_eq!(
                rule().connect_explained(Path::new(odd)).await.err(),
                Some(SocketRefusal::InvalidPath),
                "{odd}"
            );
        }
        for odd in [
            "relative/exec/x.sock",
            "/x.sock",
            "/run/../x/y.sock",
            "/run//x/y.sock",
        ] {
            assert_eq!(
                split(Path::new(odd)).err(),
                Some(SocketRefusal::InvalidPath),
                "{odd}"
            );
        }
        for absent in ["/limeos-unused", "/limeos-missing/../x.sock"] {
            assert_eq!(
                rule().connect_explained(Path::new(absent)).await.err(),
                Some(SocketRefusal::Missing),
                "{absent}"
            );
        }
        let file = l.directory.join("file");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(
            rule().connect_explained(&file).await.err(),
            Some(SocketRefusal::NotASocket)
        );
    }
}
