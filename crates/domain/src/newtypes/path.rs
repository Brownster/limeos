//! Absolute paths.
use super::invalid;
use crate::{Error, Result};

/// Longest absolute path any planner accepts, in bytes.
pub const MAX_PATH_BYTES: usize = 512;

/// The absolute-path rule: begins with `/`, at most [`MAX_PATH_BYTES`] bytes,
/// no control character, and lexically canonical: every component after the
/// root is non-empty and neither `.` nor `..` (so no `//`, no trailing `/`
/// except the root itself). `/` alone is valid; contexts that refuse the root
/// say so with [`AbsolutePath::is_root`].
///
/// Lexical only: it says nothing about symlinks or what is mounted where.
/// Live paths still need descriptor traversal before any effect.
pub fn absolute_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= MAX_PATH_BYTES
        && !value.chars().any(char::is_control)
        && (value == "/"
            || value[1..]
                .split('/')
                .all(|part| !matches!(part, "" | "." | "..")))
}

/// A path that satisfies [`absolute_path`].
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbsolutePath(String);
text_newtype!(AbsolutePath, absolute_path, "absolute path");
impl AbsolutePath {
    pub fn is_root(&self) -> bool {
        self.0 == "/"
    }
    /// Components after the root; none for `/`.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0[1..].split('/').filter(|part| !part.is_empty())
    }
    /// Strictly below `root` by whole components: `/mnt/a` is beneath `/mnt`,
    /// `/mnt` and `/mntx/a` are not. `root` is a canonical path; `/` contains
    /// every other path.
    pub fn is_beneath(&self, root: &str) -> bool {
        if root == "/" {
            return !self.is_root();
        }
        self.0
            .strip_prefix(root)
            .is_some_and(|rest| rest.len() > 1 && rest.starts_with('/'))
    }
}

/// A canonical absolute path other than `/`, optionally strictly beneath
/// `/mnt`: the shape of every storage location and mountpoint.
pub fn storage_path(value: &str, under_mnt: bool) -> bool {
    AbsolutePath::parse(value)
        .is_ok_and(|path| !path.is_root() && (!under_mnt || path.is_beneath("/mnt")))
}

#[cfg(test)]
mod tests;
