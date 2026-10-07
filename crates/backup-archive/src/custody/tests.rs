//! Private seams: deterministic I/O faults, cancellation points and real
//! SIGKILL children. Public-API scenarios are in `tests/custody.rs`.

use super::*;
use limeos_domain::backups::{
    ArchiveFormat, ArchiveLimits, EntryKind, LegacyMapping, ManagedResource,
};
use std::{
    cell::Cell,
    collections::BTreeMap,
    fs,
    io::BufRead,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

const MIB: u64 = 1 << 20;
const SENTINEL: &str = "unrelated-sentinel";
const READY: &str = "LIMEOS-CUSTODY-CRASH-READY";

fn policy() -> AdmissionPolicy {
    AdmissionPolicy {
        revision: 7,
        formats: vec![ArchiveFormat::TarGzip, ArchiveFormat::TarZstd],
        limits: ArchiveLimits {
            max_compressed_bytes: 4 * MIB,
            max_decompressed_bytes: 4 * MIB,
            max_file_bytes: MIB,
            max_entries: 64,
            max_path_bytes: 256,
            max_path_depth: 16,
            max_component_bytes: 128,
            max_metadata_bytes: 4096,
            max_total_metadata_bytes: 16384,
            max_zstd_window_log: 21,
        },
        resources: vec![
            ManagedResource {
                id: "config".into(),
                destination_root: "/synthetic/config".into(),
            },
            ManagedResource {
                id: "stacks".into(),
                destination_root: "/synthetic/stacks".into(),
            },
        ],
        legacy_mappings: vec![
            LegacyMapping {
                archive_prefix: "etc/limeos".into(),
                resource: "config".into(),
                resource_prefix: String::new(),
            },
            LegacyMapping {
                archive_prefix: "opt/stacks".into(),
                resource: "stacks".into(),
                resource_prefix: String::new(),
            },
        ],
    }
}

fn limits() -> CustodyLimits {
    CustodyLimits {
        max_archive_bytes: 8 * MIB,
        max_record_bytes: MIB,
    }
}

fn never() -> bool {
    false
}

/// A stored-gzip archive larger than several copy chunks, so a child can be
/// stopped part-way through each pass.
fn archive() -> Vec<u8> {
    let mut data = Vec::new();
    for n in 0u64..8192 {
        data.extend_from_slice(&Sha256::digest(n.to_le_bytes()));
    }
    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes) in [
        ("etc/limeos/core.json", &br#"{"version":1}"#[..]),
        ("opt/stacks/data.bin", &data[..]),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_mtime(0);
        header.set_mode(0o644);
        header.set_size(bytes.len() as u64);
        header.set_cksum();
        builder.append_data(&mut header, name, bytes).unwrap();
    }
    let tar = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::none());
    encoder.write_all(&tar).unwrap();
    encoder.finish().unwrap()
}

struct Scene {
    temp: tempfile::TempDir,
}

impl Scene {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        for dir in ["custody", "staging", "scratch"] {
            fs::create_dir(temp.path().join(dir)).unwrap();
            fs::set_permissions(temp.path().join(dir), fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(temp.path().join("custody").join(SENTINEL), b"unrelated").unwrap();
        fs::write(temp.path().join("source"), archive()).unwrap();
        Self { temp }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
    fn root(&self) -> CustodyRoot {
        CustodyRoot::open(&self.path("custody")).unwrap()
    }
    fn staging(&self) -> StagingRoot {
        StagingRoot::open(&self.path("staging")).unwrap()
    }
    fn source(&self) -> File {
        File::open(self.path("source")).unwrap()
    }
    fn retain_with(&self, io: &mut impl CustodyIo) -> Result<RetainedArchive, CustodyError> {
        retain_with_io(self.source(), policy(), &self.root(), &limits(), &never, io)
    }
    fn custody_names(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(self.path("custody"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
    /// Only the unrelated sentinel remains, byte-identical.
    fn assert_clean(&self) {
        assert_eq!(self.custody_names(), [SENTINEL]);
        assert_eq!(
            fs::read(self.path("custody").join(SENTINEL)).unwrap(),
            b"unrelated"
        );
        assert_eq!(fs::read(self.path("source")).unwrap(), archive());
    }
    fn one(&self, prefix: &str) -> String {
        let found: Vec<_> = self
            .custody_names()
            .into_iter()
            .filter(|n| n.starts_with(prefix))
            .collect();
        assert_eq!(found.len(), 1, "{prefix}: {:?}", self.custody_names());
        found[0].clone()
    }
    /// The binding an honest caller would hold for `record_id`, derived by
    /// retaining identical bytes in a separate scratch root.
    fn binding_for(&self, record_id: &str) -> CustodyBinding {
        let scratch = CustodyRoot::open(&self.path("scratch")).unwrap();
        let retained = retain(self.source(), policy(), &scratch, &limits(), &never).unwrap();
        let binding = CustodyBinding {
            record_id: record_id.into(),
            ..retained.binding().clone()
        };
        retained.discard().unwrap();
        binding
    }
}

fn tree(path: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            out.extend(tree(&path));
            out.insert(path, (meta.mode(), Vec::new()));
        } else {
            out.insert(path.clone(), (meta.mode(), fs::read(&path).unwrap()));
        }
    }
    out
}

fn verify(catalog: &VerifiedCatalog) {
    let mut files = 0;
    for entry in catalog.entries() {
        if let Some(mut reader) = entry.reader() {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(
                entry.metadata().sha256.as_deref(),
                Some(hex::encode(Sha256::digest(&bytes)).as_str())
            );
            files += 1;
        } else {
            assert_eq!(entry.metadata().kind, EntryKind::Directory);
        }
    }
    assert_eq!(files, catalog.manifest().file_count);
}

/// Configurable private faults over real files.
#[derive(Default)]
struct Faults<'a> {
    id: Option<[u8; 16]>,
    /// Serve these bytes instead of the selected file's, as a changing source.
    source: Option<Vec<u8>>,
    source_read: u64,
    read_max: Option<usize>,
    write_max: Option<usize>,
    interrupt: bool,
    toggle: bool,
    zero_write: bool,
    record_enospc: bool,
    archive_enospc_after: Option<usize>,
    sync_fail: Option<Barrier>,
    rename_fail: Option<Errno>,
    cancel: Option<(Point, &'a Cell<bool>)>,
    at: Option<(Point, Box<dyn FnMut() + 'a>)>,
}

impl CustodyIo for Faults<'_> {
    fn random(&mut self, bytes: &mut [u8; 16]) -> io::Result<()> {
        match self.id {
            Some(id) => {
                *bytes = id;
                Ok(())
            }
            None => getrandom::getrandom(bytes).map_err(|e| io::Error::other(e.to_string())),
        }
    }
    fn read_at(&mut self, file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        if self.interrupt {
            self.toggle = !self.toggle;
            if self.toggle {
                return Err(io::ErrorKind::Interrupted.into());
            }
        }
        let want = buffer.len().min(self.read_max.unwrap_or(usize::MAX));
        // Only the selected source is replaced; sealed copies (0400) are real.
        if let Some(source) = &self.source
            && rustix::fs::fstat(file)?.st_mode & 0o7777 != 0o400
        {
            let start = usize::try_from(offset).unwrap().min(source.len());
            let n = want.min(source.len() - start);
            buffer[..n].copy_from_slice(&source[start..start + n]);
            self.source_read += n as u64;
            return Ok(n);
        }
        file.read_at(&mut buffer[..want], offset)
    }
    fn write(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<usize> {
        if self.interrupt {
            self.toggle = !self.toggle;
            if self.toggle {
                return Err(io::ErrorKind::Interrupted.into());
            }
        }
        if self.zero_write {
            return Ok(0);
        }
        if self.record_enospc && bytes.starts_with(br#"{"binding""#) {
            file.write_all(&bytes[..1])?;
            return Err(Errno::NOSPC.into());
        }
        if let Some(remaining) = &mut self.archive_enospc_after {
            if *remaining == 0 {
                file.write_all(&bytes[..1])?;
                return Err(Errno::NOSPC.into());
            }
            *remaining -= 1;
        }
        let n = bytes.len().min(self.write_max.unwrap_or(usize::MAX));
        file.write(&bytes[..n])
    }
    fn sync(&mut self, file: &File, barrier: Barrier) -> io::Result<()> {
        if self.sync_fail == Some(barrier) {
            return Err(Errno::IO.into());
        }
        file.sync_all()
    }
    fn rename(&mut self, root: &File, from: &str, to: &str) -> io::Result<()> {
        if let Some(errno) = self.rename_fail {
            return Err(errno.into());
        }
        rename_no_replace(root, from, to)
    }
    fn reached(&mut self, point: Point) {
        if let Some((at, flag)) = self.cancel
            && at == point
        {
            flag.set(true);
        }
        if let Some((at, action)) = &mut self.at
            && *at == point
        {
            action();
        }
    }
}

#[test]
fn identity_encoding_is_versioned_order_sensitive_and_bounded() {
    let base = policy();
    let id = |p: &AdmissionPolicy| identity::policy(p, MIB).unwrap();
    assert_eq!(
        id(&base),
        "5ab96685ef8c9aed9e59792e69dc754f920586c3e08769b5d0be21448b60b556",
        "policy identity encoding v1 changed; bump FORMAT_VERSION and the domain"
    );
    let mut reordered = base.clone();
    reordered.formats.reverse();
    let mut resources = base.clone();
    resources.resources.reverse();
    let mut mappings = base.clone();
    mappings.legacy_mappings.reverse();
    for changed in [&reordered, &resources, &mappings] {
        assert_ne!(id(changed), id(&base));
    }
    // Length prefixes keep adjacent strings from sliding into each other.
    let mut left = base.clone();
    left.legacy_mappings[0].archive_prefix = "etc/limeosx".into();
    left.legacy_mappings[0].resource = "y".into();
    let mut right = base.clone();
    right.legacy_mappings[0].archive_prefix = "etc/limeos".into();
    right.legacy_mappings[0].resource = "xy".into();
    assert_ne!(id(&left), id(&right));
    // The bound is exact: the smallest accepted limit fits, one less does not.
    let mut fit = 1;
    while identity::policy(&base, fit).is_err() {
        fit += 1;
    }
    // 304 bytes, as the independent reference in the evidence also computes.
    assert_eq!(fit, 304);
    assert!(matches!(
        identity::policy(&base, fit - 1),
        Err(CustodyError::Limit(CustodyLimit::Record))
    ));

    let manifest = staging::admit_for_staging(&archive()[..], base, &never)
        .unwrap()
        .manifest()
        .clone();
    let mid = |m: &RestoreManifest| identity::manifest(m, MIB).unwrap();
    assert_eq!(
        mid(&manifest),
        "85beea489368a61f28ad3b56a9d7effdaca62d4525c8727632773b6f37e388eb",
        "manifest identity encoding v1 changed; bump FORMAT_VERSION and the domain"
    );
    let mut swapped = manifest.clone();
    swapped.entries.reverse();
    let mut header = manifest.clone();
    header.header_count += 1;
    let mut digest = manifest.clone();
    digest.entries[0].sha256 = None;
    for changed in [&swapped, &header, &digest] {
        assert_ne!(mid(changed), mid(&manifest));
    }
}

#[test]
fn changing_source_is_refused_after_reading_only_the_sentinel() {
    let scene = Scene::new();
    let bytes = archive();
    let limit = bytes.len() as u64 + 100;
    let mut grown = bytes.clone();
    grown.resize(bytes.len() + 2 * MIB as usize, 0x5a);
    let mut faults = Faults {
        source: Some(grown),
        ..Faults::default()
    };
    let small = CustodyLimits {
        max_archive_bytes: limit,
        ..limits()
    };
    assert!(matches!(
        retain_with_io(
            scene.source(),
            policy(),
            &scene.root(),
            &small,
            &never,
            &mut faults
        ),
        Err(CustodyError::Limit(CustodyLimit::Archive))
    ));
    assert_eq!(faults.source_read, limit + 1);
    scene.assert_clean();
    // A source that shrinks after selection is copied only as far as it goes,
    // and admission then judges exactly those bytes.
    let mut faults = Faults {
        source: Some(bytes[..bytes.len() / 2].to_vec()),
        ..Faults::default()
    };
    assert!(matches!(
        scene.retain_with(&mut faults),
        Err(CustodyError::Admission(_))
    ));
    scene.assert_clean();
}

#[test]
fn short_and_interrupted_reads_and_writes_preserve_the_exact_copy() {
    let scene = Scene::new();
    let mut faults = Faults {
        read_max: Some(4093),
        write_max: Some(1021),
        interrupt: true,
        ..Faults::default()
    };
    let retained = scene.retain_with(&mut faults).unwrap();
    let bytes = archive();
    assert_eq!(
        retained.binding().archive_sha256,
        hex::encode(Sha256::digest(&bytes))
    );
    let dir = scene.path("custody").join(&retained.name);
    assert_eq!(fs::read(dir.join(ARCHIVE)).unwrap(), bytes);
    retained.discard().unwrap();
    scene.assert_clean();
}

#[test]
fn zero_failed_and_full_writes_are_typed_and_cleaned() {
    let scene = Scene::new();
    fn enospc(e: &CustodyError) -> bool {
        matches!(e, CustodyError::Io { source, .. }
            if source.raw_os_error() == Some(Errno::NOSPC.raw_os_error()))
    }
    fn zero(e: &CustodyError) -> bool {
        matches!(e, CustodyError::Io { source, .. }
            if source.kind() == io::ErrorKind::WriteZero)
    }
    type Expect = fn(&CustodyError) -> bool;
    let cases: [(Faults, Expect); 3] = [
        (
            Faults {
                zero_write: true,
                ..Faults::default()
            },
            zero,
        ),
        (
            Faults {
                archive_enospc_after: Some(2),
                ..Faults::default()
            },
            enospc,
        ),
        (
            Faults {
                record_enospc: true,
                ..Faults::default()
            },
            enospc,
        ),
    ];
    for (mut faults, expected) in cases {
        let error = scene.retain_with(&mut faults).unwrap_err();
        assert!(expected(&error), "{error:?}");
        scene.assert_clean();
    }
}

#[test]
fn every_durability_barrier_failure_is_typed_and_publication_is_never_rolled_back() {
    let scene = Scene::new();
    for barrier in [Barrier::Archive, Barrier::Record, Barrier::Pending] {
        let mut faults = Faults {
            sync_fail: Some(barrier),
            ..Faults::default()
        };
        let error = scene.retain_with(&mut faults).unwrap_err();
        assert!(
            matches!(error, CustodyError::Io { .. }),
            "{barrier:?}: {error:?}"
        );
        scene.assert_clean();
    }
    // A failed root fsync after rename is reported as ambiguous, not success
    // and not rolled back. Recovery still requires full fresh validation.
    let mut faults = Faults {
        sync_fail: Some(Barrier::Root),
        ..Faults::default()
    };
    let binding = match scene.retain_with(&mut faults) {
        Err(CustodyError::AmbiguousDurability { binding, .. }) => *binding,
        other => panic!("expected ambiguous durability, got {other:?}"),
    };
    assert_eq!(
        scene.one(PUBLISHED),
        format!("{PUBLISHED}{}", binding.record_id)
    );
    let mut retained = recover(&scene.root(), &binding, policy(), &limits(), &never).unwrap();
    let catalog = retained.stage(&policy(), &scene.staging(), &never).unwrap();
    verify(&catalog);
    catalog.discard().unwrap();
    // Discard reports a failed final barrier after removing owned names.
    let mut faults = Faults {
        sync_fail: Some(Barrier::Discard),
        ..Faults::default()
    };
    assert!(matches!(
        retained.discard_with_io(&mut faults),
        Err(CustodyError::Cleanup {
            failure: None,
            cleanup: CleanupFailure::Io { .. }
        })
    ));
    scene.assert_clean();
}

#[test]
fn rename_failures_and_generated_name_collisions_never_replace_foreign_entries() {
    let scene = Scene::new();
    let id = [0xab; 16];
    let hex_id = hex::encode(id);
    let custody = scene.path("custody");
    let fixed = || Faults {
        id: Some(id),
        ..Faults::default()
    };
    for prefix in [PENDING, PUBLISHED] {
        let foreign = custody.join(format!("{prefix}{hex_id}"));
        fs::create_dir(&foreign).unwrap();
        fs::write(foreign.join("foreign"), b"keep").unwrap();
        assert!(matches!(
            scene.retain_with(&mut fixed()),
            Err(CustodyError::Collision)
        ));
        assert_eq!(fs::read(foreign.join("foreign")).unwrap(), b"keep");
        fs::remove_dir_all(&foreign).unwrap();
        scene.assert_clean();
    }
    // The published name appears after the early check: rename refuses it.
    let published = custody.join(format!("{PUBLISHED}{hex_id}"));
    let racer = published.clone();
    let mut faults = Faults {
        at: Some((
            Point::RecordSynced,
            Box::new(move || {
                fs::create_dir(&racer).unwrap();
                fs::write(racer.join("foreign"), b"keep").unwrap();
            }),
        )),
        ..fixed()
    };
    assert!(matches!(
        scene.retain_with(&mut faults),
        Err(CustodyError::Collision)
    ));
    assert_eq!(fs::read(published.join("foreign")).unwrap(), b"keep");
    fs::remove_dir_all(&published).unwrap();
    scene.assert_clean();
    // Other rename failures, including a filesystem without RENAME_NOREPLACE.
    for errno in [Errno::IO, Errno::INVAL] {
        let mut faults = Faults {
            rename_fail: Some(errno),
            ..Faults::default()
        };
        assert!(matches!(
            scene.retain_with(&mut faults),
            Err(CustodyError::Io { .. })
        ));
        scene.assert_clean();
    }
}

#[test]
fn cancellation_rolls_back_only_before_publication() {
    let scene = Scene::new();
    for point in [
        Point::Copying,
        Point::ArchiveSynced,
        Point::Admitted,
        Point::RecordSynced,
    ] {
        let flag = Cell::new(false);
        let mut faults = Faults {
            cancel: Some((point, &flag)),
            ..Faults::default()
        };
        let result = retain_with_io(
            scene.source(),
            policy(),
            &scene.root(),
            &limits(),
            &|| flag.get(),
            &mut faults,
        );
        assert!(matches!(result, Err(CustodyError::Cancelled)), "{point:?}");
        scene.assert_clean();
    }
    // After the rename, cancellation cannot undo publication.
    let flag = Cell::new(false);
    let mut faults = Faults {
        cancel: Some((Point::Renamed, &flag)),
        ..Faults::default()
    };
    let retained = retain_with_io(
        scene.source(),
        policy(),
        &scene.root(),
        &limits(),
        &|| flag.get(),
        &mut faults,
    )
    .unwrap();
    let binding = retained.binding().clone();
    drop(retained);
    // Cancellation during re-admission and during replay.
    let flag = Cell::new(false);
    let mut faults = Faults {
        cancel: Some((Point::Readmitting, &flag)),
        ..Faults::default()
    };
    let result = recover_with_io(
        &scene.root(),
        &binding,
        policy(),
        &limits(),
        &|| flag.get(),
        &mut faults,
    );
    assert!(matches!(result, Err(CustodyError::Cancelled)));
    let mut retained = recover(&scene.root(), &binding, policy(), &limits(), &never).unwrap();
    let flag = Cell::new(false);
    let mut faults = Faults {
        cancel: Some((Point::Replaying, &flag)),
        ..Faults::default()
    };
    let result = retained.stage_with_io(&policy(), &scene.staging(), &|| flag.get(), &mut faults);
    assert!(matches!(
        result,
        Err(CustodyError::Staging(StagingError::Cancelled))
    ));
    assert_eq!(fs::read_dir(scene.path("staging")).unwrap().count(), 0);
    retained.discard().unwrap();
    scene.assert_clean();
}

#[test]
fn pending_cleanup_reports_replaced_and_foreign_objects_and_keeps_them() {
    let scene = Scene::new();
    let custody = scene.path("custody");
    let id = [0xcd; 16];
    let pending = custody.join(format!("{PENDING}{}", hex::encode(id)));
    let replace = pending.clone();
    let flag = Cell::new(false);
    let mut faults = Faults {
        id: Some(id),
        cancel: Some((Point::Admitted, &flag)),
        at: Some((
            Point::Admitted,
            Box::new(move || {
                let foreign = replace.join("foreign");
                fs::write(&foreign, b"foreign archive").unwrap();
                fs::rename(&foreign, replace.join(ARCHIVE)).unwrap();
            }),
        )),
        ..Faults::default()
    };
    let result = retain_with_io(
        scene.source(),
        policy(),
        &scene.root(),
        &limits(),
        &|| flag.get(),
        &mut faults,
    );
    assert!(matches!(
        result,
        Err(CustodyError::Cleanup {
            failure: Some(ref primary),
            cleanup: CleanupFailure::Replaced("archive"),
        }) if matches!(**primary, CustodyError::Cancelled)
    ));
    assert_eq!(fs::read(pending.join(ARCHIVE)).unwrap(), b"foreign archive");
    fs::remove_dir_all(&pending).unwrap();

    let add = pending.clone();
    let flag = Cell::new(false);
    let mut faults = Faults {
        id: Some(id),
        cancel: Some((Point::RecordSynced, &flag)),
        at: Some((
            Point::RecordSynced,
            Box::new(move || fs::write(add.join("foreign"), b"keep").unwrap()),
        )),
        ..Faults::default()
    };
    let result = retain_with_io(
        scene.source(),
        policy(),
        &scene.root(),
        &limits(),
        &|| flag.get(),
        &mut faults,
    );
    assert!(matches!(
        result,
        Err(CustodyError::Cleanup {
            cleanup: CleanupFailure::Foreign(1),
            ..
        })
    ));
    // Owned names are gone; the foreign entry and its directory remain.
    let left: Vec<_> = fs::read_dir(&pending)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, ["foreign"]);
    fs::remove_dir_all(&pending).unwrap();
    scene.assert_clean();
}

/// Blocks forever at one point after telling the parent it arrived.
struct Crash(Point);

impl CustodyIo for Crash {
    fn reached(&mut self, point: Point) {
        if point == self.0 {
            println!("{READY}");
            let _ = io::stdout().flush();
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

const POINTS: [Point; 8] = [
    Point::Copying,
    Point::ArchiveSynced,
    Point::Admitted,
    Point::RecordSynced,
    Point::Renamed,
    Point::Published,
    Point::Readmitting,
    Point::Replaying,
];

/// Subprocess helper: no work unless the parent selects a private scenario.
#[test]
fn crash_child() {
    if let Some(dir) = std::env::var_os("LIMEOS_CUSTODY_FIFO_DIR") {
        let dir = PathBuf::from(dir);
        let root = CustodyRoot::open(&dir.join("custody")).unwrap();
        let binding: CustodyBinding =
            serde_json::from_slice(&fs::read(dir.join("binding.json")).unwrap()).unwrap();
        let name = std::env::var("LIMEOS_CUSTODY_FIFO_NAME").unwrap();
        assert!(matches!(
            recover(&root, &binding, policy(), &limits(), &never),
            Err(CustodyError::UnsafeObject(what)) if what == name
        ));
        return;
    }
    let (Some(point), Some(dir)) = (
        std::env::var_os("LIMEOS_CUSTODY_CRASH_POINT"),
        std::env::var_os("LIMEOS_CUSTODY_CRASH_DIR"),
    ) else {
        return;
    };
    let point = POINTS
        .into_iter()
        .find(|p| format!("{p:?}") == point.to_str().unwrap())
        .unwrap();
    let dir = PathBuf::from(dir);
    let root = CustodyRoot::open(&dir.join("custody")).unwrap();
    let mut crash = Crash(point);
    let binding = || -> CustodyBinding {
        serde_json::from_slice(&fs::read(dir.join("binding.json")).unwrap()).unwrap()
    };
    match point {
        Point::Readmitting => {
            let _ = recover_with_io(&root, &binding(), policy(), &limits(), &never, &mut crash);
        }
        Point::Replaying => {
            let mut retained = recover(&root, &binding(), policy(), &limits(), &never).unwrap();
            let staging = StagingRoot::open(&dir.join("staging")).unwrap();
            let _ = retained.stage_with_io(&policy(), &staging, &never, &mut crash);
        }
        _ => {
            let source = File::open(dir.join("source")).unwrap();
            let _ = retain_with_io(source, policy(), &root, &limits(), &never, &mut crash);
        }
    }
    panic!("child must be killed at {point:?}");
}

#[test]
fn fifo_record_and_archive_replacements_refuse_without_waiting_for_a_writer() {
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for (object, expected) in [(RECORD, "record"), (ARCHIVE, "archive")] {
        let scene = Scene::new();
        let retained = retain(scene.source(), policy(), &scene.root(), &limits(), &never).unwrap();
        fs::write(
            scene.path("binding.json"),
            serde_json::to_vec(retained.binding()).unwrap(),
        )
        .unwrap();
        let directory = scene.path("custody").join(scene.one(PUBLISHED));
        drop(retained);
        let replacement = directory.join(object);
        fs::remove_file(&replacement).unwrap();
        rustix::fs::mkfifoat(rustix::fs::CWD, &replacement, Mode::from_raw_mode(0o400)).unwrap();
        let mut child = Child(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "custody::tests::crash_child", "--nocapture"])
                .env("LIMEOS_CUSTODY_FIFO_DIR", scene.temp.path())
                .env("LIMEOS_CUSTODY_FIFO_NAME", expected)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "recovery waited for a FIFO writer: {object}"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "{object}: {status}");
        assert_eq!(
            FileType::from_raw_mode(fs::symlink_metadata(&replacement).unwrap().mode()),
            FileType::Fifo,
        );
        assert_eq!(fs::read(scene.path("source")).unwrap(), archive());
        assert_eq!(
            fs::read(scene.path("custody").join(SENTINEL)).unwrap(),
            b"unrelated",
        );
    }
}

/// Run the child until it reports `point`, then SIGKILL it.
fn kill_at(scene: &Scene, point: Point) {
    use std::os::unix::process::ExitStatusExt;
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "custody::tests::crash_child", "--nocapture"])
            .env("LIMEOS_CUSTODY_CRASH_POINT", format!("{point:?}"))
            .env("LIMEOS_CUSTODY_CRASH_DIR", scene.temp.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (ready, arrived) = mpsc::channel();
    std::thread::spawn(move || {
        for line in io::BufReader::new(stdout).lines() {
            if line.as_deref().is_ok_and(|l| l == READY) {
                let _ = ready.send(());
                return;
            }
        }
    });
    arrived
        .recv_timeout(Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("child never reached {point:?}"));
    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    assert_eq!(status.signal(), Some(9), "{point:?}");
}

#[test]
fn sigkill_at_every_custody_point_leaves_untrusted_or_freshly_validated_state() {
    for point in POINTS {
        let scene = Scene::new();
        let mut before = None;
        if matches!(point, Point::Readmitting | Point::Replaying) {
            let retained =
                retain(scene.source(), policy(), &scene.root(), &limits(), &never).unwrap();
            fs::write(
                scene.path("binding.json"),
                serde_json::to_vec(retained.binding()).unwrap(),
            )
            .unwrap();
            before = Some(tree(&scene.path("custody")));
        }
        kill_at(&scene, point);
        assert_eq!(
            fs::read(scene.path("custody").join(SENTINEL)).unwrap(),
            b"unrelated",
            "{point:?}"
        );
        assert_eq!(fs::read(scene.path("source")).unwrap(), archive());
        match point {
            Point::Copying | Point::ArchiveSynced | Point::Admitted | Point::RecordSynced => {
                // Only an untrusted pending orphan exists; nothing published.
                let orphan = scene.one(PENDING);
                assert_eq!(scene.custody_names().len(), 2, "{point:?}");
                let orphan_dir = scene.path("custody").join(&orphan);
                assert_eq!(fs::metadata(&orphan_dir).unwrap().mode() & 0o7777, 0o700);
                let snapshot = tree(&orphan_dir);
                let forged = scene.binding_for(&orphan[PENDING.len()..]);
                assert!(
                    matches!(
                        recover(&scene.root(), &forged, policy(), &limits(), &never),
                        Err(CustodyError::NotFound)
                    ),
                    "{point:?}"
                );
                // A fresh retention is independent and never adopts the orphan.
                let fresh =
                    retain(scene.source(), policy(), &scene.root(), &limits(), &never).unwrap();
                fresh.discard().unwrap();
                assert_eq!(tree(&orphan_dir), snapshot, "{point:?}");
            }
            Point::Renamed | Point::Published => {
                // Published but never delivered: usable only with the exact
                // binding, and only after full fresh validation.
                let published = scene.one(PUBLISHED);
                assert!(
                    scene
                        .custody_names()
                        .iter()
                        .all(|n| !n.starts_with(PENDING))
                );
                let binding = scene.binding_for(&published[PUBLISHED.len()..]);
                let wrong = CustodyBinding {
                    manifest_identity: "0".repeat(64),
                    ..binding.clone()
                };
                assert!(matches!(
                    recover(&scene.root(), &wrong, policy(), &limits(), &never),
                    Err(CustodyError::BindingMismatch)
                ));
                let mut retained =
                    recover(&scene.root(), &binding, policy(), &limits(), &never).unwrap();
                let catalog = retained.stage(&policy(), &scene.staging(), &never).unwrap();
                verify(&catalog);
                catalog.discard().unwrap();
                retained.discard().unwrap();
                scene.assert_clean();
            }
            Point::Readmitting | Point::Replaying => {
                // Recovery and replay never write to custody.
                assert_eq!(Some(tree(&scene.path("custody"))), before, "{point:?}");
                let staged: Vec<_> = fs::read_dir(scene.path("staging"))
                    .unwrap()
                    .map(|e| e.unwrap().file_name().into_string().unwrap())
                    .collect();
                let orphan = if point == Point::Replaying {
                    assert_eq!(staged.len(), 1);
                    assert!(staged[0].starts_with("incomplete-"));
                    Some(tree(&scene.path("staging")))
                } else {
                    assert!(staged.is_empty());
                    None
                };
                let binding: CustodyBinding =
                    serde_json::from_slice(&fs::read(scene.path("binding.json")).unwrap()).unwrap();
                let mut retained =
                    recover(&scene.root(), &binding, policy(), &limits(), &never).unwrap();
                let catalog = retained.stage(&policy(), &scene.staging(), &never).unwrap();
                verify(&catalog);
                catalog.discard().unwrap();
                if let Some(orphan) = orphan {
                    // The killed attempt is neither adopted nor removed.
                    assert_eq!(tree(&scene.path("staging")), orphan);
                }
                retained.discard().unwrap();
                scene.assert_clean();
            }
        }
    }
}
