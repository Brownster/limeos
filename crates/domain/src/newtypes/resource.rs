//! Lock and grant resource ids.
use super::invalid;
use crate::{Error, Result};

/// Lock and grant spelling of a container: `container:<id>`.
pub const CONTAINER_RESOURCE_PREFIX: &str = "container:";
/// Lock and grant spelling of a Compose project: `stack:<project>`.
pub const STACK_RESOURCE_PREFIX: &str = "stack:";

/// The opaque-id rule: exactly 64 lowercase hexadecimal digits. Full Engine
/// container IDs, image digests, plan, action and job IDs, and SHA-256 digests.
/// Uppercase is refused: the Engine reports lowercase, and one spelling per
/// value keeps lock and grant comparisons exact.
pub fn opaque_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A full Engine container ID.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContainerId(String);
text_newtype!(ContainerId, opaque_id, "container id");
impl ContainerId {
    pub fn resource(&self) -> ContainerResource {
        ContainerResource(format!("{CONTAINER_RESOURCE_PREFIX}{}", self.0))
    }
}

/// The container-resource rule: `container:` then an [`opaque_id`], with the
/// prefix exactly once (`container:container:<id>` is refused).
pub fn container_resource(value: &str) -> bool {
    container_resource_id(value).is_some()
}
/// The container ID of a valid container resource.
pub fn container_resource_id(value: &str) -> Option<&str> {
    value
        .strip_prefix(CONTAINER_RESOURCE_PREFIX)
        .filter(|id| opaque_id(id))
}

/// A container's lock and grant resource, `container:<64 lowercase hex>`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContainerResource(String);
text_newtype!(ContainerResource, container_resource, "container resource");
impl ContainerResource {
    /// The container ID, without the prefix.
    pub fn id(&self) -> &str {
        &self.0[CONTAINER_RESOURCE_PREFIX.len()..]
    }
}

/// The Compose-project rule, as the Engine labels projects: 1 to 64 bytes of
/// lowercase letters, digits, `_` and `-`, beginning with a letter or digit.
pub fn stack_project(value: &str) -> bool {
    let first = value.as_bytes().first().copied();
    value.len() <= 64
        && first.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

/// A Compose project name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StackProject(String);
text_newtype!(StackProject, stack_project, "stack project");
impl StackProject {
    /// The project's lock and grant resource, `stack:<project>`.
    pub fn resource(&self) -> String {
        format!("{STACK_RESOURCE_PREFIX}{}", self.0)
    }
}

/// The project of a valid stack resource.
pub fn stack_resource_project(value: &str) -> Option<&str> {
    value
        .strip_prefix(STACK_RESOURCE_PREFIX)
        .filter(|project| stack_project(project))
}

#[cfg(test)]
mod tests;
