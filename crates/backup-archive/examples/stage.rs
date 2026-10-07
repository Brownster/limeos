//! Admit, replay and read a private catalog without touching managed roots.
//! The caller must hold the input immutable throughout both passes.
//! Usage: stage POLICY.json ARCHIVE PRIVATE_ROOT (existing owner-only 0700).
//! Exit 0: verified and discarded; 1: refusal/I/O; 2: usage.

use limeos_backup_archive::staging::{StagingError, StagingRoot, admit_for_staging, replay};
use limeos_domain::backups::AdmissionPolicy;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
    process::ExitCode,
};

fn run(policy_path: &str, archive: &str, root: &str) -> Result<(), Box<dyn Error>> {
    let policy: AdmissionPolicy = serde_json::from_reader(File::open(policy_path)?)?;
    let root = StagingRoot::open(Path::new(root))?;
    // Rewind the same source descriptor; never reopen an archive-controlled path.
    let mut source = BufReader::new(File::open(archive)?);
    let admitted = admit_for_staging(&mut source, policy, &|| false).map_err(StagingError::from)?;
    source.seek(SeekFrom::Start(0))?;
    let catalog = replay(
        source,
        admitted.policy_snapshot().policy(),
        admitted.manifest(),
        admitted.policy_snapshot(),
        &root,
        &|| false,
    )?;
    let mut chunk = [0u8; 64 * 1024];
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
    let output = serde_json::to_string_pretty(&serde_json::json!({
        "manifest": catalog.manifest(), "reader_verified_bytes": verified_bytes
    }))?;
    catalog.discard()?;
    println!("{output}");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let [_, policy, archive, root] = args.as_slice() else {
        eprintln!("usage: stage POLICY.json ARCHIVE PRIVATE_ROOT");
        return ExitCode::from(2);
    };
    let result = run(policy, archive, root);
    if std::env::var_os("LIMEOS_REPORT_HWM").is_some() {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        if let Some(line) = status.lines().find(|l| l.starts_with("VmHWM:")) {
            eprintln!("{line}");
        }
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}
