use limeos_executor::{storage_mount_admission, storage_mount_supervision};
use limeos_executor_container::{ReceiptStore, docker::Docker};
use std::{path::Path, sync::Arc};
use tokio::{
    net::UnixListener,
    sync::{Mutex, Semaphore},
};
mod storage_runtime;
mod storage_targets;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    // The sealed guardian image bootstraps before Tokio or another thread.
    // It validates its own fixed image, parent, namespace and private transport.
    if args.len() == 2 && args[1] == "storage-mount-guardian" {
        if storage_mount_supervision::guardian().is_err() {
            std::process::exit(1);
        }
        return;
    }
    run_in_runtime(args.get(3).map(String::as_str), async_main());
}
fn run_in_runtime(role: Option<&str>, work: impl std::future::Future<Output = ()>) {
    let mut builder = tokio::runtime::Builder::new_current_thread();
    builder.enable_all();
    if matches!(role, Some("storage-mounts" | "storage")) {
        builder.max_blocking_threads(1);
    }
    let runtime = builder.build().expect("executor runtime");
    // A spare blocking worker retires while idle, changing the exact host task
    // census. Unix I/O and subprocess pipes need no startup blocking task.
    runtime.block_on(work);
}
async fn async_main() {
    let args: Vec<_> = std::env::args().collect();
    if args.iter().any(|s| matches!(s.as_str(), "--help" | "-h")) {
        println!(
            "Usage: limeos-executor POLICY SOCKET host|container|storage|storage-targets|storage-mounts [RECEIPT_DIRECTORY [read-only]]\n       limeos-executor storage-check|storage-wait --plan ROOT_OWNED_JSON\n       limeos-executor storage-inventory\n       limeos-executor storage-target-plan --contract ROOT_OWNED_JSON\n       limeos-executor storage-prepare-targets --plan ROOT_OWNED_JSON\n       limeos-executor storage-target-receipt|storage-reconcile-targets --action ACTION_ID\n       limeos-executor storage-reconcile-targets --plan ROOT_OWNED_JSON\n\nRoot operator preparation creates empty mount target directories only.\nThe storage-mounts service executes approved nonpersistent ext4 mounts.\nReconciliation observes targets and updates receipts. Other storage commands inspect.\nStdout is JSON; errors go to stderr.\nExit: 0 verified, 1 unsafe/unavailable/uncertain, 2 invalid plan/usage, 3 deadline expired."
        );
        return;
    }
    if args.get(1).is_some_and(|s| s == "--version") && args.len() == 2 {
        println!("limeos-executor {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.get(1).is_some_and(|s| s.starts_with("storage-")) {
        storage_cli(&args).await;
        return;
    }
    tracing_subscriber::fmt().json().with_target(false).init();
    if let Err(error) = run().await {
        tracing::error!(%error, "executor configuration or runtime failure");
        std::process::exit(1);
    }
}
async fn storage_cli(args: &[String]) {
    use limeos_executor_storage::{Failure, read_plan, verify, wait};
    let result = async {
        if args.len() == 4 && args[1] == "storage-reconcile-targets" && args[2] == "--action" {
            if std::env::current_exe().map_err(|_| Failure::Unavailable)?
                != Path::new("/usr/lib/limeos/limeos-executor")
            {
                return Err(Failure::Unavailable);
            }
            let value = tokio::time::timeout(
                std::time::Duration::from_secs(12),
                limeos_executor_storage::reconcile_targets(&args[3]),
            )
            .await
            .map_err(|_| Failure::TimedOut)??;
            return serde_json::to_value(value).map_err(|_| Failure::Unavailable);
        }
        if args.len() == 4 && args[1] == "storage-target-receipt" && args[2] == "--action" {
            return serde_json::to_value(limeos_executor_storage::target_receipt(&args[3])?)
                .map_err(|_| Failure::Unavailable);
        }
        if args.len() == 4 && args[1] == "storage-reconcile-targets" && args[2] == "--plan" {
            if std::env::current_exe().map_err(|_| Failure::Unavailable)?
                != Path::new("/usr/lib/limeos/limeos-executor")
            {
                return Err(Failure::Unavailable);
            }
            let plan = limeos_executor_storage::read_target_plan(Path::new(&args[3]))?;
            let value = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                limeos_executor_storage::reconcile_target_plan(&plan),
            )
            .await
            .map_err(|_| Failure::TimedOut)??;
            return serde_json::to_value(value).map_err(|_| Failure::Unavailable);
        }
        if args.len() == 4 && args[1] == "storage-target-plan" && args[2] == "--contract" {
            let contract = limeos_executor_storage::read_contract(Path::new(&args[3]))?;
            let value = tokio::time::timeout(
                std::time::Duration::from_secs(12),
                limeos_executor_storage::target_plan(&contract),
            )
            .await
            .map_err(|_| Failure::TimedOut)??;
            return serde_json::to_value(value).map_err(|_| Failure::Unavailable);
        }
        if args.len() == 4 && args[1] == "storage-prepare-targets" && args[2] == "--plan" {
            // The shadow payload is read-only, including root operator commands.
            if std::env::current_exe().map_err(|_| Failure::Unavailable)?
                != Path::new("/usr/lib/limeos/limeos-executor")
            {
                return Err(Failure::Unavailable);
            }
            let plan = limeos_executor_storage::read_target_plan(Path::new(&args[3]))?;
            let value = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                limeos_executor_storage::prepare_targets(&plan),
            )
            .await
            .map_err(|_| Failure::TimedOut)??;
            let verified = matches!(
                value.state,
                limeos_executor_storage::PreparationState::Verified
            );
            println!(
                "{}",
                serde_json::to_string(&value).map_err(|_| Failure::Unavailable)?
            );
            std::process::exit(if verified { 0 } else { 1 });
        }
        if args.len() == 2 && args[1] == "storage-inventory" {
            let value = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                limeos_executor_storage::inventory(),
            )
            .await
            .map_err(|_| Failure::TimedOut)??;
            return serde_json::to_value(value).map_err(|_| Failure::Unavailable);
        }
        if args.len() != 4
            || args[2] != "--plan"
            || !matches!(args[1].as_str(), "storage-check" | "storage-wait")
        {
            return Err(Failure::InvalidPlan);
        }
        let plan = read_plan(Path::new(&args[3]))?;
        let value = if args[1] == "storage-wait" {
            wait(&plan).await
        } else {
            tokio::time::timeout(
                std::time::Duration::from_secs(u64::from(plan.timeout_seconds)),
                verify(&plan),
            )
            .await
            .map_err(|_| Failure::TimedOut)?
        }?;
        serde_json::to_value(value).map_err(|_| Failure::Unavailable)
    }
    .await;
    match result {
        Ok(snapshot) => println!(
            "{}",
            serde_json::to_string(&snapshot).expect("serializable storage snapshot")
        ),
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::json!({"error": error, "message": error.message()})
            );
            std::process::exit(match error {
                Failure::InvalidPlan => 2,
                Failure::TimedOut => 3,
                _ => 1,
            });
        }
    }
}
async fn run() -> std::io::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(3).is_some_and(|s| s == "storage-mounts") {
        return storage_runtime::run(&args).await;
    }
    if args.get(3).is_some_and(|s| s == "storage-targets") {
        return storage_targets::run(&args).await;
    }
    if !(4..=6).contains(&args.len()) {
        return Err(std::io::Error::other(
            "usage: limeos-executor POLICY SOCKET host|container|storage [RECEIPT_DIRECTORY [read-only]]",
        ));
    }
    use limeos_executor_protocol::CeilingRole;
    let read_only = args.len() == 6;
    if read_only && args[5] != "read-only" {
        return Err(std::io::Error::other(
            "invalid executor arguments for this role",
        ));
    }
    let role = match args[3].as_str() {
        "host" => CeilingRole::Host,
        "container" if read_only => CeilingRole::ContainerReadOnly,
        "container" => CeilingRole::Container,
        "storage" if args.len() == 4 => CeilingRole::StorageReader,
        "storage" => {
            return Err(std::io::Error::other(
                "invalid executor arguments for this role",
            ));
        }
        _ => return Err(std::io::Error::other("invalid executor kind")),
    };
    // Read again twice for every request (`ceiling.rs`); this first read only
    // decides what the process sets up, and refuses to start on a bad file.
    let ceilings = Arc::new(limeos_executor::ceiling::CeilingFile::installed(
        Path::new(&args[1]),
        role,
    ));
    let ceiling = ceilings
        .load()
        .map_err(|refusal| std::io::Error::other(format!("ceiling {}: {refusal}", args[1])))?;
    let storage_reader = role == CeilingRole::StorageReader;
    let source = match role {
        CeilingRole::Host | CeilingRole::StorageReader => limeos_contracts::Source::Host,
        CeilingRole::Container | CeilingRole::ContainerReadOnly => limeos_contracts::Source::Docker,
    };
    let (receipts, epoch) = if ceiling.can_write() {
        let directory = args
            .get(4)
            .map(String::as_str)
            .unwrap_or("/var/lib/limeos/executors/container");
        let store = ReceiptStore::open(Path::new(directory))
            .map_err(|_| std::io::Error::other("unsafe or unavailable receipt store"))?;
        // Fixed for this open, which lasts as long as the process: answered
        // to `ReceiptEpoch` without taking the store's lock.
        let epoch: Arc<str> = store.epoch().into();
        (Some(Arc::new(Mutex::new(store))), Some(epoch))
    } else {
        (None, None)
    };
    let containers = Arc::new(limeos_executor::container_requests::ContainerRole {
        engine: Docker::new("/var/run/docker.sock".into()),
        receipts,
        epoch,
        docker: source == limeos_contracts::Source::Docker,
    });
    // An independent ceiling can disable collection entirely.
    let (observations, collecting) = match source {
        limeos_contracts::Source::Host if ceiling.allow_host_read => {
            (limeos_observations::start_host(), true)
        }
        limeos_contracts::Source::Docker if ceiling.allow_container_read => (
            limeos_observations::start_docker("/var/run/docker.sock".into()),
            true,
        ),
        _ => (limeos_observations::Cache::new(vec![source]), false),
    };
    let path = Path::new(&args[2]);
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        use std::os::unix::fs::FileTypeExt;
        if !meta.file_type().is_socket() || std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::other("socket conflict"));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o660))?;
    let permits = Arc::new(Semaphore::new(8));
    let storage_permit = limeos_executor::read_gate::Gate::default();
    let read_supervisor = Arc::new(limeos_executor::read_supervision::Supervisor::default());
    let stacks = limeos_observations::stacks::Cache::new();
    let stack_scan = storage_reader && ceiling.allow_stack_read;
    if stack_scan {
        let writer = stacks.clone();
        let gate = storage_permit.clone();
        let manager = read_supervisor.clone();
        let ceilings = ceilings.clone();
        let selection = limeos_executor::read_launch::StackSelection::installed()
            .map_err(|_| std::io::Error::other("invalid stack configuration selection"))?;
        tokio::spawn(async move {
            use limeos_executor::read_gate;
            loop {
                // A revoked or unreadable ceiling stops the stack scan too,
                // not only the answers.
                if !ceilings.load().is_ok_and(|c| c.allow_stack_read) {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    continue;
                }
                let manager = manager.clone();
                let selection = selection.clone();
                read_gate::refresh_stacks(&gate, read_gate::PERIODIC_WAIT, &writer, move |guard| {
                    limeos_executor::stack_read::refresh(selection, &manager, guard)
                })
                .await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });
    }
    // At most one inventory read may be stuck in the kernel at a time. With
    // one stranded the reader refuses inventories, serves everything else,
    // keeps its watchdog keep-alive (a restart would not free the read and
    // would abort the healthy reads in flight), and reports the stuck read in
    // its health answer, which core lists as degraded.
    let inventory_reads = Arc::new(limeos_executor::blocking_read::BlockingReads::new(1));
    let shared = Arc::new(limeos_executor::shared_role::SharedRole {
        ceilings,
        source,
        storage_reader,
        observations,
        collecting,
        containers,
        storage_permit,
        read_supervisor,
        stacks,
        stack_scan,
        inventory_reads,
    });
    // The container executor's receipt-store fsyncs run on this loop's thread
    // and can legitimately outlast any keep-alive interval, so it never takes
    // part in the watchdog (limeos-containerd.service says why).
    let mut watchdog = if source == limeos_contracts::Source::Docker {
        limeos_contracts::Watchdog::disabled()
    } else {
        limeos_contracts::Watchdog::from_env()
    };
    loop {
        let stream = limeos_executor::service::next_connection(&listener, &mut watchdog).await?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let shared = shared.clone();
        tokio::spawn(async move {
            let _permit = permit;
            shared.serve(stream).await;
        });
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::run_in_runtime;
    use std::{process::Command, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn task_ids() -> Vec<u32> {
        let mut tasks: Vec<_> = std::fs::read_dir("/proc/self/task")
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_str()
                    .unwrap()
                    .parse()
                    .unwrap()
            })
            .collect();
        tasks.sort_unstable();
        tasks
    }

    #[test]
    fn mount_bootstrap_keeps_its_thread_set_through_unix_and_child_io() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "runtime_tests::mount_runtime_child",
                "--test-threads=1",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "runtime probe failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("mount-runtime-probe-ok"));
    }

    #[test]
    #[ignore = "run in an isolated process by the runtime bootstrap regression"]
    fn mount_runtime_child() {
        let expected = task_ids();
        run_in_runtime(Some("storage-mounts"), async {
            assert_eq!(task_ids(), expected, "bootstrap added an idle worker");
            tokio::time::timeout(Duration::from_secs(5), async {
                let (mut left, mut right) = tokio::net::UnixStream::pair().unwrap();
                left.write_all(b"x").await.unwrap();
                let mut received = [0];
                right.read_exact(&mut received).await.unwrap();
                assert_eq!(&received, b"x");
                let mut child = tokio::process::Command::new("/bin/true")
                    .env_clear()
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .unwrap();
                let mut bytes = Vec::new();
                child
                    .stdout
                    .take()
                    .unwrap()
                    .take(1)
                    .read_to_end(&mut bytes)
                    .await
                    .unwrap();
                assert!(bytes.is_empty());
                assert!(child.wait().await.unwrap().success());
                assert_eq!(task_ids(), expected, "async I/O changed executor threads");
            })
            .await
            .unwrap();
        });
        assert_eq!(task_ids(), expected);
        println!("mount-runtime-probe-ok");
    }
}
