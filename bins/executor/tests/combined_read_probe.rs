//! Opt-in guest probe for genuine public libraries. This is not a runtime command.
use limeos_executor_container::docker::{Docker, process_evidence::EnginePeerPolicy};
use limeos_executor_storage::{
    inspect_container_dependencies, inspect_container_sources, inspect_storage_inventory,
    processes::{RunningContainerProcessBinding, inspect_running_container_processes},
    read_contract,
};
use std::{
    ffi::OsStr,
    io::{self, Write},
    path::Path,
    time::Duration,
};

const PREFIX: &[u8] = b"LIMEOS_RW040_PROBE_JSON=";
const EVENT_BYTES: usize = 256 * 1024;
const CASE_BYTES: usize = 32;
const PATH_BYTES: usize = 512;
const CASES: [&str; 7] = [
    "contract",
    "engine",
    "engine-processes",
    "engine-sources",
    "storage",
    "engine-dependencies",
    "combined",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    Contract,
    Engine,
    Processes,
    Sources,
    Storage,
    Dependencies,
    Combined,
    Invalid,
}
impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Engine => "engine",
            Self::Processes => "engine-processes",
            Self::Sources => "engine-sources",
            Self::Storage => "storage",
            Self::Dependencies => "engine-dependencies",
            Self::Combined => "combined",
            Self::Invalid => "invalid",
        }
    }
}

struct Config<'a> {
    case: Case,
    socket: &'a Path,
    contract: Option<&'a Path>,
    pause_ms: u64,
}
fn text<'a>(
    value: Option<&'a OsStr>,
    default: &'a str,
    limit: usize,
) -> Result<&'a str, &'static str> {
    let Some(value) = value else {
        return Ok(default);
    };
    if value.len() > limit {
        return Err("input_limit");
    }
    value.to_str().ok_or("non_utf8_input")
}
fn config<'a>(
    case: Option<&'a OsStr>,
    socket: Option<&'a OsStr>,
    contract: Option<&'a OsStr>,
    pause: Option<&'a OsStr>,
) -> Result<Config<'a>, &'static str> {
    let case = match text(case, "contract", CASE_BYTES)? {
        "contract" => Case::Contract,
        "engine" => Case::Engine,
        "engine-processes" => Case::Processes,
        "engine-sources" => Case::Sources,
        "storage" => Case::Storage,
        "engine-dependencies" => Case::Dependencies,
        "combined" => Case::Combined,
        _ => return Err("unknown_case"),
    };
    let socket = Path::new(text(socket, "/run/docker.sock", PATH_BYTES)?);
    let contract = contract
        .map(|value| text(Some(value), "", PATH_BYTES).map(Path::new))
        .transpose()?;
    if !socket.is_absolute() || contract.is_some_and(|path| !path.is_absolute()) {
        return Err("absolute_path_required");
    }
    if case == Case::Dependencies && contract.is_none() {
        return Err("contract_required");
    }
    let pause = text(pause, "0", 4)?;
    if pause.is_empty() || !pause.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalid_pause");
    }
    let pause_ms = pause.parse().map_err(|_| "invalid_pause")?;
    if pause_ms > 5000 {
        return Err("invalid_pause");
    }
    Ok(Config {
        case,
        socket,
        contract,
        pause_ms,
    })
}

struct Buffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Buffer {
    fn new(limit: usize) -> io::Result<Self> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(limit)
            .map_err(|_| io::Error::other("probe event allocation failed"))?;
        Ok(Self { bytes, limit })
    }
}
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::Error::other("probe event byte limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn emit(
    output: &mut impl Write,
    case: Case,
    phase: &str,
    status: &str,
    stage: &str,
    error: &serde_json::Value,
    observations: impl FnOnce(&mut Buffer) -> io::Result<()>,
) -> io::Result<()> {
    // Build a complete bounded event before writing any prefix. Snapshots are
    // serialized directly from borrowed owners, without unbounded to_value/to_vec.
    let mut event = Buffer::new(EVENT_BYTES)?;
    event.write_all(b"{\"version\":1,\"case\":")?;
    serde_json::to_writer(&mut event, case.name())?;
    event.write_all(b",\"phase\":")?;
    serde_json::to_writer(&mut event, phase)?;
    event.write_all(b",\"status\":")?;
    serde_json::to_writer(&mut event, status)?;
    event.write_all(b",\"stage\":")?;
    serde_json::to_writer(&mut event, stage)?;
    event.write_all(b",\"error\":")?;
    serde_json::to_writer(&mut event, error)?;
    write!(
        event,
        ",\"effective_uid\":{},\"observations\":{{",
        rustix::process::geteuid().as_raw()
    )?;
    observations(&mut event)?;
    event.write_all(b"}}")?;
    // The leading newline prevents libtest's progress text sharing the event line.
    output.write_all(b"\n")?;
    output.write_all(PREFIX)?;
    output.write_all(&event.bytes)?;
    output.write_all(b"\n")?;
    output.flush()
}
fn refusal(
    output: &mut impl Write,
    case: Case,
    stage: &str,
    library: &str,
    code: serde_json::Value,
) -> io::Result<()> {
    emit(
        output,
        case,
        "final",
        "refused",
        stage,
        &serde_json::json!({"library":library,"code":code}),
        |_| Ok(()),
    )
}
async fn pause(config: &Config<'_>) {
    tokio::time::sleep(Duration::from_millis(config.pause_ms)).await;
}

async fn run(config: &Config<'_>, output: &mut impl Write) -> io::Result<()> {
    if matches!(config.case, Case::Contract | Case::Combined) {
        return emit(
            output,
            config.case,
            "final",
            if config.case == Case::Contract {
                "ok"
            } else {
                "blocked"
            },
            "prerequisites",
            &serde_json::Value::Null,
            |event| {
                event.write_all(b"\"available_cases\":")?;
                serde_json::to_writer(&mut *event, &CASES)?;
                event.write_all(b",\"combined_worker\":\"unavailable\",\"standalone_supervision\":\"external_required\"")
            },
        );
    }
    if config.case == Case::Storage {
        let storage = match inspect_storage_inventory().await {
            Ok(owner) => owner,
            Err(error) => {
                return refusal(
                    output,
                    config.case,
                    "storage.collect",
                    "storage",
                    serde_json::json!(error),
                );
            }
        };
        let report = |event: &mut Buffer| {
            event.write_all(b"\"storage\":")?;
            serde_json::to_writer(event, storage.snapshot()).map_err(io::Error::other)
        };
        emit(
            output,
            config.case,
            "collected",
            "ok",
            "storage.collected",
            &serde_json::Value::Null,
            report,
        )?;
        pause(config).await;
        if let Err(error) = storage.revalidate().await {
            return refusal(
                output,
                config.case,
                "storage.revalidate",
                "storage",
                serde_json::json!(error),
            );
        }
        return emit(
            output,
            config.case,
            "final",
            "ok",
            "storage.revalidated",
            &serde_json::Value::Null,
            report,
        );
    }
    // Production protected contract reading happens before any Engine collection.
    let contract = if config.case == Case::Dependencies {
        match read_contract(config.contract.expect("validated required contract")) {
            Ok(contract) => Some(contract),
            Err(error) => {
                return refusal(
                    output,
                    config.case,
                    "contract.read",
                    "storage",
                    serde_json::json!(error),
                );
            }
        }
    } else {
        None
    };
    let engine = match Docker::new(config.socket.into())
        .inspect_storage_processes(EnginePeerPolicy::root())
        .await
    {
        Ok(owner) => owner,
        Err(error) => {
            return refusal(
                output,
                config.case,
                "engine.collect",
                "engine",
                serde_json::json!(error.0),
            );
        }
    };
    let processes =
        if config.case == Case::Processes {
            let mut bindings = Vec::new();
            bindings
                .try_reserve_exact(engine.running().len())
                .map_err(|_| io::Error::other("probe bindings allocation failed"))?;
            bindings.extend(engine.running().iter().map(|binding| {
                RunningContainerProcessBinding {
                    container: binding.container().clone(),
                    pid: binding.pid(),
                }
            }));
            match inspect_running_container_processes(engine.declarations(), &bindings) {
                Ok(owner) => Some(owner),
                Err(error) => {
                    return refusal(
                        output,
                        config.case,
                        "processes.collect",
                        "storage",
                        serde_json::json!(error),
                    );
                }
            }
        } else {
            None
        };
    let sources = if config.case == Case::Sources {
        match inspect_container_sources(engine.declarations()) {
            Ok(owner) => Some(owner),
            Err(error) => {
                return refusal(
                    output,
                    config.case,
                    "sources.collect",
                    "storage",
                    serde_json::json!(error),
                );
            }
        }
    } else {
        None
    };
    let dependencies = if let Some(contract) = &contract {
        match inspect_container_dependencies(contract, engine.declarations()).await {
            Ok(owner) => Some(owner),
            Err(error) => {
                return refusal(
                    output,
                    config.case,
                    "dependencies.collect",
                    "storage",
                    serde_json::json!(error),
                );
            }
        }
    } else {
        None
    };
    let report = |event: &mut Buffer| {
        event.write_all(b"\"engine\":")?;
        serde_json::to_writer(&mut *event, engine.snapshot())?;
        if let Some(owner) = &processes {
            event.write_all(b",\"processes\":")?;
            serde_json::to_writer(&mut *event, owner.snapshot())?;
        }
        if let Some(owner) = &sources {
            event.write_all(b",\"sources\":")?;
            serde_json::to_writer(&mut *event, owner.snapshot())?;
        }
        if let Some(owner) = &dependencies {
            event.write_all(b",\"dependencies\":")?;
            serde_json::to_writer(&mut *event, owner.snapshot())?;
        }
        Ok(())
    };
    emit(
        output,
        config.case,
        "collected",
        "ok",
        "owners.collected",
        &serde_json::Value::Null,
        report,
    )?;
    pause(config).await;
    // Never reacquire a new owner: both fresh GET validations use the retained
    // Engine's original lifetime. Namespace IDs remain local observations.
    if let Err(error) = engine.revalidate().await {
        return refusal(
            output,
            config.case,
            "engine.before_revalidate",
            "engine",
            serde_json::json!(error.0),
        );
    }
    let selected = if let Some(owner) = &processes {
        owner.revalidate()
    } else if let Some(owner) = &sources {
        owner.revalidate()
    } else if let Some(owner) = &dependencies {
        owner.revalidate().await
    } else {
        Ok(())
    };
    if let Err(error) = selected {
        return refusal(
            output,
            config.case,
            "owner.revalidate",
            "storage",
            serde_json::json!(error),
        );
    }
    if config.case != Case::Engine {
        if let Err(error) = engine.revalidate().await {
            return refusal(
                output,
                config.case,
                "engine.after_revalidate",
                "engine",
                serde_json::json!(error.0),
            );
        }
    }
    emit(
        output,
        config.case,
        "final",
        "ok",
        "owners.revalidated",
        &serde_json::Value::Null,
        report,
    )
}

#[tokio::test(flavor = "current_thread")]
async fn qualification_probe() -> io::Result<()> {
    let case = std::env::var_os("LIMEOS_RW040_PROBE_CASE");
    let socket = std::env::var_os("LIMEOS_RW040_PROBE_SOCKET");
    let contract = std::env::var_os("LIMEOS_RW040_PROBE_CONTRACT");
    let pause = std::env::var_os("LIMEOS_RW040_PROBE_PAUSE_MS");
    let mut output = io::stdout();
    match config(
        case.as_deref(),
        socket.as_deref(),
        contract.as_deref(),
        pause.as_deref(),
    ) {
        Ok(config) => run(&config, &mut output).await,
        Err(code) => emit(
            &mut output,
            Case::Invalid,
            "final",
            "invalid_input",
            "input",
            &serde_json::json!({"library":"probe","code":code}),
            |_| Ok(()),
        ),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn default_contract_and_missing_worker_have_one_final_event() {
    for (case, status) in [("contract", "ok"), ("combined", "blocked")] {
        let config = config(Some(OsStr::new(case)), None, None, None).unwrap();
        let mut output = Vec::new();
        run(&config, &mut output).await.unwrap();
        let line = std::str::from_utf8(&output).unwrap().trim();
        let value: serde_json::Value = serde_json::from_str(
            line.strip_prefix(std::str::from_utf8(PREFIX).unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["phase"], "final");
        assert_eq!(value["status"], status);
        assert_eq!(value["observations"]["combined_worker"], "unavailable");
        assert_eq!(line.matches("LIMEOS_RW040_PROBE_JSON=").count(), 1);
    }
}

#[test]
fn invalid_inputs_refuse_before_any_host_collection() {
    use std::os::unix::ffi::OsStrExt;
    for raw in [
        OsStr::new("unknown"),
        OsStr::new("engine-processes-extra-unknown-case"),
        OsStr::from_bytes(b"\xff"),
    ] {
        assert!(config(Some(raw), None, None, None).is_err());
    }
    assert!(
        config(
            Some(OsStr::new("engine")),
            Some(OsStr::new("relative.sock")),
            None,
            None
        )
        .is_err()
    );
    assert!(
        config(
            None,
            Some(OsStr::new(&format!("/{}", "x".repeat(PATH_BYTES)))),
            None,
            None
        )
        .is_err()
    );
    assert!(config(Some(OsStr::new("engine-dependencies")), None, None, None).is_err());
    for pause in ["", "-1", "1.5", "5001", "00000"] {
        assert!(config(None, None, None, Some(OsStr::new(pause))).is_err());
    }
    assert_eq!(
        config(None, None, None, Some(OsStr::new("5000")))
            .unwrap()
            .pause_ms,
        5000
    );
}

#[test]
fn event_limit_emits_no_partial_prefix_or_success() {
    let mut output = Vec::new();
    let result = emit(
        &mut output,
        Case::Contract,
        "final",
        "ok",
        "limit",
        &serde_json::Value::Null,
        |event| event.write_all(&vec![b'x'; EVENT_BYTES]),
    );
    assert!(result.is_err());
    assert!(output.is_empty());
    let mut buffer = Buffer::new(3).unwrap();
    buffer.write_all(b"abc").unwrap();
    assert!(buffer.write_all(b"d").is_err());
    assert_eq!(buffer.bytes, b"abc");
}

#[test]
fn output_and_flush_errors_are_fatal() {
    struct BrokenOutput {
        fail_flush: bool,
    }
    impl Write for BrokenOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail_flush {
                Ok(bytes.len())
            } else {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }
    for fail_flush in [false, true] {
        assert!(
            emit(
                &mut BrokenOutput { fail_flush },
                Case::Contract,
                "final",
                "ok",
                "output",
                &serde_json::Value::Null,
                |_| Ok(())
            )
            .is_err()
        );
    }
}
