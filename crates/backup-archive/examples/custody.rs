//! Retain an archive in private custody, then recover and stage it after a
//! restart, without touching managed roots.
//!
//! Usage:
//!   custody retain POLICY.json LIMITS.json ARCHIVE CUSTODY_ROOT
//!     Prints the binding JSON a trusted caller would store in its job record.
//!   custody recover POLICY.json LIMITS.json BINDING.json CUSTODY_ROOT STAGING_ROOT [--discard]
//!     Recovers the exact record, stages a fresh catalog, independently hashes
//!     every reader, discards the catalog and, with --discard, the record.
//!
//! Roots must already exist, owner-only 0700. Exit 0: success; 1: refusal or
//! I/O; 2: usage. LIMEOS_REPORT_HWM=1 prints this process's VmHWM to stderr.

use limeos_backup_archive::custody::{CustodyBinding, CustodyLimits, CustodyRoot, recover, retain};
use limeos_backup_archive::staging::StagingRoot;
use limeos_domain::backups::AdmissionPolicy;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{error::Error, fs::File, io::Read, path::Path, process::ExitCode, time::Instant};

/// Explicit trusted custody limits; synthetic values are not defaults.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    max_archive_bytes: u64,
    max_record_bytes: u64,
}

fn load(policy: &str, limits: &str) -> Result<(AdmissionPolicy, CustodyLimits), Box<dyn Error>> {
    let policy = serde_json::from_reader(File::open(policy)?)?;
    let Limits {
        max_archive_bytes,
        max_record_bytes,
    } = serde_json::from_reader(File::open(limits)?)?;
    Ok((
        policy,
        CustodyLimits {
            max_archive_bytes,
            max_record_bytes,
        },
    ))
}

fn run_retain(args: &[String]) -> Result<(), Box<dyn Error>> {
    let [policy, limits, archive, root] = args else {
        return Err("usage".into());
    };
    let (policy, limits) = load(policy, limits)?;
    let root = CustodyRoot::open(Path::new(root))?;
    let started = Instant::now();
    // The selected descriptor is consumed; its path is never reopened.
    let retained = retain(File::open(archive)?, policy, &root, &limits, &|| false)?;
    let output = serde_json::json!({
        "binding": retained.binding(),
        "file_count": retained.manifest().file_count,
        "file_bytes": retained.manifest().file_bytes,
        "retain_seconds": started.elapsed().as_secs_f64(),
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn run_recover(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (args, discard) = match args {
        [rest @ .., flag] if flag == "--discard" => (rest, true),
        rest => (rest, false),
    };
    let [policy, limits, binding, root, staging] = args else {
        return Err("usage".into());
    };
    let (policy, limits) = load(policy, limits)?;
    let binding: CustodyBinding = serde_json::from_reader(File::open(binding)?)?;
    let root = CustodyRoot::open(Path::new(root))?;
    let staging = StagingRoot::open(Path::new(staging))?;
    let started = Instant::now();
    let mut retained = recover(&root, &binding, policy.clone(), &limits, &|| false)?;
    let recovered = started.elapsed().as_secs_f64();
    let catalog = retained.stage(&policy, &staging, &|| false)?;
    let staged = started.elapsed().as_secs_f64() - recovered;
    let mut chunk = vec![0u8; 64 * 1024];
    let mut verified_bytes = 0u64;
    for entry in catalog.entries() {
        if let Some(mut reader) = entry.reader() {
            let mut hash = Sha256::new();
            let mut size = 0u64;
            loop {
                let n = reader.read(&mut chunk)?;
                if n == 0 {
                    break;
                }
                hash.update(&chunk[..n]);
                size += n as u64;
            }
            if size != entry.metadata().size
                || entry.metadata().sha256.as_deref() != Some(hex::encode(hash.finalize()).as_str())
            {
                return Err("catalog reader changed after verification".into());
            }
            verified_bytes += size;
        }
    }
    let manifest = catalog.manifest().clone();
    catalog.discard()?;
    if discard {
        retained.discard()?;
    }
    let output = serde_json::json!({
        "binding": binding,
        "manifest_file_count": manifest.file_count,
        "manifest_file_bytes": manifest.file_bytes,
        "reader_verified_bytes": verified_bytes,
        "recover_seconds": recovered,
        "stage_seconds": staged,
        "record_discarded": discard,
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("retain") => run_retain(&args[2..]),
        Some("recover") => run_recover(&args[2..]),
        _ => Err("usage".into()),
    };
    if std::env::var_os("LIMEOS_REPORT_HWM").is_some() {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        if let Some(line) = status.lines().find(|l| l.starts_with("VmHWM:")) {
            eprintln!("{line}");
        }
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.to_string() == "usage" => {
            eprintln!(
                "usage: custody retain POLICY.json LIMITS.json ARCHIVE CUSTODY_ROOT\n       \
                 custody recover POLICY.json LIMITS.json BINDING.json CUSTODY_ROOT STAGING_ROOT [--discard]"
            );
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}
