//! Fixed package worker. Its only inputs are sealed roles and protected selection.
use limeos_executor::{
    read_launch::LoadedStackRole,
    read_supervision::{self, WorkerBootstrap},
    stack_read,
};
use rustix::process::Pid;

fn run() -> Result<(), ()> {
    let mut args = std::env::args_os().skip(1);
    let role = args.next().ok_or(())?;
    if args.next().is_some() || role.len() > 8 {
        return Err(());
    }
    let loaded = LoadedStackRole::current().map_err(|_| ())?;
    match role.to_str() {
        Some("guardian") => {
            stack_read::admit_guardian().map_err(|_| ())?;
            read_supervision::guardian(loaded.worker_command()).map_err(|_| ())
        }
        Some("worker") => {
            let parent = std::env::var_os("LIMEOS_READ_GUARDIAN_PID")
                .filter(|v| v.len() <= 10)
                .and_then(|v| v.into_string().ok())
                .and_then(|v| v.parse().ok())
                .and_then(Pid::from_raw)
                .ok_or(())?;
            let boot = WorkerBootstrap::establish(parent).map_err(|_| ())?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .max_blocking_threads(1)
                .build()
                .map_err(|_| ())?;
            match runtime.block_on(boot.collect_stacks(loaded.catalog())) {
                Ok(report) => read_supervision::write_worker_report(&report),
                Err(_) => read_supervision::write_worker_refusal(),
            }
            .map_err(|_| ())
        }
        _ => Err(()),
    }
}
fn main() {
    if run().is_err() {
        std::process::exit(80);
    }
}
