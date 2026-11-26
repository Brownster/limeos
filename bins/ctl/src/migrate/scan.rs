//! `migrate scan`: a read-only, bounded inventory of the frozen app's state.
//!
//! Directories are opened `O_NOFOLLOW|O_DIRECTORY` and entries are examined
//! relative to the open descriptor, so a swapped-in symlink is never followed.
//! Only regular files are opened; everything else is listed and refused.
use rustix::fs::{AtFlags, CWD, Dir, FileType, Mode, OFlags, Stat};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy)]
pub struct Limits {
    pub max_files: usize,
    pub max_depth: usize,
    pub max_dir_entries: usize,
    pub max_hash_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_files: 20_000,
            max_depth: 8,
            max_dir_entries: 5_000,
            max_hash_bytes: 256 << 20,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Imported,
    Deferred,
    Unsupported,
}

/// The closed classification table. Anything not named here is unsupported.
pub fn classify(relative: &Path) -> (Class, &'static str) {
    let name = relative
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let in_dir = |dir: &str| {
        relative
            .parent()
            .is_some_and(|p| p.components().any(|c| c.as_os_str() == dir))
    };
    if name == "credentials.env" {
        return (Class::Imported, "users: limeosctl migrate import-users");
    }
    if [
        ".sqlite3",
        ".sqlite3-wal",
        ".sqlite3-shm",
        ".sqlite3-journal",
    ]
    .iter()
    .any(|s| name.ends_with(s))
    {
        return (Class::Deferred, "SQLite history stays with the frozen app");
    }
    if in_dir("integrations") {
        return (
            Class::Deferred,
            "integration state stays with the frozen app",
        );
    }
    if in_dir("storage_plugins") {
        return (
            Class::Deferred,
            "storage plugin state stays with the frozen app",
        );
    }
    if name.contains(".backup.") || name.ends_with(".bak") {
        return (Class::Deferred, "policy backup stays with the frozen app");
    }
    if [
        "auto_update.json",
        "backup_config.json",
        "package-approvals.json",
        "pending-actions.json",
    ]
    .contains(&name.as_str())
    {
        return (Class::Deferred, "settings stay with the frozen app");
    }
    (Class::Unsupported, "no import rule")
}

/// Names that may hold secrets show only size and mode, never owner or hash.
fn redacted(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("secret") || name.contains("env")
}

#[derive(Serialize)]
pub struct FileEntry {
    pub location: String,
    pub path: String,
    pub kind: &'static str,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gid: Option<u32>,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub class: Class,
    pub reason: &'static str,
    /// For a `virtualenv` entry: how many entries under it were counted and not listed, at most
    /// `VIRTUALENV_COUNT_LIMIT` (then `entries_not_listed_bounded` is true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries_not_listed: Option<u64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub entries_not_listed_bounded: bool,
    #[serde(skip)]
    fingerprint: Fingerprint,
    #[serde(skip)]
    digest: Option<[u8; 32]>,
    #[serde(skip)]
    root: PathBuf,
    #[serde(skip)]
    relative: PathBuf,
}
#[derive(Serialize)]
pub struct Problem {
    pub path: String,
    pub error: String,
}
#[derive(Serialize)]
pub struct LocationReport {
    pub label: String,
    pub path: String,
    pub state: &'static str,
}
#[derive(Serialize, Default)]
pub struct Summary {
    pub files: usize,
    pub directories: usize,
    pub imported: usize,
    pub deferred: usize,
    pub unsupported: usize,
    pub bytes: u64,
    /// Virtual environments listed as one entry each, and the entries under them not listed.
    pub virtualenvs: usize,
    pub virtualenv_entries_not_listed: u64,
}
#[derive(Serialize)]
pub struct Verification {
    pub checked: usize,
    pub changed: Vec<String>,
}
#[derive(Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub complete: bool,
    pub locations: Vec<LocationReport>,
    pub files: Vec<FileEntry>,
    pub problems: Vec<Problem>,
    pub summary: Summary,
    pub verification: Verification,
}

#[derive(Clone, PartialEq, Eq, Default)]
struct Fingerprint {
    dev: u64,
    ino: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    size: u64,
    mtime: (i64, i128),
}
fn fingerprint(st: &Stat) -> Fingerprint {
    Fingerprint {
        dev: st.st_dev,
        ino: st.st_ino,
        mode: st.st_mode,
        uid: st.st_uid,
        gid: st.st_gid,
        size: st.st_size as u64,
        mtime: (st.st_mtime, i128::from(st.st_mtime_nsec)),
    }
}

pub struct Location {
    pub label: &'static str,
    pub display: PathBuf,
    pub root: PathBuf,
}

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

fn open_dir(parent: Option<&OwnedFd>, name: &Path) -> rustix::io::Result<OwnedFd> {
    match parent {
        Some(fd) => rustix::fs::openat(fd, name, DIR_FLAGS, Mode::empty()),
        None => rustix::fs::openat(CWD, name, DIR_FLAGS, Mode::empty()),
    }
}
/// Open a regular file without following a final symlink, without blocking on
/// a FIFO swapped in after the lstat, and without updating atime when allowed.
fn open_file(dir: &OwnedFd, name: &OsStr) -> rustix::io::Result<OwnedFd> {
    let base = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    rustix::fs::openat(dir, name, base | OFlags::NOATIME, Mode::empty())
        .or_else(|_| rustix::fs::openat(dir, name, base, Mode::empty()))
}
fn digest_of(fd: OwnedFd, expected: &Fingerprint, cap: u64) -> Result<Option<[u8; 32]>, String> {
    let st = rustix::fs::fstat(&fd).map_err(|e| e.to_string())?;
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile
        || fingerprint(&st).ino != expected.ino
    {
        return Err("changed while scanning".into());
    }
    if st.st_size as u64 > cap {
        return Ok(None);
    }
    let mut file = std::fs::File::from(fd);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > cap {
            return Ok(None);
        }
        hasher.update(&buffer[..n]);
    }
    Ok(Some(hasher.finalize().into()))
}

const VIRTUALENV_REASON: &str = "Python virtual environment (pyvenv.cfg with a home line, and a Python under bin/ or a lib/python*/site-packages): software, not state; one entry, its entries counted, not listed";
/// A virtual environment's entries are counted up to this many, and at most this deep.
pub const VIRTUALENV_COUNT_LIMIT: u64 = 200_000;
const VIRTUALENV_COUNT_DEPTH: usize = 64;

/// A directory is a Python virtual environment (PEP 405) when it holds a regular `pyvenv.cfg` whose
/// first 4 KiB have a `home = ...` line, and either a `bin/python*` entry or a
/// `lib/python*/site-packages` directory. A state directory that merely holds a file of that name is
/// walked like any other. Everything is examined relative to the open directory, never following a
/// link.
fn is_virtualenv(dir: &OwnedFd) -> bool {
    let Ok(fd) = open_file(dir, OsStr::new("pyvenv.cfg")) else {
        return false;
    };
    let Ok(st) = rustix::fs::fstat(&fd) else {
        return false;
    };
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
        return false;
    }
    let mut head = Vec::new();
    if std::fs::File::from(fd)
        .take(4096)
        .read_to_end(&mut head)
        .is_err()
    {
        return false;
    }
    let has_home = String::from_utf8_lossy(&head).lines().any(|line| {
        line.split_once('=')
            .is_some_and(|(key, value)| key.trim() == "home" && !value.trim().is_empty())
    });
    has_home && (has_python(dir) || has_site_packages(dir))
}
fn names_in(dir: &OwnedFd, sub: &str) -> Vec<std::ffi::OsString> {
    let Ok(fd) = open_dir(Some(dir), Path::new(sub)) else {
        return Vec::new();
    };
    let Ok(entries) = Dir::read_from(&fd) else {
        return Vec::new();
    };
    entries
        .flatten()
        .take(4096)
        .map(|e| OsStr::from_bytes(e.file_name().to_bytes()).to_owned())
        .collect()
}
fn has_python(dir: &OwnedFd) -> bool {
    names_in(dir, "bin")
        .iter()
        .any(|n| n.as_bytes().starts_with(b"python"))
}
fn has_site_packages(dir: &OwnedFd) -> bool {
    let Ok(lib) = open_dir(Some(dir), Path::new("lib")) else {
        return false;
    };
    names_in(dir, "lib")
        .iter()
        .filter(|n| n.as_bytes().starts_with(b"python"))
        .any(|n| {
            open_dir(Some(&lib), Path::new(n))
                .and_then(|py| open_dir(Some(&py), Path::new("site-packages")))
                .is_ok()
        })
}
/// Entries under DIR (files, links and directories), counted without following a link, stopping
/// at `VIRTUALENV_COUNT_LIMIT` entries or `VIRTUALENV_COUNT_DEPTH` levels; true when bounded.
fn count_entries(dir: &OwnedFd, depth: usize, total: &mut u64) -> bool {
    let Ok(entries) = Dir::read_from(dir) else {
        return false;
    };
    let mut bounded = false;
    for entry in entries.flatten() {
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        if *total >= VIRTUALENV_COUNT_LIMIT {
            return true;
        }
        *total += 1;
        let name = OsStr::from_bytes(name);
        if let Ok(st) = rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)
            && FileType::from_raw_mode(st.st_mode) == FileType::Directory
        {
            if depth >= VIRTUALENV_COUNT_DEPTH {
                bounded = true;
                continue;
            }
            if let Ok(sub) = open_dir(Some(dir), Path::new(name)) {
                bounded |= count_entries(&sub, depth + 1, total);
            }
        }
    }
    bounded
}

struct Walk<'a> {
    limits: Limits,
    location: &'a Location,
    files: Vec<FileEntry>,
    problems: Vec<Problem>,
    directories: usize,
    truncated: bool,
}
impl Walk<'_> {
    fn display(&self, relative: &Path) -> String {
        relative.to_string_lossy().into_owned()
    }
    fn problem(&mut self, relative: &Path, error: impl std::fmt::Display) {
        self.problems.push(Problem {
            path: format!("{}/{}", self.location.label, self.display(relative)),
            error: error.to_string(),
        });
    }
    fn walk(&mut self, dir: &OwnedFd, relative: &Path, depth: usize, total: &mut usize) {
        self.directories += 1;
        let mut names = Vec::new();
        match Dir::read_from(dir) {
            Ok(entries) => {
                for entry in entries {
                    let Ok(entry) = entry else {
                        self.problem(relative, "directory read failed");
                        self.truncated = true;
                        return;
                    };
                    let name = entry.file_name().to_bytes();
                    if name == b"." || name == b".." {
                        continue;
                    }
                    names.push(OsStr::from_bytes(name).to_owned());
                    if names.len() > self.limits.max_dir_entries {
                        self.problem(relative, "directory entry limit exceeded; not listed");
                        self.truncated = true;
                        return;
                    }
                }
            }
            Err(e) => {
                self.problem(relative, format!("unreadable directory: {e}"));
                return;
            }
        }
        names.sort();
        for name in names {
            if *total >= self.limits.max_files {
                self.truncated = true;
                return;
            }
            let child = relative.join(&name);
            let st = match rustix::fs::statat(dir, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
                Ok(st) => st,
                Err(e) => {
                    self.problem(&child, format!("stat failed: {e}"));
                    continue;
                }
            };
            match FileType::from_raw_mode(st.st_mode) {
                FileType::Directory => match open_dir(Some(dir), Path::new(&name)) {
                    // A Python virtual environment is software the frozen app installed (wybie's
                    // agent supervisor has ~1,100 files in one, deeper than the depth bound), not
                    // state: one entry, never descended, whatever its depth.
                    Ok(fd) if is_virtualenv(&fd) => {
                        *total += 1;
                        let mut counted = 0;
                        let bounded = count_entries(&fd, 0, &mut counted);
                        self.virtualenv(&child, &st, counted, bounded);
                    }
                    Ok(_) if depth >= self.limits.max_depth => {
                        self.problem(&child, "depth limit exceeded; not listed");
                        self.truncated = true;
                    }
                    Ok(fd) => self.walk(&fd, &child, depth + 1, total),
                    Err(e) => self.problem(&child, format!("unreadable directory: {e}")),
                },
                kind => {
                    *total += 1;
                    self.file(dir, &name, &child, &st, kind);
                }
            }
        }
    }
    fn virtualenv(&mut self, child: &Path, st: &Stat, counted: u64, bounded: bool) {
        let fp = fingerprint(st);
        let secret = child
            .file_name()
            .is_some_and(|n| redacted(&n.to_string_lossy()));
        self.files.push(FileEntry {
            location: self.location.label.into(),
            path: self.display(child),
            kind: "virtualenv",
            size: 0,
            uid: (!secret).then_some(fp.uid),
            gid: (!secret).then_some(fp.gid),
            mode: format!("{:04o}", fp.mode & 0o7777),
            sha256: None,
            class: Class::Unsupported,
            reason: VIRTUALENV_REASON,
            entries_not_listed: Some(counted),
            entries_not_listed_bounded: bounded,
            fingerprint: fp,
            digest: None,
            root: self.location.root.clone(),
            relative: child.to_owned(),
        });
    }
    fn file(&mut self, dir: &OwnedFd, name: &OsStr, child: &Path, st: &Stat, kind: FileType) {
        let fp = fingerprint(st);
        let (class, reason) = classify(child);
        let regular = kind == FileType::RegularFile;
        let (kind_name, class, reason) = if regular {
            ("file", class, reason)
        } else {
            (
                match kind {
                    FileType::Symlink => "symlink",
                    FileType::Fifo => "fifo",
                    FileType::Socket => "socket",
                    FileType::BlockDevice => "block_device",
                    FileType::CharacterDevice => "char_device",
                    _ => "other",
                },
                Class::Unsupported,
                "non-regular file refused; not opened",
            )
        };
        let mut digest = None;
        if regular {
            match open_file(dir, name)
                .map_err(|e| e.to_string())
                .and_then(|fd| digest_of(fd, &fp, self.limits.max_hash_bytes))
            {
                Ok(d) => digest = d,
                Err(e) => self.problem(child, format!("unreadable file: {e}")),
            }
        }
        let secret = redacted(&name.to_string_lossy());
        self.files.push(FileEntry {
            location: self.location.label.into(),
            path: self.display(child),
            kind: kind_name,
            size: fp.size,
            uid: (!secret).then_some(fp.uid),
            gid: (!secret).then_some(fp.gid),
            mode: format!("{:04o}", fp.mode & 0o7777),
            sha256: digest.filter(|_| !secret).map(hex::encode),
            class,
            reason,
            entries_not_listed: None,
            entries_not_listed_bounded: false,
            fingerprint: fp,
            digest,
            root: self.location.root.clone(),
            relative: child.to_owned(),
        });
    }
}

/// Re-examine one recorded entry the way the walk did.
fn recheck(entry: &FileEntry, cap: u64) -> Result<(), String> {
    let mut dir = open_dir(None, &entry.root).map_err(|e| e.to_string())?;
    let mut parts: Vec<_> = entry.relative.components().collect();
    let last = parts.pop().ok_or("empty path")?;
    for part in parts {
        dir = open_dir(Some(&dir), Path::new(part.as_os_str())).map_err(|e| e.to_string())?;
    }
    let st = rustix::fs::statat(&dir, last.as_os_str(), AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| e.to_string())?;
    if fingerprint(&st) != entry.fingerprint {
        return Err("metadata changed".into());
    }
    if entry.kind == "file" {
        let fd = open_file(&dir, last.as_os_str()).map_err(|e| e.to_string())?;
        if digest_of(fd, &entry.fingerprint, cap)? != entry.digest {
            return Err("content changed".into());
        }
    }
    Ok(())
}

pub fn scan(locations: &[Location], limits: Limits) -> Report {
    let mut report = Report {
        schema_version: SCHEMA_VERSION,
        complete: true,
        locations: Vec::new(),
        files: Vec::new(),
        problems: Vec::new(),
        summary: Summary::default(),
        verification: Verification {
            checked: 0,
            changed: Vec::new(),
        },
    };
    let mut total = 0;
    for location in locations {
        let mut state = "scanned";
        let mut walk = Walk {
            limits,
            location,
            files: Vec::new(),
            problems: Vec::new(),
            directories: 0,
            truncated: false,
        };
        match open_dir(None, &location.root) {
            Ok(fd) => walk.walk(&fd, Path::new(""), 0, &mut total),
            Err(rustix::io::Errno::NOENT) => state = "absent",
            Err(e) => {
                state = "unreadable";
                walk.problem(Path::new(""), format!("cannot open location: {e}"));
            }
        }
        if walk.truncated || state == "unreadable" || !walk.problems.is_empty() {
            report.complete = false;
        }
        report.summary.directories += walk.directories;
        report.files.append(&mut walk.files);
        report.problems.append(&mut walk.problems);
        report.locations.push(LocationReport {
            label: location.label.into(),
            path: location.display.to_string_lossy().into_owned(),
            state,
        });
    }
    for entry in &report.files {
        report.summary.files += 1;
        if let Some(n) = entry.entries_not_listed {
            report.summary.virtualenvs += 1;
            report.summary.virtualenv_entries_not_listed += n;
        }
        report.summary.bytes += entry.size;
        match entry.class {
            Class::Imported => report.summary.imported += 1,
            Class::Deferred => report.summary.deferred += 1,
            Class::Unsupported => report.summary.unsupported += 1,
        }
    }
    for entry in &report.files {
        report.verification.checked += 1;
        if recheck(entry, limits.max_hash_bytes).is_err() {
            report
                .verification
                .changed
                .push(format!("{}/{}", entry.location, entry.path));
        }
    }
    report
}
impl Report {
    pub fn render_text(&self) -> String {
        let mut out = format!(
            "limeosctl migrate scan (report schema {})\n",
            self.schema_version
        );
        for l in &self.locations {
            out += &format!("location {} {} {}\n", l.label, l.path, l.state);
        }
        for f in &self.files {
            out += &format!(
                "{:11} {}/{} {} size={}",
                format!("[{:?}]", f.class).to_lowercase(),
                f.location,
                f.path,
                f.kind,
                f.size
            );
            if let (Some(u), Some(g)) = (f.uid, f.gid) {
                out += &format!(" owner={u}:{g}");
            }
            out += &format!(" mode={}", f.mode);
            if let Some(n) = f.entries_not_listed {
                let at_least = if f.entries_not_listed_bounded {
                    "at least "
                } else {
                    ""
                };
                out += &format!(" entries_not_listed={at_least}{n}");
            }
            if let Some(h) = &f.sha256 {
                out += &format!(" sha256={h}");
            }
            out += &format!(" ({})\n", f.reason);
        }
        for p in &self.problems {
            out += &format!("problem {}: {}\n", p.path, p.error);
        }
        let s = &self.summary;
        out += &format!(
            "summary: {} files in {} directories, {} bytes; imported={} deferred={} unsupported={}\n",
            s.files, s.directories, s.bytes, s.imported, s.deferred, s.unsupported
        );
        if s.virtualenvs > 0 {
            out += &format!(
                "virtual environments: {} listed as one entry each; {} entries under them counted and not listed\n",
                s.virtualenvs, s.virtualenv_entries_not_listed
            );
        }
        out += &format!(
            "verification: {} checked, {} changed; complete={}\n",
            self.verification.checked,
            self.verification.changed.len(),
            self.complete
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn location(dir: &Path, label: &'static str) -> Location {
        Location {
            label,
            display: PathBuf::from("/").join(label),
            root: dir.join(label),
        }
    }
    fn fixture() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let etc = t.path().join("etc");
        std::fs::create_dir_all(etc.join("integrations")).unwrap();
        std::fs::write(
            etc.join("credentials.env"),
            "PIHEALTH_USERS=holly:scrypt:SECRETHASH\n",
        )
        .unwrap();
        std::fs::set_permissions(
            etc.join("credentials.env"),
            std::fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        std::fs::write(etc.join("integrations/mattermost.env"), "TOKEN=hunter2").unwrap();
        std::fs::write(etc.join("integrations/package-updates.json"), "{}").unwrap();
        std::fs::write(etc.join("agent-policy.json"), "{}").unwrap();
        std::fs::write(etc.join("agent-policy.json.backup.20260101_000000"), "{}").unwrap();
        let state = t.path().join("state");
        std::fs::create_dir_all(state.join("agent-actions")).unwrap();
        std::fs::write(state.join("metrics.sqlite3"), b"SQLite").unwrap();
        std::fs::write(state.join("agent-actions/actions.sqlite3"), b"SQLite").unwrap();
        for n in [
            "auto_update.json",
            "backup_config.json",
            "package-approvals.json",
            "pending-actions.json",
        ] {
            std::fs::write(state.join(n), "{}").unwrap();
        }
        t
    }
    fn class_of(report: &Report, path: &str) -> Class {
        report.files.iter().find(|f| f.path == path).unwrap().class
    }

    #[test]
    fn closed_table_classifies_every_known_source_and_defaults_to_unsupported() {
        let t = fixture();
        let r = scan(
            &[
                location(t.path(), "etc"),
                location(t.path(), "state"),
                location(t.path(), "missing"),
            ],
            Limits::default(),
        );
        assert!(r.complete);
        assert_eq!(class_of(&r, "credentials.env"), Class::Imported);
        for p in [
            "integrations/mattermost.env",
            "integrations/package-updates.json",
            "agent-policy.json.backup.20260101_000000",
        ] {
            assert_eq!(class_of(&r, p), Class::Deferred, "{p}");
        }
        assert_eq!(class_of(&r, "agent-policy.json"), Class::Unsupported);
        for p in [
            "metrics.sqlite3",
            "agent-actions/actions.sqlite3",
            "auto_update.json",
            "backup_config.json",
            "package-approvals.json",
            "pending-actions.json",
        ] {
            assert_eq!(class_of(&r, p), Class::Deferred, "{p}");
        }
        assert_eq!(r.locations[2].state, "absent");
        assert_eq!(
            (
                r.summary.imported,
                r.summary.deferred,
                r.summary.unsupported
            ),
            (1, 9, 1)
        );
        assert!(r.verification.changed.is_empty());
        assert_eq!(r.verification.checked, r.summary.files);
    }

    #[test]
    fn secret_named_files_show_size_and_mode_only_and_nothing_leaks() {
        let t = fixture();
        let r = scan(&[location(t.path(), "etc")], Limits::default());
        let json = serde_json::to_string(&r).unwrap();
        let text = r.render_text();
        for out in [&json, &text] {
            assert!(!out.contains("SECRETHASH") && !out.contains("hunter2"));
        }
        let cred = r
            .files
            .iter()
            .find(|f| f.path == "credentials.env")
            .unwrap();
        assert_eq!(
            (cred.mode.as_str(), cred.uid, cred.gid, cred.sha256.clone()),
            ("0640", None, None, None)
        );
        let plain = r
            .files
            .iter()
            .find(|f| f.path == "agent-policy.json")
            .unwrap();
        assert_eq!(
            plain.sha256.as_deref(),
            Some(hex::encode(Sha256::digest(b"{}")).as_str())
        );
        assert!(json.contains("\"schema_version\":1"));
    }

    #[test]
    fn symlinks_and_fifos_are_listed_refused_and_never_opened_or_followed() {
        let t = fixture();
        let etc = t.path().join("etc");
        let outside = t.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("leak.json"), "{}").unwrap();
        symlink(&outside, etc.join("linkdir")).unwrap();
        symlink(etc.join("agent-policy.json"), etc.join("linkfile")).unwrap();
        rustix::fs::mknodat(
            CWD,
            etc.join("pipe"),
            FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        let r = scan(&[location(t.path(), "etc")], Limits::default());
        let kind = |p: &str| r.files.iter().find(|f| f.path == p).unwrap();
        assert_eq!(kind("linkdir").kind, "symlink");
        assert_eq!(kind("linkfile").kind, "symlink");
        assert_eq!(kind("pipe").kind, "fifo");
        assert!(
            ["linkdir", "linkfile", "pipe"]
                .iter()
                .all(|p| kind(p).class == Class::Unsupported && kind(p).sha256.is_none())
        );
        assert!(r.files.iter().all(|f| !f.path.contains("leak.json")));
        assert!(r.verification.changed.is_empty());
    }

    #[test]
    fn oversize_files_are_listed_without_a_hash_and_still_verified_by_metadata() {
        let t = fixture();
        std::fs::write(t.path().join("etc/big.bin"), vec![7u8; 4096]).unwrap();
        let limits = Limits {
            max_hash_bytes: 1024,
            ..Limits::default()
        };
        let r = scan(&[location(t.path(), "etc")], limits);
        let big = r.files.iter().find(|f| f.path == "big.bin").unwrap();
        assert_eq!((big.size, big.sha256.is_none()), (4096, true));
        assert!(r.verification.changed.is_empty());
    }

    #[test]
    fn unreadable_directories_make_the_report_incomplete_but_do_not_stop_it() {
        if rustix::process::geteuid().is_root() {
            return;
        }
        let t = fixture();
        let locked = t.path().join("etc/locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::write(locked.join("x.json"), "{}").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let r = scan(&[location(t.path(), "etc")], Limits::default());
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!r.complete);
        assert!(
            r.problems
                .iter()
                .any(|p| p.path == "etc/locked" && p.error.contains("unreadable"))
        );
        assert!(r.files.iter().any(|f| f.path == "credentials.env"));
    }

    #[test]
    fn huge_directories_wide_trees_and_deep_trees_are_bounded() {
        let t = tempfile::tempdir().unwrap();
        let etc = t.path().join("etc");
        std::fs::create_dir_all(etc.join("wide")).unwrap();
        for i in 0..50 {
            std::fs::write(etc.join(format!("wide/f{i:03}")), "x").unwrap();
        }
        let limits = Limits {
            max_dir_entries: 10,
            ..Limits::default()
        };
        let r = scan(&[location(t.path(), "etc")], limits);
        assert!(!r.complete && r.files.is_empty());
        assert!(r.problems.iter().any(|p| p.error.contains("entry limit")));

        let limits = Limits {
            max_files: 20,
            ..Limits::default()
        };
        let r = scan(&[location(t.path(), "etc")], limits);
        assert!(!r.complete && r.files.len() == 20);

        let mut deep = etc.join("d");
        for _ in 0..12 {
            deep = deep.join("d");
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep.json"), "{}").unwrap();
        let limits = Limits {
            max_depth: 3,
            ..Limits::default()
        };
        let r = scan(&[location(t.path(), "etc")], limits);
        assert!(
            !r.complete
                && r.files
                    .iter()
                    .all(|f| f.path != "d/d/d/d/d/d/d/d/d/d/d/d/d/deep.json")
        );
        assert!(r.problems.iter().any(|p| p.error.contains("depth limit")));
    }

    #[test]
    fn a_virtualenv_is_one_entry_never_descended_and_keeps_the_report_complete() {
        // wybie: state/agent-supervisor/venv, ~1,100 files, 74 directories past the depth bound.
        let t = fixture();
        let venv = t.path().join("state/agent-supervisor/venv");
        let mut deep = venv.join("lib/python3.11/site-packages/pip/_internal/cli");
        for _ in 0..6 {
            deep = deep.join("__pycache__");
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("main.cpython-311.pyc"), b"pyc").unwrap();
        std::fs::write(venv.join("pyvenv.cfg"), "home = /usr/bin\n").unwrap();
        std::fs::write(t.path().join("state/agent-supervisor/.profile"), "x").unwrap();
        // A directory named like one but without pyvenv.cfg is still walked and bounded.
        let fake = t.path().join("state/lookalike/a/b/c/d/e/f/g/h/i");
        std::fs::create_dir_all(&fake).unwrap();
        std::fs::write(fake.join("deep.json"), "{}").unwrap();
        let r = scan(&[location(t.path(), "state")], Limits::default());
        let entries: Vec<_> = r
            .files
            .iter()
            .filter(|f| f.path.starts_with("agent-supervisor/venv"))
            .collect();
        assert_eq!(entries.len(), 1);
        let venv_entry = entries[0];
        assert_eq!(
            (
                venv_entry.path.as_str(),
                venv_entry.kind,
                venv_entry.class,
                venv_entry.sha256.is_none()
            ),
            (
                "agent-supervisor/venv",
                "virtualenv",
                Class::Unsupported,
                true
            )
        );
        assert!(venv_entry.reason.contains("not listed"));
        // lib, python3.11, site-packages, pip, _internal, cli, six __pycache__, the .pyc, pyvenv.cfg.
        assert_eq!(
            (
                venv_entry.entries_not_listed,
                venv_entry.entries_not_listed_bounded
            ),
            (Some(14), false)
        );
        assert_eq!(
            (
                r.summary.virtualenvs,
                r.summary.virtualenv_entries_not_listed
            ),
            (1, 14)
        );
        assert!(
            r.files
                .iter()
                .any(|f| f.path == "agent-supervisor/.profile")
        );
        assert!(r.problems.iter().all(|p| !p.path.contains("venv")));
        // Only the walked look-alike hits the depth bound.
        assert!(!r.complete);
        assert!(
            r.problems
                .iter()
                .any(|p| p.path.starts_with("state/lookalike/") && p.error.contains("depth limit"))
        );
        assert!(r.verification.changed.is_empty());
        std::fs::remove_dir_all(t.path().join("state/lookalike")).unwrap();
        let r = scan(&[location(t.path(), "state")], Limits::default());
        assert!(r.complete && r.problems.is_empty(), "{:?}", r.render_text());
        let text = r.render_text();
        assert!(text.contains("state/agent-supervisor/venv virtualenv"));
        assert!(text.contains("entries_not_listed=14"));
        assert!(text.contains("virtual environments: 1 listed as one entry each; 14 entries"));
    }

    #[test]
    fn a_directory_that_merely_holds_a_file_named_pyvenv_cfg_is_walked() {
        let t = fixture();
        let state = t.path().join("state");
        // No home line; a home line but no Python and no site-packages; a home line and a bin/ with
        // no python: all are state directories, listed file by file.
        for (dir, cfg, extra) in [
            ("plain", "this is a settings file\n", None),
            ("homeonly", "home = /usr/bin\n", None),
            ("binnopy", "home = /usr/bin\n", Some("bin/tool")),
        ] {
            std::fs::create_dir_all(state.join(dir)).unwrap();
            std::fs::write(state.join(dir).join("pyvenv.cfg"), cfg).unwrap();
            std::fs::write(state.join(dir).join("settings.json"), "{}").unwrap();
            if let Some(extra) = extra {
                std::fs::create_dir_all(state.join(dir).join(extra).parent().unwrap()).unwrap();
                std::fs::write(state.join(dir).join(extra), "#!/bin/sh\n").unwrap();
            }
        }
        let r = scan(&[location(t.path(), "state")], Limits::default());
        for dir in ["plain", "homeonly", "binnopy"] {
            assert!(
                r.files
                    .iter()
                    .any(|f| f.path == format!("{dir}/settings.json") && f.kind == "file"),
                "{dir}"
            );
            assert!(
                r.files
                    .iter()
                    .any(|f| f.path == format!("{dir}/pyvenv.cfg") && f.kind == "file"),
                "{dir}"
            );
        }
        assert!(r.files.iter().all(|f| f.kind != "virtualenv"));
        assert_eq!(r.summary.virtualenvs, 0);
        // With a Python under bin/ it is one.
        std::fs::write(state.join("binnopy/bin/python3"), "").unwrap();
        let r = scan(&[location(t.path(), "state")], Limits::default());
        assert!(
            r.files
                .iter()
                .any(|f| f.path == "binnopy" && f.kind == "virtualenv")
        );
    }

    #[test]
    fn a_changed_original_fails_verification() {
        let t = fixture();
        let r = scan(&[location(t.path(), "etc")], Limits::default());
        let entry = r
            .files
            .iter()
            .find(|f| f.path == "agent-policy.json")
            .unwrap();
        std::fs::write(t.path().join("etc/agent-policy.json"), "{ }").unwrap();
        assert!(recheck(entry, Limits::default().max_hash_bytes).is_err());
        let other = r
            .files
            .iter()
            .find(|f| f.path == "credentials.env")
            .unwrap();
        assert!(recheck(other, Limits::default().max_hash_bytes).is_ok());
    }

    #[test]
    fn scanning_leaves_every_original_byte_for_byte_unchanged() {
        let t = fixture();
        let snapshot = |root: &Path| -> Vec<(PathBuf, Vec<u8>, u32)> {
            let mut out = Vec::new();
            let mut stack = vec![root.to_owned()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(d).unwrap() {
                    let p = e.unwrap().path();
                    if p.is_dir() {
                        stack.push(p);
                    } else {
                        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
                        out.push((p.clone(), std::fs::read(&p).unwrap(), mode));
                    }
                }
            }
            out.sort();
            out
        };
        let before = snapshot(t.path());
        let r = scan(
            &[location(t.path(), "etc"), location(t.path(), "state")],
            Limits::default(),
        );
        assert!(r.verification.changed.is_empty());
        assert_eq!(before, snapshot(t.path()));
    }
}
