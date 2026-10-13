use super::*;
use std::{process::Command, sync::Barrier, thread};

fn sample() -> Sample {
    Sample {
        open_fds: 16,
        rss_bytes: BASELINE_RSS,
        threads: 4,
        soft_nofile: Some(WORKER_FDS as u64),
    }
}
fn session() -> BudgetSession {
    BudgetSession::admit(sample()).unwrap()
}
fn ready() -> BudgetSession {
    let mut session = session();
    for owner in OWNERS {
        session
            .enter_with(owner.phase, || Ok(sample()))
            .unwrap()
            .commit_with(|| Ok(sample()))
            .unwrap();
    }
    session
}

#[test]
fn owner_model_fits_preserved_service_caps_with_explicit_margin() {
    let p = plan();
    assert_eq!(p.retained_fds, 178);
    assert_eq!(
        p.retained_fds + p.baseline_fds + p.transient_fds,
        p.worker_soft_nofile
    );
    assert_eq!(p.worker_soft_nofile, 226);
    assert!(p.worker_soft_nofile < p.service_nofile);
    assert_eq!(
        OWNERS
            .iter()
            .map(|o| o.modeled_payload_bytes)
            .sum::<usize>()
            + REPORT,
        21 * MIB + REPORT
    );
    assert!(
        OWNERS
            .iter()
            .map(|o| o.modeled_payload_bytes)
            .sum::<usize>()
            + REPORT
            <= p.modeled_payload_bytes
    );
    let unit = include_str!("../../../../packaging/systemd/limeos-storage-reader.service");
    for line in [
        "LimitNOFILE=256",
        "MemoryMax=64M",
        "MemorySwapMax=0",
        "TasksMax=16",
        "CapabilityBoundingSet=",
        "NoNewPrivileges=yes",
    ] {
        assert!(unit.lines().any(|s| s == line));
    }
}

#[test]
fn profile_selection_cannot_widen_any_parent_and_admits_destination_whole_counts() {
    let s = session();
    assert_eq!(s.configuration_profile().limits().max_retained_fds, 33);
    assert_eq!(s.configuration_profile().limits().max_file_bytes, 16 * 1024);
    assert_eq!(s.engine_profile(), EngineReadProfile::CombinedV1);
    assert_eq!(s.process_profile(), ProcessReadProfile::CombinedV1);
    assert_eq!(s.storage_profile(), StorageReadProfile::CombinedV1);
    assert_eq!(s.destination_limits().max_retained_fds, 32);
    assert_eq!(s.destination_limits().max_report_bytes, 32 * 1024);
}

#[test]
fn baseline_refuses_excess_or_missing_launch_ceiling_before_any_phase() {
    assert!(BudgetSession::admit(sample()).is_ok());
    for s in [
        Sample {
            open_fds: 17,
            ..sample()
        },
        Sample {
            rss_bytes: BASELINE_RSS + 1,
            ..sample()
        },
        Sample {
            threads: 5,
            ..sample()
        },
        Sample {
            threads: 0,
            ..sample()
        },
        Sample {
            soft_nofile: None,
            ..sample()
        },
        Sample {
            soft_nofile: Some(256),
            ..sample()
        },
        Sample {
            soft_nofile: Some(225),
            ..sample()
        },
    ] {
        assert!(matches!(
            BudgetSession::admit(s),
            Err(Refusal::ResourceLimit)
        ));
    }
}

#[test]
fn complete_sequence_keeps_previous_owner_reservations() {
    let mut s = session();
    for (i, owner) in OWNERS.iter().enumerate() {
        s.enter_with(owner.phase, || Ok(sample()))
            .unwrap()
            .commit_with(|| Ok(sample()))
            .unwrap();
        assert_eq!(
            s.retained_fds,
            OWNERS[..=i].iter().map(|o| o.retained_fds).sum::<usize>()
        );
        assert_eq!(s.next, i + 1);
    }
    assert_eq!(s.payload, 21 * MIB + REPORT);
    assert_eq!(s.retained_fds, 178);
    assert_eq!(
        s.report_with(&serde_json::json!({"observation":"fixture"}), || Ok(
            sample()
        ))
        .unwrap()
        .as_bytes(),
        b"{\"observation\":\"fixture\"}"
    );
}

#[test]
fn skipped_or_repeated_phases_poison_without_sampling() {
    let mut s = session();
    assert!(matches!(
        s.enter_with(Phase::Engine, || panic!("no host read on invalid phase")),
        Err(Refusal::InvalidPhase)
    ));
    assert!(matches!(
        s.enter_with(Phase::Configuration, || panic!(
            "no host read after poisoning"
        )),
        Err(Refusal::Poisoned)
    ));
    let mut s = session();
    s.enter_with(Phase::Configuration, || Ok(sample()))
        .unwrap()
        .commit_with(|| Ok(sample()))
        .unwrap();
    assert!(matches!(
        s.enter_with(Phase::Configuration, || panic!("no repeated phase")),
        Err(Refusal::InvalidPhase)
    ));
}

#[test]
fn dropped_inflight_phase_or_observation_failure_cannot_resume() {
    let mut s = session();
    drop(s.enter_with(Phase::Configuration, || Ok(sample())).unwrap());
    assert!(matches!(
        s.enter_with(Phase::Configuration, || Ok(sample())),
        Err(Refusal::Poisoned)
    ));
    let mut s = session();
    assert!(matches!(
        s.enter_with(Phase::Configuration, || Err(Refusal::Unavailable)),
        Err(Refusal::Unavailable)
    ));
    assert!(s.poisoned);
}

#[test]
fn exact_transient_fd_rss_and_thread_allowance_refuses_next_resource() {
    let mut s = session();
    let guard = s.enter_with(Phase::Configuration, || Ok(sample())).unwrap();
    let exact = Sample {
        open_fds: 16 + 33 + 32,
        rss_bytes: WORKER_RSS,
        ..sample()
    };
    guard.commit_with(|| Ok(exact)).unwrap();
    for excess in [
        Sample {
            open_fds: exact.open_fds + 1,
            ..exact
        },
        Sample {
            rss_bytes: WORKER_RSS + 1,
            ..exact
        },
        Sample {
            threads: 5,
            ..exact
        },
        Sample {
            soft_nofile: Some(256),
            ..exact
        },
    ] {
        let mut s = session();
        let guard = s.enter_with(Phase::Configuration, || Ok(sample())).unwrap();
        assert_eq!(
            guard.commit_with(|| Ok(excess)),
            Err(Refusal::ResourceLimit)
        );
        assert!(s.poisoned);
    }
}

#[test]
fn report_is_complete_bounded_and_escaped_bytes_consume_capacity() {
    assert_eq!(
        ready()
            .report_with(&"x".repeat(REPORT - 2), || Ok(sample()))
            .unwrap()
            .as_bytes()
            .len(),
        REPORT
    );
    assert!(matches!(
        ready().report_with(&"x".repeat(REPORT - 1), || Ok(sample())),
        Err(Refusal::ResourceLimit)
    ));
    assert!(matches!(
        ready().report_with(&"\n".repeat(REPORT / 2), || Ok(sample())),
        Err(Refusal::ResourceLimit)
    ));
    assert!(matches!(
        session().report_with(&"early", || panic!("report not ready")),
        Err(Refusal::InvalidPhase)
    ));
    assert!(matches!(
        ready().report_with(&"observation", || Err(Refusal::Unavailable)),
        Err(Refusal::Unavailable)
    ));
}

#[test]
fn status_parser_refuses_truncated_duplicate_missing_or_amplified_metrics() {
    assert_eq!(
        status("Name: private\nVmRSS: 123 kB\nThreads: 2\n"),
        Ok((123 * 1024, 2))
    );
    for text in [
        "VmRSS: 1 kB\nThreads: 2",
        "Threads: 2\n",
        "VmRSS: 1 bytes\nThreads: 2\n",
        "VmRSS: 1 kB extra\nThreads: 2\n",
        "VmRSS: 1 kB\nVmRSS: 1 kB\nThreads: 2\n",
        "VmRSS: 1 kB\nThreads: 2\nThreads: 2\n",
        "VmRSS: 18446744073709551615 kB\nThreads: 2\n",
    ] {
        assert_eq!(status(text), Err(Refusal::Unavailable));
    }
    assert_eq!(
        status(&format!("{}\n", "x".repeat(16 * 1024))),
        Err(Refusal::Unavailable)
    );
}

#[test]
fn single_flight_refuses_without_queue_and_drop_releases_capacity() {
    let m = SingleFlight::default();
    let guard = m.try_acquire().unwrap();
    assert!(matches!(m.try_acquire(), Err(Refusal::Busy)));
    drop(guard);
    assert!(m.try_acquire().is_ok());
}

#[test]
fn simultaneous_admission_has_exactly_one_holder() {
    let manager = Arc::new(SingleFlight::default());
    let before = Arc::new(Barrier::new(8));
    let after = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let manager = manager.clone();
            let before = before.clone();
            let after = after.clone();
            thread::spawn(move || {
                before.wait();
                let guard = manager.try_acquire();
                after.wait();
                guard.is_ok()
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|t| usize::from(t.join().unwrap()))
            .sum::<usize>(),
        1
    );
    assert!(manager.try_acquire().is_ok());
}

#[test]
fn real_worker_resource_observation_in_owned_child_with_lowered_limit() {
    const FLAG: &str = "LIMEOS_COMBINED_BUDGET_SELF_PROBE";
    if std::env::var_os(FLAG).is_none() {
        let parent_limit = getrlimit(Resource::Nofile);
        let output=Command::new(std::env::current_exe().unwrap()).args(["--exact","read_budget::tests::real_worker_resource_observation_in_owned_child_with_lowered_limit","--nocapture","--test-threads=1"]).env(FLAG,"1").output().unwrap();
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
    assert!(old.current.is_none_or(|n| n >= 226));
    rustix::process::setrlimit(
        Resource::Nofile,
        rustix::process::Rlimit {
            current: Some(226),
            maximum: old.maximum,
        },
    )
    .unwrap();
    let mut s = BudgetSession::admit_worker().unwrap();
    println!(
        "worker_budget_observation soft_nofile=226 baseline_fds={} baseline_rss_bytes={} baseline_threads={} real_owner_collection=false root_authorization=false",
        s.baseline.open_fds, s.baseline.rss_bytes, s.baseline.threads
    );
    for owner in OWNERS {
        s.enter(owner.phase).unwrap().commit().unwrap();
    }
    let report = s
        .report(&serde_json::json!({"fixture":"no owner collection"}))
        .unwrap();
    assert!(report.as_bytes().len() < REPORT);
    assert_eq!(getrlimit(Resource::Nofile).maximum, old.maximum);
}
