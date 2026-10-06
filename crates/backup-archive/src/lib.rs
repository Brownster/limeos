//! Bounded streaming inspection of legacy LimeOS backup archives.
//!
//! [`inspect`] reads one `.tar.gz` or `.tar.zst` stream once, under limits
//! owned by trusted [`AdmissionPolicy`], and returns an [`InspectionReport`]
//! only after reaching a verified end. [`inspect_and_admit`] adds the pure
//! domain admission step and returns a digest-bound [`RestoreManifest`].
//!
//! Nothing here writes to disk, follows links or resolves destinations.

use flate2::bufread::MultiGzDecoder;
use limeos_domain::backups::{
    AdmissionPolicy, ArchiveFormat, EntryKind, FindingCode, InspectedEntry, InspectionReport,
    Rejection, RestoreManifest, admit, display_path,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    error::Error as StdError,
    fmt,
    io::{self, BufRead, BufReader, Read},
};
use tar::{Archive, Header, PaxExtensions};

pub mod staging;

const GZIP_MAGIC: [u8; 3] = [0x1f, 0x8b, 0x08];
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
const BLOCK: u64 = 512;
const CHUNK: usize = 64 * 1024;
/// Pax keys that may appear but carry no authority: archived ownership and
/// times are ignored. Any other key, including `size`, is rejected.
const INFORMATIONAL_PAX_KEYS: &[&str] =
    &["mtime", "atime", "ctime", "uid", "gid", "uname", "gname"];

/// Why a reader stopped on purpose; carried through decoder and tar errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Compressed,
    Decompressed,
    Cancelled,
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Stop::Compressed => "compressed byte limit reached",
            Stop::Decompressed => "decompressed byte limit reached",
            Stop::Cancelled => "inspection cancelled",
        })
    }
}

impl StdError for Stop {}

fn stop_of(error: &io::Error) -> Option<Stop> {
    let mut source: Option<&(dyn StdError + 'static)> = error.get_ref().map(|e| e as _);
    while let Some(e) = source {
        if let Some(stop) = e.downcast_ref::<Stop>() {
            return Some(*stop);
        }
        // `io::Error::source` skips the error it wraps, so look inside nested
        // `io::Error`s explicitly in case a decoder or parser re-wraps one.
        source = match e.downcast_ref::<io::Error>() {
            Some(inner) => inner.get_ref().map(|e| e as _),
            None => e.source(),
        };
    }
    None
}

/// Counts every byte and refuses to read more than one byte past `limit`, so
/// exceeding the limit is detected without consuming the rest. Polls
/// cancellation on every read.
struct Limited<'c, R> {
    inner: R,
    count: u64,
    limit: u64,
    reason: Stop,
    hasher: Option<Sha256>,
    cancelled: &'c dyn Fn() -> bool,
}

impl<R: Read> Read for Limited<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let allowed = self.limit.saturating_sub(self.count).saturating_add(1);
        let want = buf
            .len()
            .min(usize::try_from(allowed).unwrap_or(usize::MAX));
        let n = loop {
            if (self.cancelled)() {
                return Err(io::Error::other(Stop::Cancelled));
            }
            match self.inner.read(&mut buf[..want]) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => break result?,
            }
        };
        self.count = self
            .count
            .checked_add(n as u64)
            .ok_or_else(|| io::Error::other(self.reason))?;
        if self.count > self.limit {
            return Err(io::Error::other(self.reason));
        }
        if let Some(hasher) = &mut self.hasher {
            hasher.update(&buf[..n]);
        }
        Ok(n)
    }
}

// The four sniffed bytes replay only into the decoder. Limited already counted
// and hashed them while reading the source, so they are never counted twice.
type Source<'c, R> = BufReader<io::Chain<io::Cursor<[u8; 4]>, Limited<'c, R>>>;

enum Decoder<'c, R: Read> {
    Gzip(MultiGzDecoder<Source<'c, R>>),
    Zstd(zstd::stream::read::Decoder<'static, Source<'c, R>>),
}

impl<R: Read> Read for Decoder<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Decoder::Gzip(d) => d.read(buf),
            Decoder::Zstd(d) => d.read(buf),
        }
    }
}

impl<'c, R: Read> Decoder<'c, R> {
    fn into_source(self) -> Source<'c, R> {
        match self {
            Decoder::Gzip(d) => d.into_inner(),
            Decoder::Zstd(d) => d.finish(),
        }
    }
}

fn reject(code: FindingCode, entry: Option<u64>, path: Option<&str>, detail: &str) -> Rejection {
    Rejection::one(code, entry, path, detail)
}

/// Translate a read or parse error into a typed finding.
fn classify(error: &io::Error, entry: Option<u64>) -> Rejection {
    let text = error.to_string();
    let code = match stop_of(error) {
        Some(Stop::Compressed) => FindingCode::CompressedLimit,
        Some(Stop::Decompressed) => FindingCode::DecompressedLimit,
        Some(Stop::Cancelled) => FindingCode::Cancelled,
        None if text.contains("Frame requires too much memory") => FindingCode::DecoderMemoryLimit,
        None if error.kind() == io::ErrorKind::UnexpectedEof
            || text.contains("failed to read entire block") =>
        {
            FindingCode::Truncated
        }
        None => FindingCode::Malformed,
    };
    reject(code, entry, None, &text)
}

/// Extension records waiting for the member they describe.
#[derive(Default)]
struct Pending {
    long_name: Option<Vec<u8>>,
    long_link: Option<Vec<u8>>,
    pax_path: Option<Vec<u8>>,
    pax_linkpath: Option<Vec<u8>>,
    pax_seen: bool,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.long_name.is_none() && self.long_link.is_none() && !self.pax_seen
    }
}

fn gnu_name(mut body: Vec<u8>, index: u64) -> Result<Vec<u8>, Rejection> {
    // GNU tar writes the name followed by exactly one NUL.
    if body.last() == Some(&0) {
        body.pop();
    }
    if body.contains(&0) {
        return Err(reject(
            FindingCode::UnsafeCharacter,
            Some(index),
            None,
            "NUL inside a GNU long name",
        ));
    }
    Ok(body)
}

fn read_pax(body: &[u8], pending: &mut Pending, index: u64) -> Result<(), Rejection> {
    let mut seen: Vec<&str> = Vec::new();
    for record in PaxExtensions::new(body) {
        let record = record.map_err(|e| {
            reject(
                FindingCode::Malformed,
                Some(index),
                None,
                &format!("pax record: {e}"),
            )
        })?;
        let key = record.key().map_err(|_| {
            reject(
                FindingCode::Malformed,
                Some(index),
                None,
                "pax key is not UTF-8",
            )
        })?;
        if seen.contains(&key) {
            return Err(reject(
                FindingCode::AmbiguousName,
                Some(index),
                None,
                &format!("pax key {key} repeated"),
            ));
        }
        seen.push(key);
        let value = record.value_bytes().to_vec();
        match key {
            "path" => pending.pax_path = Some(value),
            "linkpath" => pending.pax_linkpath = Some(value),
            k if k.starts_with("GNU.sparse.") => {
                return Err(reject(
                    FindingCode::Sparse,
                    Some(index),
                    None,
                    &format!("pax key {k}"),
                ));
            }
            k if INFORMATIONAL_PAX_KEYS.contains(&k) => {}
            k => {
                return Err(reject(
                    FindingCode::UnsupportedExtension,
                    Some(index),
                    None,
                    &format!("pax key {}", display_path(k)),
                ));
            }
        }
    }
    pending.pax_seen = true;
    Ok(())
}

fn effective_name(
    header_name: Vec<u8>,
    long: Option<Vec<u8>>,
    pax: Option<Vec<u8>>,
    index: u64,
) -> Result<Vec<u8>, Rejection> {
    match (long, pax) {
        (Some(_), Some(_)) => Err(reject(
            FindingCode::AmbiguousName,
            Some(index),
            None,
            "both a GNU long name and a pax record name one member",
        )),
        (Some(name), None) | (None, Some(name)) => {
            if name.contains(&0) {
                return Err(reject(
                    FindingCode::UnsafeCharacter,
                    Some(index),
                    None,
                    "NUL inside an extended name",
                ));
            }
            Ok(name)
        }
        (None, None) => Ok(header_name),
    }
}

fn utf8(name: Vec<u8>, index: u64) -> Result<String, Rejection> {
    String::from_utf8(name).map_err(|e| {
        let lossy = String::from_utf8_lossy(e.as_bytes()).into_owned();
        reject(FindingCode::NonUtf8Name, Some(index), Some(&lossy), "")
    })
}

/// Read one extension record whose declared size the caller already checked.
fn read_record(entry: &mut impl Read, size: u64, index: u64) -> Result<Vec<u8>, Rejection> {
    let mut body = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    entry
        .take(size)
        .read_to_end(&mut body)
        .map_err(|e| classify(&e, Some(index)))?;
    if body.len() as u64 != size {
        return Err(reject(
            FindingCode::Truncated,
            Some(index),
            None,
            "extension record ended early",
        ));
    }
    Ok(body)
}

/// Identify the compression from its magic bytes; filenames are never used.
fn detect(magic: &[u8; 4]) -> Result<ArchiveFormat, Rejection> {
    if magic.starts_with(&ZSTD_MAGIC) {
        Ok(ArchiveFormat::TarZstd)
    } else if magic.starts_with(&GZIP_MAGIC) {
        Ok(ArchiveFormat::TarGzip)
    } else {
        Err(reject(
            FindingCode::UnrecognizedCompression,
            None,
            None,
            "neither gzip nor zstd magic",
        ))
    }
}

/// Stream one archive under `policy` limits and return facts about every
/// member, or the first reason it cannot be admitted. `cancelled` is polled on
/// every read; a cancelled inspection returns no report.
pub fn inspect<R: Read>(
    source: R,
    policy: &AdmissionPolicy,
    cancelled: &dyn Fn() -> bool,
) -> Result<InspectionReport, Rejection> {
    policy.validate()?;
    let limits = &policy.limits;
    let mut compressed = Limited {
        inner: source,
        count: 0,
        limit: limits.max_compressed_bytes,
        reason: Stop::Compressed,
        hasher: Some(Sha256::new()),
        cancelled,
    };
    let mut magic = [0; 4];
    compressed
        .read_exact(&mut magic)
        .map_err(|e| classify(&e, None))?;
    let format = detect(&magic)?;
    if !policy.formats.contains(&format) {
        return Err(reject(FindingCode::FormatNotAllowed, None, None, ""));
    }
    let buffered = BufReader::with_capacity(CHUNK, io::Cursor::new(magic).chain(compressed));
    let decoder = match format {
        ArchiveFormat::TarGzip => Decoder::Gzip(MultiGzDecoder::new(buffered)),
        ArchiveFormat::TarZstd => {
            let mut decoder = zstd::stream::read::Decoder::with_buffer(buffered)
                .map_err(|e| classify(&e, None))?;
            decoder
                .window_log_max(limits.max_zstd_window_log)
                .map_err(|e| classify(&e, None))?;
            Decoder::Zstd(decoder)
        }
    };
    let mut archive = Archive::new(Limited {
        inner: decoder,
        count: 0,
        limit: limits.max_decompressed_bytes,
        reason: Stop::Decompressed,
        hasher: None,
        cancelled,
    });
    let mut entries_out = Vec::new();
    let mut completed_files = BTreeSet::new();
    let mut coalesced_self_hardlinks = 0;
    let mut header_count: u64 = 0;
    let mut metadata_bytes: u64 = 0;
    let mut pending = Pending::default();
    let mut chunk = vec![0u8; CHUNK];
    {
        // Raw iteration: the library would otherwise read extension records of
        // any declared size into memory and apply pax overrides itself.
        let entries = archive.entries().map_err(|e| classify(&e, None))?.raw(true);
        for next in entries {
            let index = header_count;
            if cancelled() {
                return Err(reject(FindingCode::Cancelled, Some(index), None, ""));
            }
            let mut entry = next.map_err(|e| classify(&e, Some(index)))?;
            header_count += 1;
            if header_count > limits.max_entries {
                return Err(reject(FindingCode::EntryLimit, Some(index), None, ""));
            }
            let header: Header = entry.header().clone();
            if header.as_gnu().is_none() && header.as_ustar().is_none() {
                return Err(reject(
                    FindingCode::UnsupportedHeaderFormat,
                    Some(index),
                    None,
                    "only GNU and ustar headers are supported",
                ));
            }
            let size = header.entry_size().map_err(|e| classify(&e, Some(index)))?;
            let kind = header.entry_type();
            if kind.is_gnu_longname() || kind.is_gnu_longlink() || kind.is_pax_local_extensions() {
                // Refuse an oversized declaration before allocating or reading.
                if size > limits.max_metadata_bytes {
                    return Err(reject(
                        FindingCode::MetadataLimit,
                        Some(index),
                        None,
                        &format!("{size} bytes declared"),
                    ));
                }
                metadata_bytes = metadata_bytes
                    .checked_add(size)
                    .filter(|total| *total <= limits.max_total_metadata_bytes)
                    .ok_or_else(|| {
                        reject(FindingCode::TotalMetadataLimit, Some(index), None, "")
                    })?;
                let body = read_record(&mut entry, size, index)?;
                let repeated = if kind.is_gnu_longname() {
                    pending.long_name.replace(gnu_name(body, index)?).is_some()
                } else if kind.is_gnu_longlink() {
                    pending.long_link.replace(gnu_name(body, index)?).is_some()
                } else if pending.pax_seen {
                    true
                } else {
                    read_pax(&body, &mut pending, index)?;
                    false
                };
                if repeated {
                    return Err(reject(
                        FindingCode::AmbiguousName,
                        Some(index),
                        None,
                        "two extension records of one kind describe one member",
                    ));
                }
                continue;
            }
            if kind.is_pax_global_extensions() {
                return Err(reject(
                    FindingCode::UnsupportedExtension,
                    Some(index),
                    None,
                    "pax global header",
                ));
            }
            let taken = std::mem::take(&mut pending);
            let name = effective_name(
                header.path_bytes().into_owned(),
                taken.long_name,
                taken.pax_path,
                index,
            )?;
            let path = utf8(name, index)?;
            let at = |code, detail: &str| reject(code, Some(index), Some(&path), detail);
            if kind.is_symlink() || kind.is_hard_link() {
                let header_link = header
                    .link_name_bytes()
                    .map(|l| l.into_owned())
                    .unwrap_or_default();
                let link = effective_name(header_link, taken.long_link, taken.pax_linkpath, index)?;
                // GNU tar emits these when the frozen helper lists a file
                // both through its parent directory and as an explicit source.
                // The earlier file has already been fully read and hashed.
                // Count this header, but create no entry or link operation.
                if kind.is_hard_link()
                    && size == 0
                    && link == path.as_bytes()
                    && completed_files.contains(&path)
                {
                    coalesced_self_hardlinks += 1;
                    continue;
                }
                let detail = format!("target {}", display_path(&String::from_utf8_lossy(&link)));
                return Err(if kind.is_symlink() {
                    at(FindingCode::Symlink, &detail)
                } else if link == path.as_bytes() {
                    at(FindingCode::LegacySelfHardlink, &detail)
                } else {
                    at(FindingCode::Hardlink, &detail)
                });
            }
            if taken.long_link.is_some() || taken.pax_linkpath.is_some() {
                return Err(at(
                    FindingCode::AmbiguousName,
                    "link name on a member that is not a link",
                ));
            }
            let entry_kind = if kind.is_character_special() || kind.is_block_special() {
                return Err(at(FindingCode::Device, ""));
            } else if kind.is_fifo() {
                return Err(at(FindingCode::Fifo, ""));
            } else if kind.is_gnu_sparse() {
                return Err(at(FindingCode::Sparse, ""));
            } else if kind.is_dir() {
                EntryKind::Directory
            } else if matches!(kind.as_byte(), b'0' | b'\0') {
                EntryKind::File
            } else {
                return Err(at(
                    FindingCode::UnsupportedEntryType,
                    &format!("type {:?}", char::from(kind.as_byte())),
                ));
            };
            let mode = header.mode().map_err(|e| classify(&e, Some(index)))? & 0o7777;
            let sha256 = match entry_kind {
                EntryKind::Directory => {
                    if size != 0 {
                        return Err(at(FindingCode::DirectoryWithData, &format!("{size} bytes")));
                    }
                    None
                }
                EntryKind::File => {
                    // The declaration alone is enough to refuse.
                    if size > limits.max_file_bytes {
                        return Err(at(
                            FindingCode::FileSizeLimit,
                            &format!("{size} bytes declared"),
                        ));
                    }
                    let mut hasher = Sha256::new();
                    let mut read: u64 = 0;
                    loop {
                        let n = entry
                            .read(&mut chunk)
                            .map_err(|e| classify(&e, Some(index)))?;
                        if n == 0 {
                            break;
                        }
                        hasher.update(&chunk[..n]);
                        read += n as u64;
                    }
                    if read != size {
                        return Err(at(FindingCode::Truncated, "file data ended early"));
                    }
                    completed_files.insert(path.clone());
                    Some(hex::encode(hasher.finalize()))
                }
            };
            entries_out.push(InspectedEntry {
                header_index: index,
                archive_path: path,
                kind: entry_kind,
                size,
                sha256,
                archived_mode: mode,
            });
        }
    }
    if !pending.is_empty() {
        return Err(reject(
            FindingCode::ExtensionWithoutMember,
            Some(header_count),
            None,
            "",
        ));
    }
    // The library stops at the first zero block, or at a bare EOF. Require the
    // second end-of-archive block and nothing but zero padding after it.
    let mut decompressed = archive.into_inner();
    let mut trailing: u64 = 0;
    loop {
        let n = decompressed
            .read(&mut chunk)
            .map_err(|e| classify(&e, None))?;
        if n == 0 {
            break;
        }
        if chunk[..n].iter().any(|b| *b != 0) {
            return Err(reject(
                FindingCode::TrailingData,
                None,
                None,
                "data after the end-of-archive marker",
            ));
        }
        trailing += n as u64;
    }
    if trailing < BLOCK || trailing % BLOCK != 0 {
        return Err(reject(
            FindingCode::Truncated,
            None,
            None,
            "missing or partial end-of-archive marker",
        ));
    }
    let decompressed_bytes = decompressed.count;
    let mut source = decompressed.inner.into_source();
    if !source
        .fill_buf()
        .map_err(|e| classify(&e, None))?
        .is_empty()
    {
        return Err(reject(
            FindingCode::TrailingData,
            None,
            None,
            "bytes after the compressed stream",
        ));
    }
    let (_, compressed) = source.into_inner().into_inner();
    Ok(InspectionReport {
        format,
        archive_sha256: hex::encode(compressed.hasher.unwrap_or_default().finalize()),
        compressed_bytes: compressed.count,
        decompressed_bytes,
        header_count,
        metadata_bytes,
        coalesced_self_hardlinks,
        entries: entries_out,
    })
}

/// Inspect one archive and admit it under `policy`. A manifest is returned
/// only for a complete, fully admitted archive.
pub fn inspect_and_admit<R: Read>(
    source: R,
    policy: &AdmissionPolicy,
    cancelled: &dyn Fn() -> bool,
) -> Result<RestoreManifest, Rejection> {
    let report = inspect(source, policy, cancelled)?;
    admit(policy, &report)
}

#[cfg(test)]
mod tests;
