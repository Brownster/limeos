use args::{Usage, is_help};
use limeos_contracts::{CoreRequest, CoreResponse, Enrollment, VERSION};
use std::io::Read;
mod args;
mod ceiling_check;
mod job;
mod migrate;
#[tokio::main(flavor = "current_thread")]
async fn main() {
    // A shipped process never runs on the soak's simulated clock or hook.
    limeos_persistence::refuse_test_support();
    if let Err(e) = run().await {
        eprintln!("{e}");
        // 2: malformed command line, nothing done; 1: refused or failed.
        std::process::exit(if e.is::<Usage>() { 2 } else { 1 });
    }
}

/// What a command line asks for, decided before anything is touched.
#[derive(Debug, PartialEq, Eq)]
enum Route<'a> {
    Version,
    Help,
    CheckCeiling(&'a str, &'a str, Option<&'a str>),
    CheckConfig(&'a str),
    Migrate(&'a str, &'a [String]),
    Job(&'a str, &'a [String]),
    Core(&'a str, &'a str),
}

/// `--help`, `-h` and `--version` count only in command position, never as
/// another option's value.
fn route(args: &[String]) -> Result<Route<'_>, Usage> {
    let invalid = || Usage("invalid arguments; run limeosctl --help".into());
    match args {
        [] => return Ok(Route::Help),
        [first, ..] if first == "--version" => return Ok(Route::Version),
        [first, ..] if is_help(first) => return Ok(Route::Help),
        [cmd, file, uid] if cmd == "check-ceiling" => {
            return Ok(Route::CheckCeiling(file, uid, None));
        }
        [cmd, file, uid, role] if cmd == "check-ceiling" => {
            return Ok(Route::CheckCeiling(file, uid, Some(role.as_str())));
        }
        [cmd, file] if cmd == "check-config" => return Ok(Route::CheckConfig(file)),
        [cmd, ..] if cmd == "check-ceiling" || cmd == "check-config" => return Err(invalid()),
        _ => {}
    }
    let (socket, rest) = match args.split_first() {
        Some((flag, rest)) if flag == "--socket" => {
            let path = args::value(flag, rest.first())?;
            (path.as_str(), rest.get(1..).unwrap_or_default())
        }
        _ => (DEFAULT_SOCKET, args),
    };
    match rest {
        [] => Err(invalid()),
        [first, ..] if is_help(first) => Ok(Route::Help),
        [cmd, rest @ ..] if cmd == "migrate" => Ok(Route::Migrate(socket, rest)),
        [cmd, rest @ ..] if cmd == "job" => Ok(Route::Job(socket, rest)),
        [cmd] if matches!(cmd.as_str(), "status" | "bootstrap" | "enroll") => {
            Ok(Route::Core(socket, cmd))
        }
        [cmd] if cmd.starts_with('-') => Err(invalid()),
        [_] => Err(Usage("unknown command; run limeosctl --help".into())),
        _ => Err(invalid()),
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let (socket, command) = match route(&args)? {
        Route::Version => {
            println!("limeosctl {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Route::Help => {
            println!(
                "Usage: limeosctl [--socket PATH] status|bootstrap|enroll\nbootstrap: root-only, one-use token valid for 15 minutes.\nenroll: root-only JSON from stdin with token, username, password.\nNo passwords or tokens are accepted in command arguments.\nExit status: 0 done, 1 refused or failed, 2 invalid command line (nothing done).\n{}\n{}\n{}",
                ceiling_check::USAGE,
                migrate::USAGE,
                job::USAGE
            );
            return Ok(());
        }
        Route::CheckCeiling(file, uid, role) => {
            let role = ceiling_check::role(file, role)?;
            let uid = uid.parse::<u32>()?;
            return ceiling_check::check_file(std::path::Path::new(file), uid, role)
                .map_err(Into::into);
        }
        Route::CheckConfig(file) => {
            match limeos_persistence::config::read(std::path::Path::new(file))? {limeos_persistence::config::ConfigRead::Valid(_)=>return Ok(()),_=>return Err("Invalid or missing configuration. Repair /etc/limeos/core.json, then run dpkg --configure limeos. Original preserved.".into())}
        }
        Route::Migrate(socket, rest) => return migrate::run(socket, rest).await,
        Route::Job(socket, rest) => return job::run(socket, rest).await,
        Route::Core(socket, command) => (socket, command),
    };
    let request = match command {
        "status" => CoreRequest::Health { version: VERSION },
        "bootstrap" => CoreRequest::IssueBootstrap { version: VERSION },
        _ => {
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
    };
    let response = rpc(socket, &request).await?;
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}
const DEFAULT_SOCKET: &str = "/run/limeos-core/core.sock";
/// The ceiling bytes parse and suit the container role and `core_uid`.
#[cfg(test)]
fn ceiling_acceptable(bytes: &[u8], core_uid: u32) -> bool {
    serde_json::from_slice(bytes).is_ok_and(|ceiling| {
        ceiling_check::check(
            &ceiling,
            core_uid,
            limeos_executor_protocol::CeilingRole::Container,
        )
        .is_ok()
    })
}
async fn rpc(
    socket: &str,
    request: &CoreRequest,
) -> Result<CoreResponse, Box<dyn std::error::Error>> {
    tokio::time::timeout(limeos_contracts::RPC_DEADLINE, async {
        let mut stream = tokio::net::UnixStream::connect(socket).await?;
        limeos_contracts::write_frame(&mut stream, request).await?;
        let response: CoreResponse = limeos_contracts::read_frame(&mut stream).await?;
        if let CoreResponse::Error(error) = response {
            return Err(error.message.into());
        }
        Ok::<_, Box<dyn std::error::Error>>(response)
    })
    .await
    .map_err(|_| "local request timed out")?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_and_version_count_only_in_command_position() {
        assert_eq!(route(&args(&["--version"])).unwrap(), Route::Version);
        assert_eq!(route(&args(&["--help"])).unwrap(), Route::Help);
        assert_eq!(route(&args(&["-h"])).unwrap(), Route::Help);
        assert_eq!(route(&args(&[])).unwrap(), Route::Help);
        assert_eq!(
            route(&args(&["--socket", "/s", "--help"])).unwrap(),
            Route::Help
        );
        // Later on, they are handed on as ordinary words for the subcommand to judge.
        let resolve = args(&["job", "resolve", "j", "--reason", "--version"]);
        assert_eq!(
            route(&resolve).unwrap(),
            Route::Job(DEFAULT_SOCKET, &resolve[1..])
        );
        let scan = args(&["migrate", "scan", "--root", "--help"]);
        assert_eq!(
            route(&scan).unwrap(),
            Route::Migrate(DEFAULT_SOCKET, &scan[1..])
        );
        assert!(route(&args(&["status", "--version"])).is_err());
        assert!(route(&args(&["status", "--help"])).is_err());
    }

    #[test]
    fn the_socket_needs_a_real_value_and_commands_take_no_extra_words() {
        let ok = args(&["--socket", "/run/x.sock", "status"]);
        assert_eq!(route(&ok).unwrap(), Route::Core("/run/x.sock", "status"));
        assert_eq!(
            route(&args(&["enroll"])).unwrap(),
            Route::Core(DEFAULT_SOCKET, "enroll")
        );
        for bad in [
            vec!["--socket"],
            vec!["--socket", "--version"],
            vec!["--socket", "-h", "status"],
            vec!["--socket", "/s"],
            vec!["status", "extra"],
            vec!["bootstrap", "--force"],
            vec!["frobnicate"],
            vec!["--frobnicate"],
            vec!["check-ceiling", "/etc/x.json"],
            vec!["check-config"],
        ] {
            assert!(route(&args(&bad)).is_err(), "{bad:?}");
        }
    }
}
