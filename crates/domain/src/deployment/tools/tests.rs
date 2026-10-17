use super::*;

fn observation<T>(value: T) -> Observation<T> {
    Observation {
        sampled_at: 100,
        value,
    }
}
fn supplied<'a>(
    config: &'a Observation<ConfigState>,
    payload: &'a Observation<PayloadState>,
    service: &'a Observation<ServiceState>,
) -> Facts<'a> {
    Facts {
        config: Some(config),
        payload: Some(payload),
        service: Some(service),
    }
}
fn stored() -> Observation<ConfigState> {
    observation(ConfigState::Stored {
        config: Config::default(),
    })
}
fn preview(
    current: &Observation<ConfigState>,
    payload: &Observation<PayloadState>,
    service: &Observation<ServiceState>,
    now: i64,
    desired: Config,
) -> std::result::Result<ConfigPreview, Blocked> {
    preview_config(now, supplied(current, payload, service), desired).unwrap()
}
#[test]
fn frozen_defaults_are_explicit_and_validated() {
    let config = Config::default();
    assert_eq!(config.share_path(), "/srv/copyparty");
    assert_eq!(config.port(), 3923);
    assert_eq!(Config::new("/srv/copyparty", 3923).unwrap(), config);
}
#[test]
fn literal_case_and_internal_spaces_survive() {
    let config = Config::new("/srv/Media Library/TV", 65535).unwrap();
    assert_eq!(config.share_path(), "/srv/Media Library/TV");
    assert_eq!(config.port(), 65535);
    assert_eq!(Config::new("/srv/media library/TV", 1).unwrap().port(), 1);
}
#[test]
fn invalid_paths_are_refused_without_normalization() {
    for path in [
        "",
        "/",
        "relative",
        " /data",
        "/data ",
        "/data/",
        "//data",
        "/data//tv",
        "/data/./tv",
        "/data/../tv",
        "/data/ tv",
        "/data/\ntv",
        "/data\0",
    ] {
        assert_eq!(
            Config::new(path, 3923),
            Err(ConfigIssue::InvalidPath),
            "{path:?}"
        );
    }
    assert_eq!(
        Config::new(&format!("/{}", "x".repeat(256)), 1),
        Err(ConfigIssue::InvalidPath)
    );
    assert_eq!(
        Config::new(&format!("/{}", "x".repeat(MAX_PATH_BYTES)), 1),
        Err(ConfigIssue::Limit)
    );
}
#[test]
fn ports_do_not_wrap_or_admit_zero() {
    for port in [0, 65_536, u32::MAX] {
        assert_eq!(Config::new("/data", port), Err(ConfigIssue::InvalidPort));
    }
}
#[test]
fn debug_hides_configuration_paths() {
    let config = Config::new("/private/customer", 8080).unwrap();
    assert!(!format!("{config:?}").contains("customer"));
    assert!(!format!("{:?}", ConfigState::Stored { config }).contains("customer"));
}
#[test]
fn unavailable_observations_are_null_without_absence_claims() {
    let report = observe(105, Facts::default()).unwrap();
    assert_eq!(report.config.freshness, Freshness::Unavailable);
    assert_eq!(report.payload.value, None);
    assert_eq!(report.service.value, None);
    assert_eq!(report.config.sampled_at, None);
    assert_eq!(
        report.applied_configuration,
        AppliedConfiguration::NotEstablished
    );
}
#[test]
fn freshness_uses_original_inclusive_deadlines() {
    let config = stored();
    let payload = observation(PayloadState::Present);
    let service = observation(ServiceState::Active);
    let facts = supplied(&config, &payload, &service);
    assert_eq!(
        observe(190, facts).unwrap().config.freshness,
        Freshness::Fresh
    );
    let stale = observe(191, facts).unwrap();
    assert_eq!(stale.config.freshness, Freshness::Stale);
    assert_eq!(stale.config.sampled_at, Some(100));
    assert!(stale.config.value.is_none());
    assert!(observe(250, facts).unwrap().service.value.is_none());
    let future = observe(99, facts).unwrap();
    assert_eq!(future.service.freshness, Freshness::Future);
    assert_eq!(future.service.value, None);
}
#[test]
fn component_freshness_is_independent() {
    let config = stored();
    let payload = Observation {
        sampled_at: 101,
        value: PayloadState::Present,
    };
    let service = Observation {
        sampled_at: 192,
        value: ServiceState::Active,
    };
    let report = observe(191, supplied(&config, &payload, &service)).unwrap();
    assert_eq!(report.config.freshness, Freshness::Stale);
    assert_eq!(report.payload.value, Some(PayloadState::Present));
    assert_eq!(report.service.freshness, Freshness::Future);
}
#[test]
fn invalid_clocks_and_sample_times_are_errors() {
    assert!(observe(-1, Facts::default()).is_err());
    assert!(observe(MAX_TIME + 1, Facts::default()).is_err());
    for sampled_at in [-1, MAX_TIME + 1] {
        let config = Observation {
            sampled_at,
            value: ConfigState::Stored {
                config: Config::default(),
            },
        };
        assert!(
            observe(
                100,
                Facts {
                    config: Some(&config),
                    ..Facts::default()
                }
            )
            .is_err()
        );
    }
    assert!(observe(MAX_TIME, Facts::default()).is_ok());
}
#[test]
fn fresh_failures_stay_unknown_without_erasing_other_components() {
    let config = observation(ConfigState::Unknown {
        reason: ConfigIssue::Unavailable,
    });
    let payload = observation(PayloadState::Present);
    let service = observation(ServiceState::Unknown(ServiceIssue::Unavailable));
    let report = observe(105, supplied(&config, &payload, &service)).unwrap();
    assert_eq!(report.config.value, Some(config.value.clone()));
    assert_eq!(report.payload.value, Some(PayloadState::Present));
    assert_eq!(report.service.value, Some(service.value));
}
#[test]
fn active_and_equal_saved_fields_never_claim_applied_configuration() {
    let current = stored();
    let payload = observation(PayloadState::Present);
    let service = observation(ServiceState::Active);
    let plan = preview(&current, &payload, &service, 105, Config::default()).unwrap();
    assert!(!plan.save_required);
    assert!(plan.changed.is_empty());
    assert_eq!(
        plan.applied_configuration,
        AppliedConfiguration::NotEstablished
    );
    assert_eq!(plan.unresolved, GATES);
    let serialized = serde_json::to_string(&plan).unwrap();
    for absent in [
        "ExecStart",
        "extra_args",
        "download_url",
        "artifact",
        "receipt",
    ] {
        assert!(!serialized.contains(absent));
    }
}
#[test]
fn missing_config_uses_defaults_but_requires_saving() {
    let current = observation(ConfigState::Defaults {
        config: Config::default(),
    });
    let payload = observation(PayloadState::Present);
    let service = observation(ServiceState::NotFound);
    let plan = preview(&current, &payload, &service, 105, Config::default()).unwrap();
    assert!(plan.save_required);
    assert!(plan.changed.is_empty());
    assert_eq!(plan.unresolved.len(), 6);
}
#[test]
fn changed_fields_are_exact_and_deterministic() {
    let current = stored();
    let payload = observation(PayloadState::Present);
    let service = observation(ServiceState::Inactive);
    for (desired, fields) in [
        (
            Config::new("/srv/CopyParty", 3923).unwrap(),
            vec![Field::SharePath],
        ),
        (
            Config::new("/srv/copyparty", 8080).unwrap(),
            vec![Field::Port],
        ),
        (
            Config::new("/srv/CopyParty", 8080).unwrap(),
            vec![Field::SharePath, Field::Port],
        ),
    ] {
        let plan = preview(&current, &payload, &service, 105, desired.clone()).unwrap();
        assert_eq!(plan.changed, fields);
        assert_eq!(plan.desired, desired);
        assert!(plan.save_required);
    }
}
#[test]
fn missing_unreadable_and_unsupported_payloads_block_previews() {
    let current = stored();
    let service = observation(ServiceState::Active);
    for (value, blocked) in [
        (PayloadState::Absent, Blocked::PayloadMissing),
        (PayloadState::UnexpectedType, Blocked::PayloadUnsupported),
        (PayloadState::Unavailable, Blocked::PayloadObservation),
    ] {
        assert_eq!(
            preview(
                &current,
                &observation(value),
                &service,
                105,
                Config::default()
            ),
            Err(blocked)
        );
    }
}
#[test]
fn stale_or_unknown_configuration_cannot_generate_a_preview() {
    let current = stored();
    let payload = observation(PayloadState::Present);
    let service = observation(ServiceState::Active);
    assert_eq!(
        preview(&current, &payload, &service, 191, Config::default()),
        Err(Blocked::ConfigObservation)
    );
    for reason in [
        ConfigIssue::UnsupportedOptions,
        ConfigIssue::InvalidDocument,
    ] {
        let current = observation(ConfigState::Unknown { reason });
        assert_eq!(
            preview(&current, &payload, &service, 105, Config::default()),
            Err(Blocked::ConfigUnsupported)
        );
    }
}
#[test]
fn unreadable_and_incomplete_config_are_observation_blocks() {
    let payload = observation(PayloadState::Present);
    let service = observation(ServiceState::Active);
    for reason in [ConfigIssue::Unavailable, ConfigIssue::Incomplete] {
        let current = observation(ConfigState::Unknown { reason });
        assert_eq!(
            preview(&current, &payload, &service, 105, Config::default()),
            Err(Blocked::ConfigObservation)
        );
    }
}
#[test]
fn default_marker_cannot_hide_nondefault_values() {
    let config = observation(ConfigState::Defaults {
        config: Config::new("/other", 8080).unwrap(),
    });
    assert!(
        observe(
            105,
            Facts {
                config: Some(&config),
                ..Facts::default()
            }
        )
        .is_err()
    );
}
#[test]
fn service_transitions_masks_and_failures_remain_distinct() {
    let current = stored();
    let payload = observation(PayloadState::Present);
    for (value, expected) in [
        (ServiceState::Activating, Blocked::ServiceBusy),
        (ServiceState::Deactivating, Blocked::ServiceBusy),
        (ServiceState::Reloading, Blocked::ServiceBusy),
        (ServiceState::Masked, Blocked::ServiceUnsupported),
        (ServiceState::LoadError, Blocked::ServiceUnsupported),
        (
            ServiceState::Unknown(ServiceIssue::Incomplete),
            Blocked::ServiceObservation,
        ),
    ] {
        assert_eq!(
            preview(
                &current,
                &payload,
                &observation(value),
                105,
                Config::default()
            ),
            Err(expected)
        );
    }
    assert!(
        preview(
            &current,
            &payload,
            &observation(ServiceState::Failed),
            105,
            Config::default()
        )
        .is_ok()
    );
}
#[test]
fn stale_payload_and_service_are_not_refreshed_by_a_new_config_read() {
    let current = Observation {
        sampled_at: 191,
        value: stored().value,
    };
    let payload = observation(PayloadState::Present);
    let service = Observation {
        sampled_at: 191,
        value: ServiceState::Active,
    };
    assert_eq!(
        preview(&current, &payload, &service, 191, Config::default()),
        Err(Blocked::PayloadObservation)
    );
    let payload = Observation {
        sampled_at: 191,
        value: PayloadState::Present,
    };
    let service = observation(ServiceState::Active);
    assert_eq!(
        preview(&current, &payload, &service, 191, Config::default()),
        Err(Blocked::ServiceObservation)
    );
}
#[test]
fn maximal_literal_path_stays_within_report_budget() {
    let path = format!("/{}/{}", "\"".repeat(255), "x".repeat(255));
    assert_eq!(path.len(), MAX_PATH_BYTES);
    let config = observation(ConfigState::Stored {
        config: Config::new(&path, 65535).unwrap(),
    });
    let report = observe(
        105,
        Facts {
            config: Some(&config),
            ..Facts::default()
        },
    )
    .unwrap();
    report.check_size().unwrap();
    assert!(serde_json::to_vec(&report).unwrap().len() < MAX_REPORT_BYTES);
}
