use super::*;
use std::{path::Path, process::Command, rc::Rc};

fn window() -> CollectionWindow {
    CollectionWindow {
        started: Instant::now(),
        started_wall: 100,
        wall_high_water: Cell::new(100),
        failed: Cell::new(None),
    }
}
fn paths() -> ConfigurationPaths<'static> {
    ConfigurationPaths {
        contract: Path::new("/not-opened/limeos/contract.json"),
        pools: Path::new("/not-opened/limeos/pools.json"),
        protection: Path::new("/not-opened/limeos/protection.json"),
        shares: Path::new("/not-opened/limeos/shares.json"),
    }
}
#[derive(Debug)]
struct Dropped(Rc<Cell<bool>>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn monotonic_window_accepts_before_and_refuses_at_exact_original_deadline() {
    let w = window();
    assert_eq!(
        w.check_at(w.started + WINDOW - Duration::from_nanos(1), 104, false),
        Ok(())
    );
    assert_eq!(
        w.check_at(w.started + WINDOW, 104, false),
        Err(Failure::Expired)
    );
    // A retry cannot renew the original window, even with an earlier sample.
    assert_eq!(w.check_at(w.started, 100, false), Err(Failure::Expired));
}

#[test]
fn wall_expiry_refuses_even_when_monotonic_time_is_inside_window() {
    let w = window();
    assert_eq!(w.check_at(w.started, 104, false), Ok(()));
    assert_eq!(w.check_at(w.started, 105, false), Err(Failure::Expired));
}

#[test]
fn backward_wall_time_refuses_above_the_original_issue_time() {
    let w = window();
    assert_eq!(w.check_at(w.started, 103, false), Ok(()));
    assert_eq!(w.check_at(w.started, 102, false), Err(Failure::Clock));
    assert_eq!(w.check_at(w.started, 104, false), Err(Failure::Clock));
}

#[test]
fn negative_wall_and_backward_monotonic_samples_refuse() {
    let w = window();
    assert_eq!(w.check_at(w.started, -1, false), Err(Failure::Clock));
    let w = window();
    assert_eq!(
        w.check_at(w.started - Duration::from_nanos(1), 100, false),
        Err(Failure::Clock)
    );
}

#[test]
fn cancellation_permanently_refuses_the_window() {
    let w = window();
    assert_eq!(w.check_at(w.started, 100, true), Err(Failure::Cancelled));
    assert_eq!(w.check_at(w.started, 100, false), Err(Failure::Cancelled));
    assert_eq!(
        w.check(&|| panic!("a poisoned window must not poll callbacks again")),
        Err(Failure::Cancelled)
    );
}

#[tokio::test]
async fn cancellation_before_work_does_not_poll_the_operation() {
    let w = CollectionWindow::new().unwrap();
    let polled = Cell::new(false);
    let result = w
        .checked(Stage::Collect(Phase::Engine), &|| true, async {
            polled.set(true);
            Ok(())
        })
        .await;
    assert_eq!(
        result,
        Err(Refusal {
            stage: Stage::Collect(Phase::Engine),
            failure: Failure::Cancelled,
        })
    );
    assert!(!polled.get());
}

#[tokio::test]
async fn cancellation_after_work_discards_a_completed_value() {
    let w = CollectionWindow::new().unwrap();
    let cancelled = Cell::new(false);
    let dropped = Rc::new(Cell::new(false));
    let result = w
        .checked(
            Stage::Collect(Phase::Physical),
            &|| cancelled.get(),
            async {
                cancelled.set(true);
                Ok(Dropped(dropped.clone()))
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(Refusal {
            stage: Stage::Collect(Phase::Physical),
            failure: Failure::Cancelled,
        })
    ));
    assert!(dropped.get());
}

#[tokio::test]
async fn final_report_gate_discards_bytes_on_late_cancellation() {
    let w = CollectionWindow::new().unwrap();
    let cancelled = Cell::new(false);
    // Tests the final publication gate only; these are not fabricated owners.
    let result = w
        .checked(Stage::Report, &|| cancelled.get(), async {
            cancelled.set(true);
            Ok(b"complete observation".to_vec())
        })
        .await;
    assert_eq!(
        result,
        Err(Refusal {
            stage: Stage::Report,
            failure: Failure::Cancelled,
        })
    );
}

#[tokio::test]
async fn adapter_failure_keeps_reason_and_stage_and_prevents_further_work() {
    for failure in [
        Failure::Budget(read_budget::Refusal::ResourceLimit),
        Failure::Storage(limeos_executor_storage::Failure::WrongNamespace),
        Failure::Engine(ErrorCode::Conflict),
        Failure::InvalidPlan,
    ] {
        let w = CollectionWindow::new().unwrap();
        assert_eq!(
            w.checked(Stage::EngineBeforeValidation, &|| false, async {
                Err::<(), _>(failure)
            })
            .await,
            Err(Refusal {
                stage: Stage::EngineBeforeValidation,
                failure,
            })
        );
        assert_eq!(
            w.checked(Stage::Report, &|| false, async {
                panic!("failed window must not poll later work");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await,
            Err(Refusal {
                stage: Stage::Report,
                failure,
            })
        );
    }
}

#[tokio::test]
async fn successful_work_passes_both_checks_without_renewing_start_time() {
    let w = CollectionWindow::new().unwrap();
    let started = w.started;
    let wall = w.started_wall;
    assert_eq!(w.wall(&|| false).unwrap(), w.wall_high_water.get());
    assert_eq!(
        w.checked(Stage::Report, &|| false, async { Ok(7) }).await,
        Ok(7)
    );
    assert_eq!(w.started, started);
    assert_eq!(w.started_wall, wall);
    assert_eq!(
        w.check_at(started, wall + WINDOW.as_secs() as i64 + 1, false),
        Err(Failure::Expired)
    );
    assert_eq!(
        w.wall(&|| panic!("the planner cannot renew a failed window")),
        Err(Failure::Expired)
    );
}

#[tokio::test]
async fn blocked_synchronous_work_is_refused_after_return_not_independently_killed() {
    let w = CollectionWindow::new().unwrap();
    let dropped = Rc::new(Cell::new(false));
    let result = w
        .checked(Stage::Collect(Phase::Physical), &|| false, async {
            // An independent supervisor remains necessary: this blocks the runtime.
            std::thread::sleep(WINDOW + Duration::from_millis(20));
            Ok(Dropped(dropped.clone()))
        })
        .await;
    assert!(matches!(
        result,
        Err(Refusal {
            failure: Failure::Expired,
            ..
        })
    ));
    assert!(dropped.get());
    assert!(w.started.elapsed() >= WINDOW);
}

#[tokio::test]
async fn public_entry_cancellation_precedes_worker_admission_and_host_access() {
    assert!(matches!(
        collect(paths(), &|| true).await,
        Err(Refusal {
            stage: Stage::Admission,
            failure: Failure::Cancelled,
        })
    ));
}

#[test]
fn real_public_nonroot_refusal_in_owned_worker_with_closed_limit() {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    const FLAG: &str = "LIMEOS_COMBINED_COLLECTION_NONROOT_PROBE";
    // This local test cannot safely exercise positive host collection as root.
    assert!(
        !rustix::process::geteuid().is_root(),
        "run this local suite without root"
    );
    if std::env::var_os(FLAG).is_none() {
        let parent_limit = getrlimit(Resource::Nofile);
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "read_collection::tests::real_public_nonroot_refusal_in_owned_worker_with_closed_limit",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FLAG, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
        assert_eq!(getrlimit(Resource::Nofile), parent_limit);
        return;
    }
    let old = getrlimit(Resource::Nofile);
    assert!(old.current.is_none_or(|limit| limit >= 226));
    setrlimit(
        Resource::Nofile,
        Rlimit {
            current: Some(226),
            maximum: old.maximum,
        },
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(matches!(
        runtime.block_on(collect(paths(), &|| false)),
        Err(Refusal {
            stage: Stage::Collect(Phase::Configuration),
            failure: Failure::Storage(limeos_executor_storage::Failure::Unavailable),
        })
    ));
    println!(
        "RW040_NONROOT_COLLECTION_REFUSAL soft_nofile=226 uid={} stage=configuration",
        rustix::process::geteuid().as_raw()
    );
}
