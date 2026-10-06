//! Inspect one archive under a policy file and print the manifest or rejection.
//!
//! Usage: cargo run --release --example inspect -- POLICY.json ARCHIVE
//! Exit status 0 means admitted, 1 rejected, 2 usage or I/O error.

use limeos_backup_archive::inspect_and_admit;
use limeos_domain::backups::AdmissionPolicy;
use std::{fs::File, io::BufReader, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let [_, policy, archive] = args.as_slice() else {
        eprintln!("usage: inspect POLICY.json ARCHIVE");
        return ExitCode::from(2);
    };
    let policy: AdmissionPolicy = match std::fs::read(policy).map(|b| serde_json::from_slice(&b)) {
        Ok(Ok(policy)) => policy,
        Ok(Err(e)) => {
            eprintln!("invalid policy: {e}");
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("cannot read policy: {e}");
            return ExitCode::from(2);
        }
    };
    let file = match File::open(archive) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("cannot open archive: {e}");
            return ExitCode::from(2);
        }
    };
    let result = inspect_and_admit(BufReader::new(file), &policy, &|| false);
    // Peak resident memory of this process since exec, for footprint evidence.
    if std::env::var_os("LIMEOS_REPORT_HWM").is_some() {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        if let Some(line) = status.lines().find(|l| l.starts_with("VmHWM:")) {
            eprintln!("{line}");
        }
    }
    match result {
        Ok(manifest) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&manifest).unwrap_or_default()
            );
            ExitCode::SUCCESS
        }
        Err(rejection) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&rejection).unwrap_or_default()
            );
            ExitCode::from(1)
        }
    }
}
