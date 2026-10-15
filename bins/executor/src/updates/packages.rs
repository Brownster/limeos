//! Parse bounded read-only dpkg-query output; this module never starts a process.
use limeos_domain::deployment::packages::{Installed, Selection, package_name, version_valid};

pub const MAX_QUERY_BYTES: usize = 1024;
/// Fixed projection for a future protected single-package collector.
pub const DPKG_FORMAT: &str = "${Package}\t${Architecture}\t${db:Status-Want}\t${db:Status-Status}\t${db:Status-Eflag}\t${Version}\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Architecture {
    Amd64,
    Arm64,
}
impl Architecture {
    fn name(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
        }
    }
}
pub struct QueryOutput<'a> {
    pub exit_code: Option<i32>,
    pub stdout: &'a [u8],
    pub stderr: &'a [u8],
    pub truncated: bool,
}

/// Unknown is deliberate: process failure/no match never proves package absence.
/// Ordinary bytes and flags here are not protected observations or authority.
pub fn parse_dpkg(
    expected: &str,
    architecture: Architecture,
    output: QueryOutput<'_>,
) -> Installed {
    if !package_name(expected)
        || output.exit_code != Some(0)
        || output.truncated
        || output.stdout.len() > MAX_QUERY_BYTES
        || !output.stderr.is_empty()
    {
        return Installed::Unknown {};
    }
    let Ok(text) = std::str::from_utf8(output.stdout) else {
        return Installed::Unknown {};
    };
    let Some(line) = text.strip_suffix('\n') else {
        return Installed::Unknown {};
    };
    if line.contains(['\n', '\r', '\0']) {
        return Installed::Unknown {};
    }
    let fields: Vec<_> = line.split('\t').collect();
    if fields.len() != 6
        || fields[0] != expected
        || ![architecture.name(), "all"].contains(&fields[1])
    {
        return Installed::Unknown {};
    }
    let selection = match fields[2] {
        "install" => Selection::Install,
        "hold" => Selection::Hold,
        "deinstall" => Selection::Deinstall,
        "purge" => Selection::Purge,
        "unknown" => Selection::Unknown,
        _ => return Installed::Unknown {},
    };
    if fields[4] != "ok" {
        return if fields[4] == "reinstreq" {
            Installed::Broken {}
        } else {
            Installed::Unknown {}
        };
    }
    match fields[3] {
        "not-installed" if fields[5].is_empty() => Installed::Absent {},
        "config-files" if version_valid(fields[5]) => Installed::Absent {},
        "installed" if version_valid(fields[5]) => Installed::Present {
            version: fields[5].into(),
            selection,
        },
        "half-installed" | "unpacked" | "half-configured" | "triggers-awaited"
        | "triggers-pending" => Installed::Broken {},
        _ => Installed::Unknown {},
    }
}

#[cfg(test)]
mod tests;
