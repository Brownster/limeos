//! Versioned, deterministic identity encodings for custody bindings.
//!
//! Each identity is the lowercase hex SHA-256 of a domain-separated binary
//! encoding. Integers are fixed-width big-endian; strings and byte strings are
//! a u64 length followed by their bytes; sequences are a u64 count followed by
//! their items in stored order; enums are one tag byte; `Option` is tag 0 or
//! tag 1 followed by the value. Every struct is destructured, so adding a
//! domain field fails to compile until the encoding (and its version) changes.
//! The encoded length is bounded; an oversized value is refused, not hashed.

use super::{CustodyError, CustodyLimit};
use limeos_domain::backups::{
    AdmissionPolicy, ArchiveFormat, ArchiveLimits, EntryKind, LegacyMapping, ManagedResource,
    ManifestEntry, RestoreManifest,
};
use sha2::{Digest, Sha256};

const POLICY_DOMAIN: &str = "limeos.backup-custody.policy-identity.v1";
const MANIFEST_DOMAIN: &str = "limeos.backup-custody.manifest-identity.v1";

struct Encoder {
    hasher: Sha256,
    written: u64,
    limit: u64,
    overflow: bool,
}

impl Encoder {
    fn new(domain: &str, limit: u64) -> Self {
        let mut encoder = Self {
            hasher: Sha256::new(),
            written: 0,
            limit,
            overflow: false,
        };
        encoder.str(domain);
        encoder
    }

    fn raw(&mut self, bytes: &[u8]) {
        match self.written.checked_add(bytes.len() as u64) {
            Some(total) if !self.overflow && total <= self.limit => {
                self.written = total;
                self.hasher.update(bytes);
            }
            _ => self.overflow = true,
        }
    }

    fn u8(&mut self, value: u8) {
        self.raw(&[value]);
    }

    fn u32(&mut self, value: u32) {
        self.raw(&value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.raw(&value.to_be_bytes());
    }

    fn str(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.raw(value.as_bytes());
    }

    fn count(&mut self, len: usize) {
        self.u64(len as u64);
    }

    fn format(&mut self, format: ArchiveFormat) {
        self.u8(match format {
            ArchiveFormat::TarGzip => 1,
            ArchiveFormat::TarZstd => 2,
        });
    }

    fn finish(self) -> Result<String, CustodyError> {
        if self.overflow {
            return Err(CustodyError::Limit(CustodyLimit::Record));
        }
        Ok(hex::encode(self.hasher.finalize()))
    }
}

/// Identity of the complete policy, preserving every vector's order.
pub(super) fn policy(policy: &AdmissionPolicy, limit: u64) -> Result<String, CustodyError> {
    let AdmissionPolicy {
        revision,
        formats,
        limits,
        resources,
        legacy_mappings,
    } = policy;
    let mut e = Encoder::new(POLICY_DOMAIN, limit);
    e.u64(*revision);
    e.count(formats.len());
    for format in formats {
        e.format(*format);
    }
    let ArchiveLimits {
        max_compressed_bytes,
        max_decompressed_bytes,
        max_file_bytes,
        max_entries,
        max_path_bytes,
        max_path_depth,
        max_component_bytes,
        max_metadata_bytes,
        max_total_metadata_bytes,
        max_zstd_window_log,
    } = limits;
    e.u64(*max_compressed_bytes);
    e.u64(*max_decompressed_bytes);
    e.u64(*max_file_bytes);
    e.u64(*max_entries);
    e.u32(*max_path_bytes);
    e.u32(*max_path_depth);
    e.u32(*max_component_bytes);
    e.u64(*max_metadata_bytes);
    e.u64(*max_total_metadata_bytes);
    e.u32(*max_zstd_window_log);
    e.count(resources.len());
    for ManagedResource {
        id,
        destination_root,
    } in resources
    {
        e.str(id);
        e.str(destination_root);
    }
    e.count(legacy_mappings.len());
    for LegacyMapping {
        archive_prefix,
        resource,
        resource_prefix,
    } in legacy_mappings
    {
        e.str(archive_prefix);
        e.str(resource);
        e.str(resource_prefix);
    }
    e.finish()
}

/// Identity of the complete v2 manifest, including entry order.
pub(super) fn manifest(manifest: &RestoreManifest, limit: u64) -> Result<String, CustodyError> {
    let RestoreManifest {
        manifest_version,
        archive_sha256,
        format,
        policy_revision,
        compressed_bytes,
        decompressed_bytes,
        header_count,
        coalesced_self_hardlinks,
        file_count,
        directory_count,
        file_bytes,
        entries,
    } = manifest;
    let mut e = Encoder::new(MANIFEST_DOMAIN, limit);
    e.u32(*manifest_version);
    e.str(archive_sha256);
    e.format(*format);
    e.u64(*policy_revision);
    e.u64(*compressed_bytes);
    e.u64(*decompressed_bytes);
    e.u64(*header_count);
    e.u64(*coalesced_self_hardlinks);
    e.u64(*file_count);
    e.u64(*directory_count);
    e.u64(*file_bytes);
    e.count(entries.len());
    for ManifestEntry {
        resource,
        relative_path,
        kind,
        size,
        sha256,
        archived_permissions,
        archive_path,
    } in entries
    {
        e.str(resource);
        e.str(relative_path);
        e.u8(match kind {
            EntryKind::File => 1,
            EntryKind::Directory => 2,
        });
        e.u64(*size);
        match sha256 {
            None => e.u8(0),
            Some(digest) => {
                e.u8(1);
                e.str(digest);
            }
        }
        e.u32(*archived_permissions);
        e.str(archive_path);
    }
    e.finish()
}
