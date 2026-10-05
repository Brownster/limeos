use limeos_contracts::{CoreRequest, CoreResponse, Enrollment, VERSION};
use std::io::Read;
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("limeosctl {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "check-ceiling") && args.len() == 3 {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::symlink_metadata(&args[1])?;
        if !meta.is_file() || meta.uid() != 0 || meta.mode() & 0o022 != 0 || meta.len() > 65536 {
            return Err("Unsafe executor ceiling; preserve and repair the original file.".into());
        }
        let ceiling: limeos_executor_protocol::Ceiling =
            serde_json::from_slice(&std::fs::read(&args[1])?)
                .map_err(|_| "Invalid executor ceiling; preserve and repair the original file.")?;
        if !ceiling.configuration_valid() || ceiling.core_uid != args[2].parse::<u32>()? {
            return Err(
                "Executor ceiling identity/version mismatch; repair the original file.".into(),
            );
        }
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "check-config") && args.len() == 2 {
        match limeos_persistence::config::read(std::path::Path::new(&args[1]))? {limeos_persistence::config::ConfigRead::Valid(_)=>return Ok(()),_=>return Err("Invalid or missing configuration. Repair /etc/limeos/core.json, then run dpkg --configure limeos. Original preserved.".into())}
    }
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Usage: limeosctl [--socket PATH] status|bootstrap|enroll\nbootstrap: root-only, one-use token valid for 15 minutes.\nenroll: root-only JSON from stdin with token, username, password.\nNo passwords or tokens are accepted in command arguments."
        );
        return Ok(());
    }
    let (socket, command): (&str, &String) =
        if args.first().is_some_and(|s| s == "--socket") && args.len() == 3 {
            (&args[1], &args[2])
        } else if args.len() == 1 {
            ("/run/limeos-core/core.sock", &args[0])
        } else {
            return Err("invalid arguments; run limeosctl --help".into());
        };
    let request = match command.as_str() {
        "status" => CoreRequest::Health { version: VERSION },
        "bootstrap" => CoreRequest::IssueBootstrap { version: VERSION },
        "enroll" => {
            let mut input = String::new();
            std::io::stdin().take(16385).read_to_string(&mut input)?;
            if input.len() > 16384 {
                return Err("enrollment input too large".into());
            }
            let enrollment: Enrollment =
                serde_json::from_str(&input).map_err(|_| "invalid enrollment input")?;
            CoreRequest::Enroll {
                version: VERSION,
                enrollment,
            }
        }
        _ => return Err("unknown command; run limeosctl --help".into()),
    };
    tokio::time::timeout(limeos_contracts::RPC_DEADLINE, async {
        let mut stream = tokio::net::UnixStream::connect(socket).await?;
        limeos_contracts::write_frame(&mut stream, &request).await?;
        let response: CoreResponse = limeos_contracts::read_frame(&mut stream).await?;
        if let CoreResponse::Error(error) = response {
            return Err(error.message.into());
        }
        println!("{}", serde_json::to_string(&response)?);
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await
    .map_err(|_| "local request timed out")??;
    Ok(())
}
