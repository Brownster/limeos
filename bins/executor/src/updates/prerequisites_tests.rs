use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixtures {
    memory: Vec<MemoryCase>,
    journal: Vec<JournalCase>,
}

#[derive(Deserialize)]
struct MemoryCase {
    name: String,
    controllers: Option<String>,
    controllers_state: Option<String>,
    firmware: Option<String>,
    firmware_state: Option<String>,
    legacy: Option<String>,
    expected: String,
    reason: Option<String>,
    target: Option<String>,
    replacement: Option<String>,
}

#[derive(Deserialize)]
struct JournalCase {
    name: String,
    complete: Option<bool>,
    main: Option<String>,
    main_state: Option<String>,
    #[serde(default)]
    drops: Vec<DropCase>,
    expected: String,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct DropCase {
    name: String,
    body: Option<String>,
    state: Option<String>,
    origin: Option<String>,
}

fn fixtures() -> Fixtures {
    serde_json::from_str(include_str!(
        "../../../../tests/fixtures/deployment/prerequisites/cases.json"
    ))
    .unwrap()
}

fn fact<'a>(body: &'a Option<String>, state: &Option<String>) -> FileFact<'a> {
    match state.as_deref() {
        Some("unavailable") => FileFact::Unavailable,
        Some("masked") => FileFact::Masked,
        None => body
            .as_deref()
            .map_or(FileFact::Absent, |s| FileFact::Bytes(s.as_bytes())),
        _ => panic!("invalid fixture state"),
    }
}

fn result(assessment: &Assessment) -> (&str, Option<String>) {
    match assessment {
        Assessment::Satisfied => ("satisfied", None),
        Assessment::Unknown(reason) => ("unknown", Some(format!("{reason:?}"))),
        Assessment::Missing(Repair::Blocked(reason)) => ("blocked", Some(format!("{reason:?}"))),
        Assessment::Missing(Repair::RebootOrRuntimeInvestigation) => ("pending", None),
        Assessment::Missing(Repair::Proposed(_)) => ("proposed", None),
    }
}

fn proposal(assessment: Assessment) -> Proposal {
    let Assessment::Missing(Repair::Proposed(proposal)) = assessment else {
        panic!("expected proposal, got {assessment:?}");
    };
    proposal
}

fn memory<'a>(controllers: &'a [u8], boot: &'a [u8]) -> MemoryFacts<'a> {
    MemoryFacts {
        controllers: FileFact::Bytes(controllers),
        boot_platform: BootPlatform::RaspberryPiArm64,
        firmware_cmdline: FileFact::Bytes(boot),
        legacy_cmdline: FileFact::Absent,
    }
}

fn journal<'a>(main: &'a [u8], drops: &'a [SelectedDropIn<'a>]) -> JournalFacts<'a> {
    JournalFacts {
        complete: true,
        main: FileFact::Bytes(main),
        drop_ins: drops,
    }
}

fn drop_in<'a>(name: &'a str, bytes: &'a [u8]) -> SelectedDropIn<'a> {
    SelectedDropIn {
        basename: name,
        origin: DropInOrigin::Administrator,
        content: FileFact::Bytes(bytes),
    }
}

#[test]
fn frozen_memory_layouts_and_defect_fixtures() {
    for case in fixtures().memory {
        let facts = MemoryFacts {
            controllers: fact(&case.controllers, &case.controllers_state),
            boot_platform: BootPlatform::RaspberryPiArm64,
            firmware_cmdline: fact(&case.firmware, &case.firmware_state),
            legacy_cmdline: fact(&case.legacy, &None),
        };
        let assessed = assess_memory(facts);
        assert_eq!(
            result(&assessed),
            (case.expected.as_str(), case.reason),
            "{}",
            case.name
        );
        if let Assessment::Missing(Repair::Proposed(proposed)) = assessed {
            assert_eq!(
                Some(proposed.target().path()),
                case.target.as_deref(),
                "{}",
                case.name
            );
            assert_eq!(
                Some(proposed.replacement()),
                case.replacement.as_deref().map(str::as_bytes),
                "{}",
                case.name
            );
            assert!(proposed.matches_memory(facts));
            assert_eq!(
                proposed.verification_required(),
                VerificationRequired::RebootAndMemoryController
            );
            let after = match proposed.target() {
                Target::FirmwareCmdline => MemoryFacts {
                    firmware_cmdline: FileFact::Bytes(proposed.replacement()),
                    ..facts
                },
                Target::LegacyCmdline => MemoryFacts {
                    legacy_cmdline: FileFact::Bytes(proposed.replacement()),
                    ..facts
                },
                Target::JournalDropIn => unreachable!(),
            };
            assert_eq!(
                assess_memory(after),
                Assessment::Missing(Repair::RebootOrRuntimeInvestigation)
            );
            assert_eq!(
                assess_memory(MemoryFacts {
                    controllers: FileFact::Bytes(b"cpu memory\n"),
                    ..after
                }),
                Assessment::Satisfied
            );
        }
    }
}

#[test]
fn frozen_journal_layouts_and_defect_fixtures() {
    for case in fixtures().journal {
        let drops: Vec<_> = case
            .drops
            .iter()
            .map(|d| SelectedDropIn {
                basename: &d.name,
                origin: match d.origin.as_deref() {
                    Some("vendor") => DropInOrigin::Vendor,
                    Some("runtime") => DropInOrigin::Runtime,
                    Some("local_vendor") => DropInOrigin::LocalVendor,
                    None | Some("administrator") => DropInOrigin::Administrator,
                    _ => panic!("invalid fixture origin"),
                },
                content: fact(&d.body, &d.state),
            })
            .collect();
        let facts = JournalFacts {
            complete: case.complete.unwrap_or(true),
            main: fact(&case.main, &case.main_state),
            drop_ins: &drops,
        };
        let assessed = assess_journal(facts);
        assert_eq!(
            result(&assessed),
            (case.expected.as_str(), case.reason),
            "{}",
            case.name
        );
        if let Assessment::Missing(Repair::Proposed(proposed)) = assessed {
            assert_eq!(
                proposed.target().path(),
                "/etc/systemd/journald.conf.d/limeos.conf"
            );
            assert_eq!(proposed.before_image(), BeforeImage::Absent);
            assert_eq!(proposed.replacement(), JOURNAL_TEMPLATE);
            assert_eq!(
                proposed.verification_required(),
                VerificationRequired::JournalReloadAndEffectiveCap
            );
            assert!(proposed.matches_journal(facts));
            let mut after = drops.clone();
            after.push(drop_in(MANAGED_DROP_IN, proposed.replacement()));
            after.sort_by_key(|d| d.basename);
            assert_eq!(
                assess_journal(JournalFacts {
                    drop_ins: &after,
                    ..facts
                }),
                Assessment::Satisfied
            );
        }
    }
}

#[test]
fn firmware_path_precedes_legacy_and_before_image_covers_exact_bytes() {
    let original = b"root=FIRMWARE\n";
    let proposed = proposal(assess_memory(MemoryFacts {
        legacy_cmdline: FileFact::Bytes(b"root=LEGACY\n"),
        ..memory(b"cpu\n", original)
    }));
    assert_eq!(proposed.target(), Target::FirmwareCmdline);
    assert_eq!(
        proposed.before_image(),
        BeforeImage::Sha256(Sha256::digest(original).into())
    );
    assert!(proposed.replacement().starts_with(b"root=FIRMWARE "));
}

#[test]
fn memory_revalidation_binds_all_sources_and_runtime() {
    let facts = memory(b"cpu\n", b"root=x\n");
    let proposed = proposal(assess_memory(facts));
    assert!(proposed.matches_memory(facts));
    for changed in [
        MemoryFacts {
            boot_platform: BootPlatform::OtherOrUnknown,
            ..facts
        },
        MemoryFacts {
            controllers: FileFact::Bytes(b"cpu memory\n"),
            ..facts
        },
        MemoryFacts {
            firmware_cmdline: FileFact::Bytes(b"root=y\n"),
            ..facts
        },
        MemoryFacts {
            legacy_cmdline: FileFact::Bytes(b"root=x\n"),
            ..facts
        },
        MemoryFacts {
            firmware_cmdline: FileFact::Unavailable,
            ..facts
        },
    ] {
        assert!(!proposed.matches_memory(changed));
    }
    assert!(!proposed.matches_journal(journal(b"", &[])));
}

#[test]
fn journal_revalidation_binds_masks_origins_absence_and_full_bytes() {
    let drops = [drop_in("10-comments.conf", b"# original\n")];
    let facts = journal(b"", &drops);
    let proposed = proposal(assess_journal(facts));
    let changed_bytes = [drop_in("10-comments.conf", b"# changed\n")];
    let changed_mask = [SelectedDropIn {
        content: FileFact::Masked,
        ..drops[0]
    }];
    let changed_origin = [SelectedDropIn {
        origin: DropInOrigin::Runtime,
        ..drops[0]
    }];
    for changed in [
        JournalFacts {
            main: FileFact::Absent,
            ..facts
        },
        JournalFacts {
            complete: false,
            ..facts
        },
        JournalFacts {
            drop_ins: &changed_bytes,
            ..facts
        },
        JournalFacts {
            drop_ins: &changed_mask,
            ..facts
        },
        JournalFacts {
            drop_ins: &changed_origin,
            ..facts
        },
    ] {
        assert!(!proposed.matches_journal(changed));
    }
    assert!(!proposed.matches_memory(memory(b"cpu\n", b"root=x\n")));
}

#[test]
fn exact_managed_template_is_idempotent_but_later_reset_remains_blocked() {
    let managed = drop_in(MANAGED_DROP_IN, JOURNAL_TEMPLATE);
    assert_eq!(
        assess_journal(journal(b"", &[managed])),
        Assessment::Satisfied
    );
    assert_eq!(
        assess_journal(journal(
            b"",
            &[managed, drop_in("zz.conf", b"[Journal]\nSystemMaxUse=\n")]
        )),
        Assessment::Missing(Repair::Blocked(Reason::LaterJournalOverride))
    );
}

#[test]
fn duplicate_unsorted_path_like_and_absent_selected_sources_are_refused() {
    for drops in [
        vec![drop_in("same.conf", b""), drop_in("same.conf", b"")],
        vec![drop_in("z.conf", b""), drop_in("a.conf", b"")],
        vec![drop_in("../escape.conf", b"")],
        vec![drop_in("a.conf\n", b"")],
        vec![drop_in("config.txt", b"")],
        vec![SelectedDropIn {
            content: FileFact::Absent,
            ..drop_in("a.conf", b"")
        }],
    ] {
        assert_eq!(
            assess_journal(journal(b"", &drops)),
            Assessment::Unknown(Reason::InvalidSourceSet)
        );
    }
    assert_eq!(
        assess_journal(JournalFacts {
            main: FileFact::Masked,
            ..journal(b"", &[])
        }),
        Assessment::Unknown(Reason::InvalidSourceSet)
    );
}

#[test]
fn source_and_collection_limits_bound_work() {
    let oversized = vec![b' '; MAX_JOURNAL_SOURCE_BYTES + 1];
    assert_eq!(
        assess_journal(journal(&oversized, &[])),
        Assessment::Unknown(Reason::Limit)
    );
    let body = vec![b' '; MAX_JOURNAL_SOURCE_BYTES];
    let drops: Vec<_> = ["a.conf", "b.conf", "c.conf", "d.conf"]
        .map(|name| drop_in(name, &body))
        .into();
    assert_eq!(
        assess_journal(journal(b"# extra", &drops)),
        Assessment::Unknown(Reason::Limit)
    );
    let names: Vec<_> = (0..=MAX_DROP_INS).map(|n| format!("{n:03}.conf")).collect();
    let drops: Vec<_> = names.iter().map(|name| drop_in(name, b"")).collect();
    assert_eq!(
        assess_journal(journal(b"", &drops)),
        Assessment::Unknown(Reason::Limit)
    );
    let boot = vec![b'x'; MAX_BOOT_BYTES + 1];
    assert_eq!(
        assess_memory(memory(b"cpu", &boot)),
        Assessment::Unknown(Reason::Limit)
    );
    let controllers = vec![b'x'; MAX_CONTROLLER_BYTES + 1];
    assert_eq!(
        assess_memory(memory(&controllers, b"root=x")),
        Assessment::Unknown(Reason::Limit)
    );
    let boot = vec![b'x'; MAX_BOOT_BYTES];
    assert_eq!(
        assess_memory(memory(b"cpu", &boot)),
        Assessment::Missing(Repair::Blocked(Reason::Limit))
    );
}

#[test]
fn cap_units_whitespace_sections_and_unsupported_values() {
    for value in ["1", "200K", "200M", "1G", "1T", "1P", "1E", "000200M"] {
        let body = format!("[Other]\nSystemMaxUse=\n[Journal]\n  SystemMaxUse \t= {value} \t\n");
        assert_eq!(
            assess_journal(journal(body.as_bytes(), &[])),
            Assessment::Satisfied,
            "{value}"
        );
    }
    for body in [
        "[Journal]\nSystemMaxUse=garbage\n",
        "[Journal]\nSystemMaxUse=-1\n",
        "[Journal]\nSystemMaxUse=200MB\n",
        "[Journal]\nSystemMaxUse=200M # inline comment\n",
        "[Journal]\nSystemMaxUse=\"200M\"\n",
        "[Journal\nSystemMaxUse=200M\n",
        "[Journal]\nmalformed assignment\n",
    ] {
        assert_eq!(
            assess_journal(journal(body.as_bytes(), &[])),
            Assessment::Unknown(Reason::UnsupportedSyntax),
            "{body}"
        );
    }
}

#[test]
fn boot_repair_requires_supported_platform_and_fits_arm64_limit() {
    let unknown = MemoryFacts {
        boot_platform: BootPlatform::OtherOrUnknown,
        ..memory(b"cpu", b"root=x")
    };
    assert_eq!(
        assess_memory(unknown),
        Assessment::Missing(Repair::Blocked(Reason::UnsupportedBootLayout))
    );
    assert_eq!(
        assess_memory(MemoryFacts {
            controllers: FileFact::Bytes(b"cpu memory"),
            ..unknown
        }),
        Assessment::Satisfied
    );
    let flags_bytes = " cgroup_enable=memory cgroup_memory=1".len();
    let fits = vec![b'x'; MAX_PI_ARM64_CMDLINE_BYTES - flags_bytes];
    let proposed = proposal(assess_memory(memory(b"cpu", &fits)));
    assert_eq!(proposed.replacement().len(), MAX_PI_ARM64_CMDLINE_BYTES);
    let too_long = vec![b'x'; fits.len() + 1];
    assert_eq!(
        assess_memory(memory(b"cpu", &too_long)),
        Assessment::Missing(Repair::Blocked(Reason::Limit))
    );
}

#[test]
fn kernel_hyphen_aliases_cannot_bypass_conflict_detection() {
    for boot in [
        "root=x cgroup-disable=memory\n",
        "root=x cgroup-memory=0\n",
        "root=x cgroup-enable=memory cgroup_enable=memory\n",
    ] {
        assert_eq!(
            assess_memory(memory(b"cpu", boot.as_bytes())),
            Assessment::Missing(Repair::Blocked(Reason::ConflictingBootFlags)),
            "{boot}"
        );
    }
}

#[test]
fn invalid_bytes_cannot_generate_repairs() {
    for bytes in [&b"\xff"[..], &b"root=x\0"[..], &b"root=x\x1b"[..]] {
        assert_eq!(
            assess_memory(memory(b"cpu", bytes)),
            Assessment::Missing(Repair::Blocked(Reason::InvalidText))
        );
        assert_eq!(
            assess_journal(journal(bytes, &[])),
            Assessment::Unknown(Reason::InvalidText)
        );
    }
    assert_eq!(
        assess_memory(memory(b"memory memory", b"root=x")),
        Assessment::Unknown(Reason::UnsupportedSyntax)
    );
}

#[test]
fn debug_does_not_expose_boot_or_journal_payloads() {
    let proposed = proposal(assess_memory(memory(b"cpu", b"root=x secret=DO_NOT_LOG")));
    assert!(!format!("{proposed:?}").contains("DO_NOT_LOG"));
    assert!(!format!("{:?}", FileFact::Bytes(b"DO_NOT_LOG")).contains("DO_NOT_LOG"));
}
