//! Protected management configuration, separate from P03 previews and approvals.
use crate::compose_files::{
    self as files, Ancestor, Budget, ComposeFileIdentity, ComposeFileSelection, ComposeMountView,
    ComposeObjectIdentity, Count, Root, Validation,
};
use limeos_domain::{ComposeFileName, Error, ErrorCode, Result, compose_name, opaque_id};
use rustix::{
    fd::OwnedFd,
    fs::{Mode, OFlags},
};
use serde::{
    Deserialize, Serialize,
    de::{self, DeserializeSeed, SeqAccess, Visitor},
};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    path::Path,
    sync::atomic::{AtomicU8, AtomicU64, Ordering},
    time::{Duration, Instant},
};

const FILE_BYTES: u64 = 16 * 1024;
const REPORT_BYTES: usize = 4 * 1024;
const LIFETIME: Duration = Duration::from_secs(4);
const WORK: Duration = Duration::from_secs(2);
fn invalid() -> Error {
    Error(ErrorCode::InvalidInput)
}
fn unavailable() -> Error {
    Error(ErrorCode::Unavailable)
}
fn conflict() -> Error {
    Error(ErrorCode::Conflict)
}

fn text<'de, D: de::Deserializer<'de>, const N: usize>(
    decoder: D,
) -> std::result::Result<String, D::Error> {
    struct Text<const N: usize>;
    impl<const N: usize> Visitor<'_> for Text<N> {
        type Value = String;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "text of at most {N} bytes")
        }
        fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<String, E> {
            if value.len() > N {
                return Err(E::custom("management text limit"));
            }
            let mut owned = String::new();
            owned
                .try_reserve_exact(value.len())
                .map_err(|_| E::custom("management allocation"))?;
            owned.push_str(value);
            Ok(owned)
        }
    }
    decoder.deserialize_str(Text::<N>)
}
fn name<'de, D: de::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    text::<D, 64>(d)
}
fn engine<'de, D: de::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    text::<D, 128>(d)
}
fn directory_name<'de, D: de::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    text::<D, 512>(d)
}
fn bounded<'de, D, T, const N: usize>(decoder: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: de::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct List<T, const N: usize>(std::marker::PhantomData<T>);
    struct Entry<T> {
        allowed: bool,
        _type: std::marker::PhantomData<T>,
    }
    impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Entry<T> {
        type Value = T;
        fn deserialize<D: de::Deserializer<'de>>(
            self,
            decoder: D,
        ) -> std::result::Result<T, D::Error> {
            if !self.allowed {
                return Err(de::Error::custom("management array limit"));
            }
            T::deserialize(decoder)
        }
    }
    impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for List<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "at most {N} entries")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Vec<T>, A::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = seq.next_element_seed(Entry {
                allowed: entries.len() < N,
                _type: std::marker::PhantomData,
            })? {
                entries
                    .try_reserve_exact(1)
                    .map_err(|_| de::Error::custom("management allocation"))?;
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    decoder.deserialize_seq(List::<T, N>(std::marker::PhantomData))
}
#[derive(Deserialize)]
struct Service(#[serde(deserialize_with = "name")] String);
fn services<'de, D: de::Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    Ok(bounded::<D, Service, 16>(d)?
        .into_iter()
        .map(|service| service.0)
        .collect())
}
fn registrations<'de, D: de::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<StackRegistration>, D::Error> {
    bounded::<D, StackRegistration, 8>(d)
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorFingerprint {
    pub name: ComposeFileName,
    #[serde(deserialize_with = "name")]
    pub sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedFileExpectation {
    Absent {},
    Present {
        #[serde(deserialize_with = "name")]
        sha256: String,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StackRegistration {
    #[serde(deserialize_with = "name")]
    pub stack: String,
    #[serde(deserialize_with = "name")]
    pub project: String,
    #[serde(deserialize_with = "directory_name")]
    pub working_directory: String,
    pub operator: OperatorFingerprint,
    pub managed: ManagedFileExpectation,
    #[serde(deserialize_with = "services")]
    pub services: Vec<String>,
}
impl StackRegistration {
    pub fn selection(&self) -> Result<ComposeFileSelection> {
        ComposeFileSelection::new(
            Path::new(&self.working_directory),
            self.operator.name,
            matches!(self.managed, ManagedFileExpectation::Present { .. }),
        )
    }
}
/// Plain declarations; importing this type does not create a protected owner.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementCatalog {
    pub version: u16,
    #[serde(deserialize_with = "engine")]
    pub engine_id: String,
    #[serde(deserialize_with = "registrations")]
    pub stacks: Vec<StackRegistration>,
}
pub(crate) fn overlap(a: &str, b: &str) -> bool {
    a == b
        || a.strip_prefix(b)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || b.strip_prefix(a)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
impl ManagementCatalog {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.engine_id.is_empty()
            || self.engine_id.len() > 128
            || !self
                .engine_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
            || !(1..=8).contains(&self.stacks.len())
        {
            return Err(invalid());
        }
        for (i, stack) in self.stacks.iter().enumerate() {
            if !compose_name(&stack.stack)
                || stack.project.is_empty()
                || stack.project.len() > 64
                || !stack.project.as_bytes()[0].is_ascii_alphanumeric()
                || !stack
                    .project
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
                || !opaque_id(&stack.operator.sha256)
                || !(1..=16).contains(&stack.services.len())
                || self.stacks[..i].iter().any(|other| {
                    other.stack == stack.stack
                        || other.project == stack.project
                        || overlap(&other.working_directory, &stack.working_directory)
                })
            {
                return Err(invalid());
            }
            if let ManagedFileExpectation::Present { sha256 } = &stack.managed
                && !opaque_id(sha256)
            {
                return Err(invalid());
            }
            stack.selection()?;
            for (j, service) in stack.services.iter().enumerate() {
                if !compose_name(service) || stack.services[..j].contains(service) {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
    pub(crate) fn selected(&self, stack: &str) -> Result<usize> {
        if !compose_name(stack) {
            return Err(invalid());
        }
        self.stacks
            .iter()
            .position(|registration| registration.stack == stack)
            .ok_or(Error(ErrorCode::NotFound))
    }
}

/// Past metadata cannot recreate the root, namespace, descriptors or original read ledger.
/// ```compile_fail
/// use limeos_executor_container::compose_management::catalog::ManagementCatalogSnapshot;
/// let imported: ManagementCatalogSnapshot = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, Serialize)]
pub struct ManagementCatalogSnapshot {
    pub version: u16,
    pub observed_at: i64,
    pub path: String,
    pub mount_view: Option<ComposeMountView>,
    pub root: ComposeObjectIdentity,
    pub ancestors: Vec<ComposeObjectIdentity>,
    pub identity: ComposeFileIdentity,
    pub sha256: String,
}
/// Root-controlled configuration is not a principal permission or human approval.
/// ```compile_fail
/// use limeos_executor_container::compose_management::catalog::ManagementCatalogEvidence;
/// let imported: ManagementCatalogEvidence = serde_json::from_str("{}").unwrap();
/// ```
pub struct ManagementCatalogEvidence {
    root: Root,
    ancestors: Vec<Ancestor>,
    name: String,
    fd: OwnedFd,
    catalog: ManagementCatalog,
    snapshot: ManagementCatalogSnapshot,
    budget: Budget,
    state: AtomicU8,
    owner: u32,
}
impl ManagementCatalogEvidence {
    pub fn catalog(&self) -> &ManagementCatalog {
        &self.catalog
    }
    pub fn snapshot(&self) -> &ManagementCatalogSnapshot {
        &self.snapshot
    }
    pub fn retained_descriptor_count(&self) -> usize {
        2 + usize::from(self.root.view.is_some()) + self.ancestors.len()
    }
    fn checked(&self, operation: impl FnOnce() -> Result<()>) -> Result<()> {
        if self
            .state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.state.store(2, Ordering::SeqCst);
            return Err(conflict());
        }
        Validation {
            state: &self.state,
            completed: false,
        }
        .finish(operation())
    }
    pub fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.checked(|| {
            let deadline = Instant::now() + WORK;
            self.budget.poll(deadline, cancelled)?;
            self.validate_from(&files::host_root()?, files::now()?, deadline, cancelled)?;
            self.check_root(&files::host_root()?)?;
            files::age(self.snapshot.observed_at, files::now()?)?;
            self.budget.poll(deadline, cancelled)
        })
    }
    fn check_root(&self, fresh: &Root) -> Result<()> {
        if files::directory(&self.root.fd, self.owner)? != self.snapshot.root
            || files::directory(&fresh.fd, self.owner)? != self.snapshot.root
            || self.root.view.as_ref().map(|v| &v.identity) != self.snapshot.mount_view.as_ref()
            || fresh.view.as_ref().map(|v| &v.identity) != self.snapshot.mount_view.as_ref()
        {
            return Err(conflict());
        }
        if let Some(view) = &self.root.view
            && files::view_identity(&view.fd)? != view.identity
        {
            return Err(conflict());
        }
        Ok(())
    }
    fn walk(
        &self,
        root: &Root,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<OwnedFd>> {
        // Return the final directory; only the previous/current fresh directory overlaps.
        let mut fresh = None;
        for ancestor in &self.ancestors {
            self.budget.poll(deadline, cancelled)?;
            let fd = files::open_directory(fresh.as_ref().unwrap_or(&root.fd), &ancestor.name)?;
            if files::directory(&ancestor.fd, self.owner)? != ancestor.identity
                || files::directory(&fd, self.owner)? != ancestor.identity
            {
                return Err(conflict());
            }
            fresh = Some(fd);
        }
        Ok(fresh)
    }
    fn validate_from(
        &self,
        root: &Root,
        at: i64,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        self.budget.poll(deadline, cancelled)?;
        files::age(self.snapshot.observed_at, at)?;
        self.check_root(root)?;
        let fresh = self.walk(root, deadline, cancelled)?;
        let parent = fresh.as_ref().unwrap_or(&root.fd);
        let named = files::classified(parent, &self.name)?.ok_or_else(conflict)?;
        if identity(&named, self.owner)? != self.snapshot.identity {
            return Err(conflict());
        }
        let read = open_read(parent, &self.name)?;
        if identity(&read, self.owner)? != self.snapshot.identity {
            return Err(conflict());
        }
        for fd in [&read, &self.fd] {
            let (id, hash) = files::hash_file_with_limit(
                fd,
                self.owner,
                &self.budget,
                deadline,
                cancelled,
                FILE_BYTES,
            )?;
            if id != self.snapshot.identity || hash != self.snapshot.sha256 {
                return Err(conflict());
            }
        }
        if identity(&named, self.owner)? != self.snapshot.identity {
            return Err(conflict());
        }
        drop(named);
        drop(read);
        let final_directory = self.walk(root, deadline, cancelled)?;
        let named = files::classified(final_directory.as_ref().unwrap_or(&root.fd), &self.name)?
            .ok_or_else(conflict)?;
        if identity(&named, self.owner)? != self.snapshot.identity
            || identity(&self.fd, self.owner)? != self.snapshot.identity
        {
            return Err(conflict());
        }
        self.budget.poll(deadline, cancelled)
    }
    #[cfg(test)]
    pub(crate) fn revalidate_local(&self, path: &Path, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.checked(|| {
            self.validate_from(
                &local_root(path),
                files::now()?,
                Instant::now() + WORK,
                cancelled,
            )
        })
    }
}
fn identity(fd: &OwnedFd, owner: u32) -> Result<ComposeFileIdentity> {
    let identity = files::file_identity(fd, owner)?;
    if identity.size > FILE_BYTES {
        return Err(unavailable());
    }
    Ok(identity)
}
fn open_read(parent: &OwnedFd, name: &str) -> Result<OwnedFd> {
    rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| unavailable())
}
fn admit(path: &Path) -> Result<Vec<&str>> {
    let text = path.to_str().ok_or_else(invalid)?;
    if !text.starts_with('/')
        || text.len() > 512
        || text.chars().any(char::is_control)
        || text.contains(',')
    {
        return Err(invalid());
    }
    let mut parts = Vec::new();
    for part in text[1..].split('/') {
        if parts.len() >= 16 || matches!(part, "" | "." | "..") || part.len() > 255 {
            return Err(invalid());
        }
        parts.try_reserve_exact(1).map_err(|_| unavailable())?;
        parts.push(part);
    }
    Ok(parts)
}
fn collect(
    root: Root,
    path: &Path,
    owner: u32,
    at: i64,
    budget: Budget,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<ManagementCatalogEvidence> {
    let parts = admit(path)?;
    budget.poll(deadline, cancelled)?;
    let root_identity = files::directory(&root.fd, owner)?;
    let mut ancestors = Vec::new();
    ancestors
        .try_reserve_exact(parts.len() - 1)
        .map_err(|_| unavailable())?;
    for name in &parts[..parts.len() - 1] {
        budget.poll(deadline, cancelled)?;
        let fd = files::open_directory(
            ancestors
                .last()
                .map(|a: &Ancestor| &a.fd)
                .unwrap_or(&root.fd),
            name,
        )?;
        let identity = files::directory(&fd, owner)?;
        ancestors.push(Ancestor {
            name: (*name).into(),
            fd,
            identity,
        });
    }
    let parent = ancestors.last().map(|a| &a.fd).unwrap_or(&root.fd);
    let name = parts.last().ok_or_else(invalid)?;
    let classified = files::classified(parent, name)?.ok_or_else(unavailable)?;
    let expected = identity(&classified, owner)?;
    let fd = open_read(parent, name)?;
    if identity(&fd, owner)? != expected {
        return Err(conflict());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(expected.size as usize + 1)
        .map_err(|_| unavailable())?;
    bytes.resize(expected.size as usize + 1, 0);
    let mut offset = 0;
    while offset < bytes.len() {
        budget.poll(deadline, cancelled)?;
        let end = bytes.len().min(offset + 4096);
        budget.charge(end - offset)?;
        let read = rustix::io::pread(&fd, &mut bytes[offset..end], offset as u64)
            .map_err(|_| unavailable())?;
        if read == 0 {
            break;
        }
        offset += read;
    }
    if offset as u64 != expected.size
        || identity(&fd, owner)? != expected
        || identity(&classified, owner)? != expected
    {
        return Err(conflict());
    }
    bytes.truncate(offset);
    let catalog: ManagementCatalog = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    catalog.validate()?;
    let snapshot = ManagementCatalogSnapshot {
        version: 1,
        observed_at: at,
        path: path.to_str().ok_or_else(invalid)?.into(),
        mount_view: root.view.as_ref().map(|v| v.identity.clone()),
        root: root_identity,
        ancestors: ancestors.iter().map(|a| a.identity.clone()).collect(),
        identity: expected,
        sha256: hex::encode(Sha256::digest(&bytes)),
    };
    serde_json::to_writer(
        &mut Count {
            bytes: 0,
            max: REPORT_BYTES,
        },
        &snapshot,
    )
    .map_err(|_| unavailable())?;
    drop(classified);
    let evidence = ManagementCatalogEvidence {
        root,
        ancestors,
        name: (*name).into(),
        fd,
        catalog,
        snapshot,
        budget,
        state: AtomicU8::new(0),
        owner,
    };
    if evidence.retained_descriptor_count() > 18 {
        return Err(unavailable());
    }
    evidence.budget.poll(deadline, cancelled)?;
    Ok(evidence)
}
/// Unregistered protected configuration reader with fixed bounds and no policy overrides.
pub fn inspect_management_catalog(
    path: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<ManagementCatalogEvidence> {
    if !rustix::process::geteuid().is_root() {
        return Err(Error(ErrorCode::Forbidden));
    }
    admit(path)?;
    let at = files::now()?;
    let budget = Budget {
        spent: AtomicU64::new(0),
        expires: Instant::now() + LIFETIME,
    };
    let deadline = Instant::now() + WORK;
    budget.poll(deadline, cancelled)?;
    let owner = collect(
        files::host_root()?,
        path,
        0,
        at,
        budget,
        deadline,
        cancelled,
    )?;
    owner.revalidate(cancelled)?;
    Ok(owner)
}
#[cfg(test)]
pub(crate) fn local_root(path: &Path) -> Root {
    Root {
        fd: rustix::fs::open(
            path,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap(),
        view: None,
    }
}
#[cfg(test)]
pub(crate) fn local_catalog(root: &Path, path: &Path) -> Result<ManagementCatalogEvidence> {
    let owner = collect(
        local_root(root),
        path,
        rustix::process::geteuid().as_raw(),
        files::now()?,
        Budget {
            spent: AtomicU64::new(0),
            expires: Instant::now() + LIFETIME,
        },
        Instant::now() + WORK,
        &|| false,
    )?;
    owner.revalidate_local(root, &|| false)?;
    Ok(owner)
}
#[cfg(test)]
mod tests;
