use super::*;

fn target(index: usize, provider: Provider) -> Target {
    Target {
        container: ContainerSnapshot {
            resource: format!("container:{index:064x}"),
            image: format!("sha256:{index:064x}"),
            started_at: "2026-10-15T01:02:03.000000000Z".into(),
            running: true,
        },
        name: format!("service-{index}"),
        provider,
    }
}
fn stream(title: &str) -> Stream {
    Stream {
        title: title.into(),
        subtitle: "Example · S02E10".into(),
        user: "viewer".into(),
        client: "Web".into(),
        device: "Browser".into(),
        play_method: Some("Transcode".into()),
        paused: false,
        transcoding: true,
        transcode_reason: Some("VideoCodecNotSupported".into()),
        bitrate: Some(4_000_000.0),
        progress_percent: Some(25.0),
        position_seconds: Some(900.0),
        duration_seconds: Some(3600.0),
    }
}
fn read(targets: &[Target], samples: &[Sample<'_>]) -> ActivitySnapshot {
    snapshot(
        101,
        "engine-a",
        Some(ActivityInput {
            engine_id: "engine-a",
            sampled_at: 100,
            complete: true,
            targets,
            samples,
        }),
    )
    .unwrap()
}
#[test]
fn unavailable_never_looks_idle_but_complete_empty_inventory_has_zero() {
    let missing = snapshot(101, "engine-a", None).unwrap();
    assert_eq!(missing.source, SourceState::Unavailable);
    assert_eq!(missing.totals.streams, None);
    assert!(!missing.complete);
    let empty = read(&[], &[]);
    assert!(empty.complete);
    assert_eq!(empty.totals.streams, Some(0));
    assert_eq!(empty.totals.download_speed, Some(0.0));
}
#[test]
fn one_failing_provider_does_not_hide_successful_peer_or_invent_aggregate_zero() {
    let targets = [target(1, Provider::Jellyfin), target(2, Provider::Sonarr)];
    let data = ActivityData::Streams(vec![stream("Playing")]);
    let samples = [
        Sample {
            target: &targets[0],
            sampled_at: 100,
            result: Ok(&data),
        },
        Sample {
            target: &targets[1],
            sampled_at: 100,
            result: Err(ReadFailure::CredentialUnavailable),
        },
    ];
    let report = read(&targets, &samples);
    assert_eq!(report.streams.len(), 1);
    assert_eq!(
        report.services[1].state,
        ServiceState::CredentialUnavailable
    );
    assert!(!report.complete);
    assert_eq!(report.totals, Totals::default());
}
#[test]
fn partial_page_exposes_observed_problem_count_without_claiming_full_total() {
    let targets = [target(1, Provider::Radarr)];
    let data = ActivityData::Queue(Queue {
        total: 50,
        observed: 12,
        problems: 2,
        complete: false,
        items: vec![],
    });
    let report = read(
        &targets,
        &[Sample {
            target: &targets[0],
            sampled_at: 100,
            result: Ok(&data),
        }],
    );
    assert_eq!(report.services[0].state, ServiceState::Partial);
    assert_eq!(report.queues[0].data.problems, 2);
    assert_eq!(report.queues[0].data.observed, 12);
    assert_eq!(report.totals.queue_problems, None);
}
#[test]
fn unavailable_stale_future_and_wrong_engine_cohorts_withhold_all_rows() {
    let targets = [target(1, Provider::Jellyfin)];
    let data = ActivityData::Streams(vec![stream("Private title")]);
    let samples = [Sample {
        target: &targets[0],
        sampled_at: 100,
        result: Ok(&data),
    }];
    for (engine, at, state) in [
        ("engine-a", 92, SourceState::Stale),
        ("engine-a", 102, SourceState::Future),
        ("other", 100, SourceState::IdentityMismatch),
    ] {
        let report = snapshot(
            101,
            "engine-a",
            Some(ActivityInput {
                engine_id: engine,
                sampled_at: at,
                complete: true,
                targets: &targets,
                samples: &samples,
            }),
        )
        .unwrap();
        assert_eq!(report.source, state);
        assert_eq!(report.sampled_at, Some(at));
        assert!(report.streams.is_empty());
        assert!(report.services.is_empty());
        assert_eq!(report.totals, Totals::default());
    }
}
#[test]
fn incomplete_discovery_keeps_known_peer_but_withholds_totals() {
    let targets = [target(1, Provider::Emby)];
    let data = ActivityData::Streams(vec![stream("Playing")]);
    let samples = [Sample {
        target: &targets[0],
        sampled_at: 100,
        result: Ok(&data),
    }];
    let report = snapshot(
        101,
        "engine-a",
        Some(ActivityInput {
            engine_id: "engine-a",
            sampled_at: 100,
            complete: false,
            targets: &targets,
            samples: &samples,
        }),
    )
    .unwrap();
    assert_eq!(report.source, SourceState::Incomplete);
    assert_eq!(report.streams.len(), 1);
    assert_eq!(report.totals.streams, None);
    assert!(!report.complete);
}
#[test]
fn original_sample_time_expires_inclusively_without_restamping() {
    let targets = [target(1, Provider::Jellyfin)];
    let data = ActivityData::Streams(vec![]);
    for (at, state) in [
        (100, ServiceState::Ready),
        (101, ServiceState::Ready),
        (102, ServiceState::Future),
        (92, ServiceState::Stale),
    ] {
        let report = read(
            &targets,
            &[Sample {
                target: &targets[0],
                sampled_at: at,
                result: Ok(&data),
            }],
        );
        assert_eq!(report.services[0].state, state);
        assert_eq!(report.services[0].sampled_at, Some(at));
    }
    let report = snapshot(
        108,
        "engine-a",
        Some(ActivityInput {
            engine_id: "engine-a",
            sampled_at: 100,
            complete: true,
            targets: &targets,
            samples: &[Sample {
                target: &targets[0],
                sampled_at: 100,
                result: Ok(&data),
            }],
        }),
    )
    .unwrap();
    assert!(report.complete);
}
#[test]
fn restart_image_provider_and_name_drift_withhold_the_sample() {
    let targets = [target(1, Provider::Jellyfin)];
    let data = ActivityData::Streams(vec![stream("Previous instance")]);
    for change in 0..4 {
        let mut old = targets[0].clone();
        match change {
            0 => old.container.started_at = "2026-10-15T01:00:00.000000000Z".into(),
            1 => old.container.image = format!("sha256:{}", "f".repeat(64)),
            2 => old.provider = Provider::Emby,
            _ => old.name = "other".into(),
        }
        let report = read(
            &targets,
            &[Sample {
                target: &old,
                sampled_at: 100,
                result: Ok(&data),
            }],
        );
        assert_eq!(report.services[0].state, ServiceState::IdentityMismatch);
        assert!(report.streams.is_empty());
    }
}
#[test]
fn stopped_missing_timeout_and_invalid_services_are_visible() {
    let mut targets = [target(1, Provider::Jellyfin)];
    assert_eq!(
        read(&targets, &[]).services[0].state,
        ServiceState::Unavailable
    );
    for (error, state) in [
        (ReadFailure::Timeout, ServiceState::Timeout),
        (ReadFailure::InvalidResponse, ServiceState::InvalidResponse),
        (ReadFailure::Unsupported, ServiceState::Unsupported),
    ] {
        assert_eq!(
            read(
                &targets,
                &[Sample {
                    target: &targets[0],
                    sampled_at: 100,
                    result: Err(error)
                }]
            )
            .services[0]
                .state,
            state
        );
    }
    targets[0].container.running = false;
    assert_eq!(read(&targets, &[]).services[0].state, ServiceState::Stopped);
}
#[test]
fn duplicate_targets_samples_and_unrequested_samples_refuse() {
    let t = target(1, Provider::Jellyfin);
    let data = ActivityData::Streams(vec![]);
    assert!(
        snapshot(
            100,
            "engine-a",
            Some(ActivityInput {
                engine_id: "engine-a",
                sampled_at: 100,
                complete: true,
                targets: &[t.clone(), t.clone()],
                samples: &[]
            })
        )
        .is_err()
    );
    let samples = [
        Sample {
            target: &t,
            sampled_at: 100,
            result: Ok(&data),
        },
        Sample {
            target: &t,
            sampled_at: 100,
            result: Ok(&data),
        },
    ];
    assert!(
        snapshot(
            100,
            "engine-a",
            Some(ActivityInput {
                engine_id: "engine-a",
                sampled_at: 100,
                complete: true,
                targets: std::slice::from_ref(&t),
                samples: &samples
            })
        )
        .is_err()
    );
    assert!(
        snapshot(
            100,
            "engine-a",
            Some(ActivityInput {
                engine_id: "engine-a",
                sampled_at: 100,
                complete: true,
                targets: &[],
                samples: &samples[..1]
            })
        )
        .is_err()
    );
}
#[test]
fn display_limits_expose_omitted_counts_and_keep_full_totals() {
    let targets = [target(1, Provider::Jellyfin)];
    let data = ActivityData::Streams(
        (0..MAX_ITEMS)
            .map(|i| stream(&format!("Title {i:02}")))
            .collect(),
    );
    let report = read(
        &targets,
        &[Sample {
            target: &targets[0],
            sampled_at: 100,
            result: Ok(&data),
        }],
    );
    assert_eq!(report.streams.len(), MAX_DISPLAY);
    assert_eq!(report.omitted_streams, 12);
    assert_eq!(report.totals.streams, Some(24));
    assert_eq!(report.totals.transcodes, Some(24));
}
#[test]
fn selected_numeric_values_and_data_families_are_admitted() {
    let mut item = stream("bad metric");
    item.progress_percent = Some(f64::NAN);
    assert!(
        ActivityData::Streams(vec![item])
            .validate(Provider::Jellyfin)
            .is_err()
    );
    assert!(
        ActivityData::Streams(vec![])
            .validate(Provider::Transmission)
            .is_err()
    );
    let queue = ActivityData::Queue(Queue {
        total: 0,
        observed: 1,
        problems: 0,
        complete: false,
        items: vec![],
    });
    assert!(queue.validate(Provider::Sonarr).is_err());
}
#[test]
fn aggregate_selected_text_and_streamed_report_bounds_are_enforced() {
    let targets = [target(1, Provider::Jellyfin), target(2, Provider::Jellyfin)];
    let mut item = stream(&"x".repeat(200));
    item.user = "u".repeat(200);
    item.client = "c".repeat(200);
    item.device = "d".repeat(200);
    item.subtitle = "s".repeat(200);
    item.transcode_reason = Some("r".repeat(512));
    let data = ActivityData::Streams(vec![item; 24]);
    let samples = [
        Sample {
            target: &targets[0],
            sampled_at: 100,
            result: Ok(&data),
        },
        Sample {
            target: &targets[1],
            sampled_at: 100,
            result: Ok(&data),
        },
    ];
    assert!(
        snapshot(
            101,
            "engine-a",
            Some(ActivityInput {
                engine_id: "engine-a",
                sampled_at: 100,
                complete: true,
                targets: &targets,
                samples: &samples
            })
        )
        .is_err()
    );
    let mut report = read(&targets, &[]);
    report.services[0].target.name = "x".repeat(MAX_REPORT_BYTES);
    assert!(report.check_size().is_err());
}
#[test]
fn invalid_time_identity_and_target_count_are_refused() {
    assert!(snapshot(-1, "engine-a", None).is_err());
    assert!(snapshot(0, "", None).is_err());
    let targets: Vec<_> = (0..9).map(|i| target(i, Provider::Jellyfin)).collect();
    assert!(
        snapshot(
            100,
            "engine-a",
            Some(ActivityInput {
                engine_id: "engine-a",
                sampled_at: 100,
                complete: true,
                targets: &targets,
                samples: &[]
            })
        )
        .is_err()
    );
}
#[test]
fn order_is_deterministic() {
    let mut targets = [target(2, Provider::Jellyfin), target(1, Provider::Jellyfin)];
    let first = read(&targets, &[]);
    targets.reverse();
    assert_eq!(first, read(&targets, &[]));
}
#[test]
fn rust_wire_fixture_matches_frontend_contract() {
    let targets = [target(1, Provider::Jellyfin)];
    let data = ActivityData::Streams(vec![stream("Example title")]);
    let report = read(
        &targets,
        &[Sample {
            target: &targets[0],
            sampled_at: 100,
            result: Ok(&data),
        }],
    );
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/deployment/activity/wire.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(report).unwrap(), expected);
}
