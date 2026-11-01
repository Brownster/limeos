//! Native private fixtures exercise the production gate/parser/wait path.
//! They do not construct mount admission, root-image trust or installed proof.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let base = repository.join(".cache/a1-gated-process-fixtures");
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join(format!(
            "guardian-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    process: Process,
    marker: std::path::PathBuf,
    _directory: Scratch,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.process.child.kill();
        let _ = self.process.child.wait();
    }
}
impl Fixture {
    async fn start(mode: &str) -> Self {
        let directory = Scratch::new();
        let marker = directory.0.join("effect");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage_mount_supervision::tests::process_protocol::guardian_protocol_entry",
                "--ignored",
                "--nocapture",
            ])
            .env("LIMEOS_GATE_TEST_MODE", mode)
            .env("LIMEOS_GATE_TEST_MARKER", &marker)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let input = child.stdin.take().unwrap();
        // stderr carries the private fixture protocol so libtest stdout cannot
        // become a guardian frame. Production uses only the fixed role's stdout.
        let output = File::from(OwnedFd::from(child.stderr.take().unwrap()));
        nonblocking(&output).unwrap();
        let mut value = Self {
            process: Process {
                child,
                input: Some(input),
                output,
                _pidfd: process::pidfd_open(
                    Pid::from_raw(pid as i32).unwrap(),
                    PidfdFlags::empty(),
                )
                .unwrap(),
                identity: MountHelperIdentity {
                    pid,
                    start_time_ticks: 1,
                    mount_namespace_inode: 1,
                    cgroup: "/private-unit-fixture".into(),
                    image_digest: "f".repeat(64),
                },
                action: "a".repeat(64),
                digest: "b".repeat(64),
                bytes: vec![],
            },
            marker,
            _directory: directory,
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        while value.process.bytes.len() < READY.len() {
            pump(
                &mut value.process.output,
                &mut value.process.bytes,
                READY.len(),
            )
            .unwrap();
            assert!(Instant::now() < deadline);
            tokio::time::sleep(TICK).await;
        }
        assert_eq!(value.process.bytes, READY);
        value.process.bytes.clear();
        println!("LIMEOS_MOUNT_GATE_TEST pid={pid} mode={mode}");
        value
    }
    fn go_bytes(&self, mutate: impl FnOnce(&mut Go)) -> Vec<u8> {
        let mut go = Go {
            action: self.process.action.clone(),
            digest: self.process.digest.clone(),
            expires_at: now().unwrap() + 10,
            deadline_ticks: u64::MAX - 1,
        };
        mutate(&mut go);
        serde_json::to_vec(&go).unwrap()
    }
    fn go(&mut self, mutate: impl FnOnce(&mut Go)) {
        let bytes = self.go_bytes(mutate);
        let input = self.process.input.as_mut().unwrap();
        input
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .unwrap();
        input.write_all(&bytes).unwrap();
        input.flush().unwrap();
    }
}

#[test]
#[ignore = "private native protocol child, invoked by passing parent regressions"]
fn guardian_protocol_entry() {
    let mode = std::env::var("LIMEOS_GATE_TEST_MODE").unwrap();
    let input = io::stdin();
    nonblocking(&input).unwrap();
    let mut input = input.lock();
    let mut output = io::stderr().lock();
    output.write_all(READY).unwrap();
    output.flush().unwrap();
    let result =
        receive_go(&mut input, &"a".repeat(64), &"b".repeat(64), u64::MAX - 1).and_then(|go| {
            if parent_gone(&mut input)? {
                Err(Error(ErrorCode::Expired))
            } else {
                Ok(go)
            }
        });
    let outcome = if result.is_ok() {
        std::fs::write(
            std::env::var_os("LIMEOS_GATE_TEST_MARKER").unwrap(),
            b"unit-effect",
        )
        .unwrap();
        MountHelperOutcome::Exited { code: 0 }
    } else {
        MountHelperOutcome::NotStarted {}
    };
    if mode == "no_ack" {
        std::process::exit(0);
    }
    if mode == "malformed" {
        output.write_all(b"invalid").unwrap();
        output.flush().unwrap();
        std::process::exit(0);
    }
    if mode == "oversized_ack" {
        output.write_all(&vec![b'x'; MAX_FRAME + 1]).unwrap();
        output.flush().unwrap();
        std::process::exit(0);
    }
    let action = if mode == "wrong_action" { "c" } else { "a" }.repeat(64);
    let digest = if mode == "wrong_digest" { "c" } else { "b" }.repeat(64);
    acknowledge(&mut output, action, digest, outcome).unwrap();
    if mode == "extra" {
        output.write_all(b"extra").unwrap();
        output.flush().unwrap();
    }
    if mode == "ack_without_exit" {
        loop {
            thread::sleep(TICK);
        }
    }
    std::process::exit(if mode == "bad_exit" { 1 } else { 0 });
}

#[tokio::test]
async fn native_closed_partial_and_refused_go_prove_no_fixture_effect_after_reaping() {
    for mode in [
        "closed",
        "partial",
        "wrong_binding",
        "expired",
        "oversized_go",
    ] {
        let mut fixture = Fixture::start(mode).await;
        match mode {
            "partial" => fixture
                .process
                .input
                .as_mut()
                .unwrap()
                .write_all(&[0, 0, 1])
                .unwrap(),
            "wrong_binding" => fixture.go(|go| go.digest = "c".repeat(64)),
            "expired" => fixture.go(|go| go.expires_at = 0),
            "oversized_go" => fixture
                .process
                .input
                .as_mut()
                .unwrap()
                .write_all(&(MAX_FRAME as u32).to_be_bytes())
                .unwrap(),
            _ => {}
        }
        let proof = abort_process(&mut fixture.process).await.unwrap();
        assert!(matches!(
            proof.facts,
            MountEffectQuiescence::Reaped {
                outcome: MountHelperOutcome::NotStarted {},
                ..
            }
        ));
        assert!(!proof.facts.succeeded());
        assert!(!fixture.marker.exists());
        assert!(fixture.process.child.try_wait().unwrap().unwrap().success());
    }
}

#[tokio::test]
async fn native_abort_rejects_missing_malformed_unbound_or_unsuccessful_ack() {
    for mode in [
        "no_ack",
        "malformed",
        "wrong_action",
        "wrong_digest",
        "extra",
        "bad_exit",
        "oversized_ack",
    ] {
        let mut fixture = Fixture::start(mode).await;
        assert!(abort_process(&mut fixture.process).await.is_err());
        assert!(!fixture.marker.exists());
        // A protocol error itself does not establish termination. The private
        // fixture owns and explicitly reaps its failed child before continuing.
        let _ = fixture.process.child.kill();
        fixture.process.child.wait().unwrap();
    }
}

#[tokio::test]
async fn native_ack_does_not_prove_quiescence_while_its_owned_child_is_alive() {
    let mut fixture = Fixture::start("ack_without_exit").await;
    assert!(
        tokio::time::timeout(
            Duration::from_millis(50),
            abort_process(&mut fixture.process)
        )
        .await
        .is_err()
    );
    assert!(fixture.process.child.try_wait().unwrap().is_none());
    assert!(!fixture.marker.exists());
}

#[tokio::test]
async fn native_full_go_can_complete_but_cannot_be_accepted_as_gated_abort() {
    for only_not_started in [false, true] {
        let mut fixture = Fixture::start("full_go").await;
        fixture.go(|_| {});
        let proof = wait_process(&mut fixture.process, &|| false, only_not_started).await;
        if only_not_started {
            assert!(proof.is_err());
        } else {
            assert!(proof.unwrap().facts.succeeded());
        }
        assert_eq!(std::fs::read(&fixture.marker).unwrap(), b"unit-effect");
    }
}

#[tokio::test]
async fn native_final_authorization_and_original_deadline_precede_the_first_go_write() {
    use std::sync::atomic::AtomicBool;
    for mode in ["revoked", "expired", "authorized"] {
        let allowed = AtomicBool::new(true);
        assert!(allowed.load(Ordering::SeqCst)); // Earlier authorization.
        let mut fixture = Fixture::start("publication").await;
        let bytes = fixture.go_bytes(|_| {});
        if mode == "revoked" {
            allowed.store(false, Ordering::SeqCst);
        }
        let checked = AtomicUsize::new(0);
        let authorize = || {
            checked.fetch_add(1, Ordering::SeqCst);
            if allowed.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(Error(ErrorCode::Forbidden))
            }
        };
        let live = || {
            if mode == "expired" {
                Err(Error(ErrorCode::Expired))
            } else {
                Ok(())
            }
        };
        let released = release_process(&mut fixture.process, &bytes, &authorize, &live)
            .await
            .unwrap();
        assert_eq!(checked.load(Ordering::SeqCst), 1);
        match released {
            ProcessRelease::Sent => {
                assert_eq!(mode, "authorized");
                assert!(
                    wait_process(&mut fixture.process, &|| false, false)
                        .await
                        .unwrap()
                        .facts
                        .succeeded()
                );
                assert_eq!(std::fs::read(&fixture.marker).unwrap(), b"unit-effect");
            }
            ProcessRelease::Refused(proof) => {
                assert_ne!(mode, "authorized");
                assert!(matches!(
                    proof.facts,
                    MountEffectQuiescence::Reaped {
                        outcome: MountHelperOutcome::NotStarted {},
                        ..
                    }
                ));
                assert!(!fixture.marker.exists());
                assert!(fixture.process.child.try_wait().unwrap().unwrap().success());
            }
        }
    }
}
