//! Unregistered fixed-image policy, separate from content-only ELF pinning.
//! Configuration selection is names only; the genuine worker checks their facts.
use crate::read_supervision::{Supervisor, Ticket};
use limeos_executor_storage::configuration::ConfigurationPaths;
use rustix::fs::{MemfdFlags, Mode, OFlags, SealFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{FileExt, MetadataExt},
    },
    path::Path,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
#[cfg(test)]
mod tests;
const IMAGE: &str = "/usr/lib/limeos/limeos-combined-read-probe";
const MAX_IMAGE: u64 = 8 * 1024 * 1024;
const WINDOW: Duration = Duration::from_secs(4);
const KEYS: [&str; 4] = [
    "LIMEOS_READ_CONTRACT",
    "LIMEOS_READ_POOLS",
    "LIMEOS_READ_PROTECTION",
    "LIMEOS_READ_SHARES",
];
const DIGEST_KEY: &str = "LIMEOS_READ_IMAGE_DIGEST";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    InvalidSelection,
    UntrustedImage,
    ImageLimit,
    ImageFormat,
    Changed,
    Expired,
    Unavailable,
}
pub type Result<T> = std::result::Result<T, Refusal>;

/// Names, never retained file contents or execution authority.
/// ```compile_fail
/// use limeos_executor::read_launch::ConfigurationSelection;
/// let imported: ConfigurationSelection = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, Serialize)]
pub struct ConfigurationSelection {
    paths: [String; 4],
}
impl ConfigurationSelection {
    pub fn new(paths: ConfigurationPaths<'_>) -> Result<Self> {
        let input = [paths.contract, paths.pools, paths.protection, paths.shares];
        let mut names = Vec::new();
        names
            .try_reserve_exact(4)
            .map_err(|_| Refusal::Unavailable)?;
        for path in input {
            let text = path.to_str().ok_or(Refusal::InvalidSelection)?;
            canonical(text)?;
            if names.iter().any(|name| name == text) {
                return Err(Refusal::InvalidSelection);
            }
            names.push(text.to_owned());
        }
        Ok(Self {
            paths: names.try_into().map_err(|_| Refusal::InvalidSelection)?,
        })
    }
    pub fn paths(&self) -> ConfigurationPaths<'_> {
        ConfigurationPaths {
            contract: Path::new(&self.paths[0]),
            pools: Path::new(&self.paths[1]),
            protection: Path::new(&self.paths[2]),
            shares: Path::new(&self.paths[3]),
        }
    }
    /// Internal test-role transport: bounded names, no UID/root/clock selectors.
    pub fn from_environment() -> Result<Self> {
        let mut names = Vec::new();
        for key in KEYS {
            let value = std::env::var_os(key).ok_or(Refusal::InvalidSelection)?;
            if value.len() > 512 {
                return Err(Refusal::InvalidSelection);
            }
            names.push(value.into_string().map_err(|_| Refusal::InvalidSelection)?);
        }
        Self::new(ConfigurationPaths {
            contract: Path::new(&names[0]),
            pools: Path::new(&names[1]),
            protection: Path::new(&names[2]),
            shares: Path::new(&names[3]),
        })
    }
}
fn canonical(text: &str) -> Result<()> {
    if !text.starts_with('/') || text.len() > 512 || text.chars().any(char::is_control) {
        return Err(Refusal::InvalidSelection);
    }
    let mut count = 0;
    for part in text[1..].split('/') {
        count += 1;
        if count > 16 || matches!(part, "" | "." | "..") {
            return Err(Refusal::InvalidSelection);
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct ImageMetadata {
    bytes: u64,
    sha256: String,
}
/// Immutable content only. This type supplies no protected-path provenance.
pub struct SealedExecutable {
    file: File,
    metadata: ImageMetadata,
}
impl SealedExecutable {
    /// Copy an already-open ordinary ELF file; no authentication is substituted.
    pub fn capture(source: &File) -> Result<Arc<Self>> {
        let before = source.metadata().map_err(|_| Refusal::Unavailable)?;
        if !before.is_file() {
            return Err(Refusal::ImageFormat);
        }
        if !(64..=MAX_IMAGE).contains(&before.len()) {
            return Err(Refusal::ImageLimit);
        }
        let mut header = [0; 64];
        source
            .read_exact_at(&mut header, 0)
            .map_err(|_| Refusal::Unavailable)?;
        elf(&header)?;
        let mut file = File::from(
            rustix::fs::memfd_create(
                "limeos-read-image",
                MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
            )
            .map_err(|_| Refusal::Unavailable)?,
        );
        let mut scratch = [0; 64 * 1024];
        let mut offset = 0;
        let mut hash = Sha256::new();
        while offset < before.len() {
            let wanted = (before.len() - offset).min(scratch.len() as u64) as usize;
            source
                .read_exact_at(&mut scratch[..wanted], offset)
                .map_err(|_| Refusal::Changed)?;
            file.write_all(&scratch[..wanted])
                .map_err(|_| Refusal::Unavailable)?;
            hash.update(&scratch[..wanted]);
            offset += wanted as u64;
        }
        if identity(&source.metadata().map_err(|_| Refusal::Unavailable)?) != identity(&before) {
            return Err(Refusal::Changed);
        }
        rustix::fs::fchmod(&file, Mode::RUSR | Mode::XUSR).map_err(|_| Refusal::Unavailable)?;
        rustix::fs::fcntl_add_seals(&file, seals()).map_err(|_| Refusal::Unavailable)?;
        if !rustix::fs::fcntl_get_seals(&file)
            .map_err(|_| Refusal::Unavailable)?
            .contains(seals())
        {
            return Err(Refusal::Unavailable);
        }
        Ok(Arc::new(Self {
            file,
            metadata: ImageMetadata {
                bytes: before.len(),
                sha256: hex(&hash.finalize()),
            },
        }))
    }
    pub fn metadata(&self) -> &ImageMetadata {
        &self.metadata
    }
    /// Content-only mechanics for trusted owned test images. Not protected policy.
    pub fn start(
        self: &Arc<Self>,
        manager: &Supervisor,
        selection: ConfigurationSelection,
    ) -> std::result::Result<Ticket, crate::read_supervision::Refusal> {
        manager.start_held(
            self.command("guardian", &selection),
            Instant::now() + WINDOW,
            self.clone(),
        )
    }
    fn command(&self, role: &str, selection: &ConfigurationSelection) -> Command {
        let mut command = Command::new(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            self.file.as_raw_fd()
        ));
        command
            .arg(role)
            .env_clear()
            .env(DIGEST_KEY, &self.metadata.sha256);
        for (key, path) in KEYS.into_iter().zip(&selection.paths) {
            command.env(key, path);
        }
        command
    }
}
fn seals() -> SealFlags {
    SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL
}
fn elf(header: &[u8; 64]) -> Result<()> {
    let machine = if cfg!(target_arch = "x86_64") {
        62
    } else if cfg!(target_arch = "aarch64") {
        183
    } else {
        return Err(Refusal::ImageFormat);
    };
    if &header[..4] != b"\x7fELF"
        || header[4] != 2
        || header[5] != 1
        || header[6] != 1
        || !matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3)
        || u16::from_le_bytes([header[18], header[19]]) != machine
        || u16::from_le_bytes([header[52], header[53]]) != 64
    {
        return Err(Refusal::ImageFormat);
    }
    Ok(())
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(64);
    for byte in bytes {
        write!(&mut text, "{byte:02x}").expect("string formatting");
    }
    text
}
fn identity(meta: &Metadata) -> (u64, u64, u32, u32, u32, u64, u64, i64, i64, i64, i64) {
    (
        meta.dev(),
        meta.ino(),
        meta.mode(),
        meta.uid(),
        meta.gid(),
        meta.nlink(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
    )
}

/// Only the fixed protected-root walk constructs this policy token.
/// ```compile_fail
/// use limeos_executor::read_launch::ProtectedLaunch;
/// let imported: ProtectedLaunch = serde_json::from_str("{}").unwrap();
/// ```
pub struct ProtectedLaunch {
    image: Arc<SealedExecutable>,
    selection: ConfigurationSelection,
    deadline: Instant,
}
impl ProtectedLaunch {
    pub fn prepare(paths: ConfigurationPaths<'_>) -> Result<Self> {
        let deadline = Instant::now() + WINDOW;
        let selection = ConfigurationSelection::new(paths)?;
        let root = File::from(
            rustix::fs::open(
                "/",
                OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| Refusal::Unavailable)?,
        );
        let image = protected_image(root, &IMAGE[1..], 0)?;
        if Instant::now() >= deadline {
            return Err(Refusal::Expired);
        }
        Ok(Self {
            image,
            selection,
            deadline,
        })
    }
    pub fn image(&self) -> &ImageMetadata {
        self.image.metadata()
    }
    pub fn selection(&self) -> &ConfigurationSelection {
        &self.selection
    }
    pub fn start(
        &self,
        manager: &Supervisor,
    ) -> std::result::Result<Ticket, crate::read_supervision::Refusal> {
        if Instant::now() >= self.deadline {
            return Err(crate::read_supervision::Refusal::Deadline);
        }
        manager.start_held(
            self.image.command("guardian", &self.selection),
            self.deadline,
            self.image.clone(),
        )
    }
}

/// Current sealed executable, never reopened from the manager's numeric PID.
pub struct LoadedRole {
    image: Arc<SealedExecutable>,
    selection: ConfigurationSelection,
}
impl LoadedRole {
    pub fn current() -> Result<Self> {
        let digest = std::env::var_os(DIGEST_KEY).ok_or(Refusal::UntrustedImage)?;
        let digest = digest
            .to_str()
            .filter(|text| {
                text.len() == 64
                    && text
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .ok_or(Refusal::UntrustedImage)?;
        let file = File::open("/proc/self/exe").map_err(|_| Refusal::Unavailable)?;
        if !rustix::fs::fcntl_get_seals(&file)
            .map_err(|_| Refusal::UntrustedImage)?
            .contains(seals())
        {
            return Err(Refusal::UntrustedImage);
        }
        let metadata = file.metadata().map_err(|_| Refusal::Unavailable)?;
        if !(64..=MAX_IMAGE).contains(&metadata.len()) {
            return Err(Refusal::ImageLimit);
        }
        let mut header = [0; 64];
        file.read_exact_at(&mut header, 0)
            .map_err(|_| Refusal::Unavailable)?;
        elf(&header)?;
        let mut reader = &file;
        let mut scratch = [0; 64 * 1024];
        let mut hash = Sha256::new();
        loop {
            let n = reader
                .read(&mut scratch)
                .map_err(|_| Refusal::Unavailable)?;
            if n == 0 {
                break;
            }
            hash.update(&scratch[..n]);
        }
        if hex(&hash.finalize()) != digest {
            return Err(Refusal::UntrustedImage);
        }
        Ok(Self {
            image: Arc::new(SealedExecutable {
                file,
                metadata: ImageMetadata {
                    bytes: metadata.len(),
                    sha256: digest.into(),
                },
            }),
            selection: ConfigurationSelection::from_environment()?,
        })
    }
    pub fn worker_command(&self) -> Command {
        self.image.command("worker", &self.selection)
    }
    pub fn paths(&self) -> ConfigurationPaths<'_> {
        self.selection.paths()
    }
}

fn protected_image(root: File, relative: &str, owner: u32) -> Result<Arc<SealedExecutable>> {
    let parts: Vec<_> = relative.split('/').collect();
    let mut held = vec![root];
    let mut directories = vec![held[0].metadata().map_err(|_| Refusal::Unavailable)?];
    check_directory(directories[0].clone(), owner)?;
    for part in &parts[..parts.len() - 1] {
        let file = File::from(
            rustix::fs::openat(
                held.last().unwrap(),
                *part,
                OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| Refusal::UntrustedImage)?,
        );
        let metadata = file.metadata().map_err(|_| Refusal::Unavailable)?;
        check_directory(metadata.clone(), owner)?;
        directories.push(metadata);
        held.push(file);
    }
    let parent = held.last().unwrap();
    let name = parts[parts.len() - 1];
    let classified = File::from(
        rustix::fs::openat(
            parent,
            name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Refusal::UntrustedImage)?,
    );
    let before = classified.metadata().map_err(|_| Refusal::Unavailable)?;
    if !before.is_file()
        || before.uid() != owner
        || before.nlink() != 1
        || before.mode() & 0o6022 != 0
        || before.mode() & 0o100 == 0
    {
        return Err(Refusal::UntrustedImage);
    }
    let file = File::from(
        rustix::fs::openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Refusal::UntrustedImage)?,
    );
    if identity(&file.metadata().map_err(|_| Refusal::Unavailable)?) != identity(&before) {
        return Err(Refusal::Changed);
    }
    let image = SealedExecutable::capture(&file)?;
    if identity(&classified.metadata().map_err(|_| Refusal::Unavailable)?) != identity(&before) {
        return Err(Refusal::Changed);
    }
    for (file, original) in held.iter().zip(&directories) {
        revalidate_directory(file, original, owner)?;
    }
    // Walk every selected name freshly; directory timestamps are irrelevant.
    let mut fresh = held[0].try_clone().map_err(|_| Refusal::Unavailable)?;
    for (index, part) in parts[..parts.len() - 1].iter().enumerate() {
        fresh = File::from(
            rustix::fs::openat(
                &fresh,
                *part,
                OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| Refusal::Changed)?,
        );
        revalidate_directory(&fresh, &directories[index + 1], owner)?;
    }
    let fresh = File::from(
        rustix::fs::openat(
            &fresh,
            name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Refusal::Changed)?,
    );
    if identity(&fresh.metadata().map_err(|_| Refusal::Unavailable)?) != identity(&before) {
        return Err(Refusal::Changed);
    }
    Ok(image)
}
fn revalidate_directory(file: &File, original: &Metadata, owner: u32) -> Result<()> {
    let now = file.metadata().map_err(|_| Refusal::Unavailable)?;
    check_directory(now.clone(), owner)?;
    if (now.dev(), now.ino(), now.mode(), now.uid(), now.gid())
        != (
            original.dev(),
            original.ino(),
            original.mode(),
            original.uid(),
            original.gid(),
        )
    {
        return Err(Refusal::Changed);
    }
    Ok(())
}
fn check_directory(meta: Metadata, owner: u32) -> Result<()> {
    if !meta.is_dir() || meta.uid() != owner || meta.mode() & 0o022 != 0 || meta.nlink() == 0 {
        Err(Refusal::UntrustedImage)
    } else {
        Ok(())
    }
}
