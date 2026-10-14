//! Retained selected management declarations, never permissions or effect authority.
use crate::{
    compose_files::{self, ComposeFileEvidence, ComposeFileSnapshot, Count, Validation},
    docker::compose_evidence::{
        ComposeOwnership, EngineComposeEvidence, EngineComposeSnapshot, inspect_compose_membership,
    },
};
use limeos_domain::{Error, ErrorCode, Result, compose_name};
use serde::Serialize;
use std::{
    path::Path,
    sync::atomic::{AtomicU8, Ordering},
    time::{Duration, Instant},
};
pub mod catalog;
use catalog::{
    ManagedFileExpectation, ManagementCatalogEvidence, ManagementCatalogSnapshot, StackRegistration,
};
const LIFETIME: Duration = Duration::from_secs(4);
const REPORT_BYTES: usize = 80 * 1024;
const RETAINED_FDS: usize = 38;
fn conflict() -> Error {
    Error(ErrorCode::Conflict)
}
fn unavailable() -> Error {
    Error(ErrorCode::Unavailable)
}

struct Window {
    observed_at: i64,
    expires: Instant,
}
impl Window {
    fn new() -> Result<Self> {
        Ok(Self {
            observed_at: compose_files::now()?,
            expires: Instant::now() + LIFETIME,
        })
    }
    fn poll(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        if cancelled() {
            return Err(conflict());
        }
        compose_files::age(self.observed_at, compose_files::now()?)?;
        if Instant::now() >= self.expires {
            return Err(Error(ErrorCode::Expired));
        }
        Ok(())
    }
}
/// One borrowed report preserves all original child observations; JSON creates no owner.
/// ```compile_fail
/// use limeos_executor_container::compose_management::ComposeBindingSnapshot;
/// let imported: ComposeBindingSnapshot<'_> = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Serialize)]
pub struct ComposeBindingSnapshot<'a> {
    pub version: u16,
    pub observed_at: i64,
    pub stack_resource: &'a str,
    pub registration: &'a StackRegistration,
    pub catalog: &'a ManagementCatalogSnapshot,
    pub files: &'a ComposeFileSnapshot,
    pub engine: &'a EngineComposeSnapshot,
    /// Selected full rows are indexed into the unchanged complete Engine membership.
    pub member_indices: &'a [usize],
}
/// Only genuine protected catalog/file and authenticated Engine owners can construct this.
/// ```compile_fail
/// use limeos_executor_container::compose_management::ComposeBindingEvidence;
/// let imported: ComposeBindingEvidence = serde_json::from_str("{}").unwrap();
/// ```
pub struct ComposeBindingEvidence {
    catalog: ManagementCatalogEvidence,
    files: ComposeFileEvidence,
    engine: EngineComposeEvidence,
    selected: usize,
    stack_resource: String,
    members: Vec<usize>,
    window: Window,
    state: AtomicU8,
}
impl ComposeBindingEvidence {
    pub fn snapshot(&self) -> ComposeBindingSnapshot<'_> {
        ComposeBindingSnapshot {
            version: 1,
            observed_at: self.window.observed_at,
            stack_resource: &self.stack_resource,
            registration: &self.catalog.catalog().stacks[self.selected],
            catalog: self.catalog.snapshot(),
            files: self.files.snapshot(),
            engine: self.engine.snapshot(),
            member_indices: &self.members,
        }
    }
    pub fn retained_descriptor_count(&self) -> usize {
        self.catalog.retained_descriptor_count() + self.files.retained_descriptor_count() + 1
    }
    pub async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.validate_with(
            cancelled,
            || self.catalog.revalidate(cancelled),
            || self.files.revalidate(cancelled),
        )
        .await
    }
    async fn validate_with(
        &self,
        cancelled: &dyn Fn() -> bool,
        check_catalog: impl Fn() -> Result<()>,
        check_files: impl Fn() -> Result<()>,
    ) -> Result<()> {
        if self
            .state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.state.store(2, Ordering::SeqCst);
            return Err(conflict());
        }
        let guard = Validation {
            state: &self.state,
            completed: false,
        };
        let result =
            tokio::time::timeout_at(tokio::time::Instant::from_std(self.window.expires), async {
                self.window.poll(cancelled)?;
                check_catalog()?;
                self.window.poll(cancelled)?;
                check_files()?;
                self.window.poll(cancelled)?;
                self.engine.revalidate().await?;
                self.window.poll(cancelled)?;
                if matched(
                    &self.catalog,
                    self.selected,
                    &self.files,
                    self.engine.snapshot(),
                )? != self.members
                {
                    return Err(conflict());
                }
                check_files()?;
                self.window.poll(cancelled)?;
                check_catalog()?;
                self.window.poll(cancelled)?;
                check_report(&self.snapshot(), REPORT_BYTES)
            })
            .await
            .map_err(|_| Error(ErrorCode::Expired))
            .and_then(|result| result);
        guard.finish(result)
    }
}
fn matched(
    catalog: &ManagementCatalogEvidence,
    selected: usize,
    files: &ComposeFileEvidence,
    engine: &EngineComposeSnapshot,
) -> Result<Vec<usize>> {
    let registration = catalog
        .catalog()
        .stacks
        .get(selected)
        .ok_or_else(conflict)?;
    let observation = files.snapshot();
    let selection = registration.selection()?;
    let expected_count = 1 + usize::from(selection.includes_managed());
    if engine.version != 1
        || engine.containers.len() > 16
        || engine.engine_id != catalog.catalog().engine_id
        || observation.root != catalog.snapshot().root
        || observation.mount_view != catalog.snapshot().mount_view
        || observation.selection.working_directory() != selection.working_directory()
        || observation.selection.operator() != selection.operator()
        || observation.selection.includes_managed() != selection.includes_managed()
        || observation.files.len() != expected_count
        || observation.files[0].sha256 != registration.operator.sha256
    {
        return Err(conflict());
    }
    if let ManagedFileExpectation::Present { sha256 } = &registration.managed
        && observation.files[1].sha256 != *sha256
    {
        return Err(conflict());
    }
    let mut indices = Vec::new();
    indices
        .try_reserve_exact(engine.containers.len())
        .map_err(|_| unavailable())?;
    for (index, member) in engine.containers.iter().enumerate() {
        let ComposeOwnership::Compose {
            project, service, ..
        } = &member.ownership
        else {
            continue;
        };
        if project.name == registration.project {
            if project.working_directory != registration.working_directory
                || project.config_files.len() != observation.files.len()
                || project
                    .config_files
                    .iter()
                    .zip(&observation.files)
                    .any(|(name, file)| *name != file.path)
                || !registration.services.contains(service)
            {
                return Err(conflict());
            }
            indices.push(index);
        } else if catalog::overlap(&project.working_directory, &registration.working_directory)
            || project
                .config_files
                .iter()
                .any(|name| catalog::overlap(name, &registration.working_directory))
        {
            return Err(conflict());
        }
    }
    Ok(indices)
}
pub(crate) fn check_report(snapshot: &ComposeBindingSnapshot<'_>, max: usize) -> Result<()> {
    serde_json::to_writer(&mut Count { bytes: 0, max }, snapshot).map_err(|_| unavailable())
}
fn assemble(
    catalog: ManagementCatalogEvidence,
    selected: usize,
    files: ComposeFileEvidence,
    engine: EngineComposeEvidence,
    window: Window,
) -> Result<ComposeBindingEvidence> {
    window.poll(&|| false)?;
    let members = matched(&catalog, selected, &files, engine.snapshot())?;
    let stack_resource = format!("stack:{}", catalog.catalog().stacks[selected].stack);
    let owner = ComposeBindingEvidence {
        catalog,
        selected,
        files,
        engine,
        window,
        stack_resource,
        members,
        state: AtomicU8::new(0),
    };
    if owner.retained_descriptor_count() > RETAINED_FDS {
        return Err(unavailable());
    }
    check_report(&owner.snapshot(), REPORT_BYTES)?;
    Ok(owner)
}
/// Unregistered read-only factory: select a stack ID, never child policy or file overrides.
pub async fn inspect_compose_binding(
    catalog_path: &Path,
    stack: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<ComposeBindingEvidence> {
    if !rustix::process::geteuid().is_root() {
        return Err(Error(ErrorCode::Forbidden));
    }
    if !compose_name(stack) {
        return Err(Error(ErrorCode::InvalidInput));
    }
    let window = Window::new()?;
    let deadline = tokio::time::Instant::from_std(window.expires);
    tokio::time::timeout_at(deadline, async {
        window.poll(cancelled)?;
        let catalog = catalog::inspect_management_catalog(catalog_path, cancelled)?;
        let selected = catalog.catalog().selected(stack)?;
        let selection = catalog.catalog().stacks[selected].selection()?;
        let file_fds = 3 + selection.working_directory().components().count() - 1
            + usize::from(selection.includes_managed());
        if catalog.retained_descriptor_count() + file_fds + 1 > RETAINED_FDS {
            return Err(unavailable());
        }
        window.poll(cancelled)?;
        let engine = inspect_compose_membership().await?;
        window.poll(cancelled)?;
        let files = compose_files::inspect_compose_files(&selection, cancelled)?;
        window.poll(cancelled)?;
        let owner = assemble(catalog, selected, files, engine, window)?;
        owner.revalidate(cancelled).await?;
        Ok(owner)
    })
    .await
    .map_err(|_| Error(ErrorCode::Expired))?
}

#[cfg(test)]
pub(crate) mod tests;
