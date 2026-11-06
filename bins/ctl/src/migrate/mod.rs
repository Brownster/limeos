//! Cutover tooling. Every subcommand is read-only against the frozen app.
mod catalog;
mod ceiling;
mod scan;
mod users;

use std::path::{Path, PathBuf};

pub const USAGE: &str = "migrate scan [--root DIR] [--etc-dir P] [--state-dir P] [--stacks-dir P] [--checkout P] [--json]\n  read-only, root-only inventory; classifies imported|deferred|unsupported.\nmigrate import-users --env-file PATH\n  root-only; hashes go to the core over its socket, never argv.\nmigrate propose-ceiling --docker-ps FILE --stacks DIR --core-uid UID --out FILE\n  offline; writes a deny-by-default container ceiling, never overwrites.\nmigrate propose-catalog --out FILE [--stacks DIR] [--install]\n  root-only, read-only against Docker and DIR (default /opt/stacks); writes a PROPOSED\n  stack management catalog from Compose labels; --install also places it root-owned.";

fn under(root: &Path, path: &str) -> PathBuf {
    root.join(path.trim_start_matches('/'))
}

fn scan_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (mut root, mut etc, mut state, mut stacks) = (
        "/".to_owned(),
        "/etc/limeos".to_owned(),
        "/var/lib/limeos".to_owned(),
        "/opt/stacks".to_owned(),
    );
    let (mut checkout, mut json) = (None, false);
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        if flag == "--json" {
            json = true;
            continue;
        }
        let value = it.next().ok_or("missing value for a scan flag")?.clone();
        match flag.as_str() {
            "--root" => root = value,
            "--etc-dir" => etc = value,
            "--state-dir" => state = value,
            "--stacks-dir" => stacks = value,
            "--checkout" => checkout = Some(value),
            _ => return Err("unknown scan argument; run limeosctl --help".into()),
        }
    }
    if rustix::process::geteuid().as_raw() != 0 {
        return Err("migrate scan is root-only so it can read every state file".into());
    }
    let root = Path::new(&root);
    let mut locations = vec![("etc", etc), ("state", state), ("stacks", stacks)];
    if let Some(checkout) = checkout {
        locations.push(("checkout", checkout));
    }
    let locations: Vec<_> = locations
        .into_iter()
        .map(|(label, path)| scan::Location {
            label,
            root: under(root, &path),
            display: PathBuf::from(path),
        })
        .collect();
    let report = scan::scan(&locations, scan::Limits::default());
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report.render_text());
    }
    if !report.verification.changed.is_empty() {
        return Err("an original file changed during the scan; do not trust this report".into());
    }
    if !report.complete {
        return Err("the scan is incomplete (see problems); the report lists what was read".into());
    }
    Ok(())
}

pub async fn run(socket: &str, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match args.split_first() {
        Some((cmd, rest)) if cmd == "scan" => scan_command(rest),
        Some((cmd, rest)) if cmd == "import-users" => {
            if rustix::process::geteuid().as_raw() != 0 {
                return Err("migrate import-users is root-only".into());
            }
            users::run(socket, rest).await
        }
        Some((cmd, rest)) if cmd == "propose-ceiling" => ceiling::run(rest),
        Some((cmd, rest)) if cmd == "propose-catalog" => catalog::run(rest),
        _ => Err(format!("usage:\n{USAGE}").into()),
    }
}
