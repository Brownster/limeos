//! Opt-in supplied combined-reader seam. No installed command or UID override.
use limeos_executor::{
    read_launch::{ConfigurationSelection, LoadedRole, ProtectedLaunch},
    read_supervision::{self, Supervisor, WorkerBootstrap},
};
use rustix::process::Pid;
use std::io::{self, Write};
fn emit(event: &[u8]) -> io::Result<()> {
    let mut output = io::stdout().lock();
    output.write_all(b"LIMEOS_RW040_COMBINED_JSON=")?;
    output.write_all(event)?;
    output.write_all(b"\n")?;
    output.flush()
}
struct Event(Vec<u8>);
impl Write for Event {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > 264 * 1024 - self.0.len() {
            return Err(io::Error::other("event limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let argument = args.next();
    if args.next().is_some() || argument.as_ref().is_some_and(|value| value.len() > 16) {
        return Err("role input limit".into());
    }
    let role = match argument.as_deref() {
        None => "contract",
        Some(value) => value.to_str().ok_or("non-UTF8 role")?,
    };
    match role {
        "contract" => {
            emit(br#"{"version":1,"phase":"final","status":"blocked","stage":"prerequisites","fixed_image":"/usr/lib/limeos/limeos-combined-read-probe","installed_qualification":false}"#)?;
        }
        "probe" => {
            let selection = ConfigurationSelection::from_environment()
                .map_err(|error| format!("selection: {error:?}"))?;
            let launch = match ProtectedLaunch::prepare(selection.paths()) {
                Ok(launch) => launch,
                Err(error) => {
                    emit(format!("{{\"version\":1,\"phase\":\"final\",\"status\":\"refused\",\"stage\":\"prepare\",\"error\":\"{error:?}\"}}").as_bytes())?;
                    return Ok(());
                }
            };
            let manager = Supervisor::default();
            let result = launch.start(&manager).and_then(|ticket| ticket.wait());
            // Build one complete bounded final event before any output prefix.
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(264 * 1024)?;
            let mut event = Event(bytes);
            event.write_all(b"{\"version\":1,\"phase\":\"final\",")?;
            match result {
                Ok(report) => {
                    event.write_all(
                        b"\"status\":\"ok\",\"stage\":\"supervised\",\"observations\":",
                    )?;
                    event.write_all(report.as_bytes())?;
                }
                Err(error) => {
                    write!(
                        &mut event,
                        "\"status\":\"refused\",\"stage\":\"supervised\",\"error\":\"{error:?}\""
                    )?;
                }
            }
            event.write_all(b",\"image\":")?;
            serde_json::to_writer(&mut event, launch.image())?;
            event.write_all(b",\"selection\":")?;
            serde_json::to_writer(&mut event, launch.selection())?;
            event.write_all(b"}")?;
            emit(&event.0)?;
        }
        "guardian" => {
            let loaded = LoadedRole::current().map_err(|error| format!("image: {error:?}"))?;
            read_supervision::guardian(loaded.worker_command())
                .map_err(|error| format!("guardian: {error:?}"))?;
        }
        "worker" => {
            let loaded = LoadedRole::current().map_err(|error| format!("image: {error:?}"))?;
            let parent = std::env::var_os("LIMEOS_READ_GUARDIAN_PID")
                .filter(|value| value.len() <= 10)
                .and_then(|value| value.into_string().ok())
                .and_then(|value| value.parse().ok())
                .and_then(Pid::from_raw)
                .ok_or("guardian PID")?;
            let boot = WorkerBootstrap::establish(parent)
                .map_err(|error| format!("bootstrap: {error:?}"))?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            match runtime.block_on(boot.collect(loaded.paths())) {
                Ok(report) => read_supervision::write_worker_report(report.as_bytes()),
                Err(error) => {
                    if !rustix::process::geteuid().is_root()
                        && !matches!(
                            error,
                            limeos_executor::read_collection::Refusal {
                                stage: limeos_executor::read_collection::Stage::Collect(
                                    limeos_executor::read_budget::Phase::Configuration
                                ),
                                failure: limeos_executor::read_collection::Failure::Storage(
                                    limeos_executor_storage::Failure::Unavailable
                                ),
                            }
                        )
                    {
                        return Err(
                            "nonroot request did not reach the genuine configuration guard".into(),
                        );
                    }
                    read_supervision::write_worker_refusal()
                }
            }
            .map_err(|error| format!("worker output: {error:?}"))?;
        }
        _ => return Err("unknown closed role".into()),
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("combined probe failed: {error}");
        std::process::exit(80);
    }
}
