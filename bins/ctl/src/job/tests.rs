use super::*;
use limeos_contracts::{ResolutionTargetView, ResolveStatus};
use limeos_domain::{
    ContainerAction, ContainerSnapshot, JobResolution, JobState, ResolutionEvidence,
    ResolutionMember, ResolutionTarget, ResolveKind,
};
use std::sync::{Arc, Mutex};

const JOB: &str = "e";
fn job() -> String {
    JOB.repeat(64)
}
fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}
fn expected() -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", "b".repeat(64)),
        image: format!("sha256:{}", "c".repeat(64)),
        started_at: "2026-10-05T07:00:00Z".into(),
        running: true,
    }
}
fn view() -> ResolutionView {
    let measured = ResolutionEvidence::observe(
        ContainerAction::Restart,
        &[&expected()],
        &[Some(ContainerSnapshot {
            started_at: "2026-10-05T07:01:00Z".into(),
            ..expected()
        })],
    )
    .unwrap();
    let evidence_digest = measured.digest().unwrap();
    ResolutionView {
        job: job(),
        state: JobState::NeedsIntervention,
        kind: ResolveKind::Container,
        operation: ContainerAction::Restart,
        targets: vec![ResolutionTargetView {
            expected: expected(),
            measured: measured.targets[0].clone(),
        }],
        evidence_digest: Some(evidence_digest),
        allowed: vec![
            DeclaredOutcome::VerifiedDone,
            DeclaredOutcome::AcceptedDivergence,
        ],
        members: Some(vec![ResolutionMember {
            container: "b".repeat(64),
            state: limeos_domain::ExecutionState::OutcomeUnknown,
            error: Some(limeos_domain::ErrorCode::Unavailable),
        }]),
        pending: None,
        resolution: None,
    }
}

#[test]
fn parse_accepts_the_documented_forms_in_any_flag_order() {
    let j = job();
    assert_eq!(
        parse(&args(&["show-resolution", &j, "--admin", "alice"])).unwrap(),
        Command::Show {
            admin: "alice".into(),
            job: j.clone()
        }
    );
    assert_eq!(
        parse(&args(&[
            "resolve",
            &j,
            "--outcome",
            "accepted-divergence",
            "--admin",
            "alice",
            "--reason",
            "looked by hand"
        ]))
        .unwrap(),
        Command::Resolve {
            admin: "alice".into(),
            job: j.clone(),
            declared: DeclaredOutcome::AcceptedDivergence,
            reason: Some("looked by hand".into()),
            digest: None,
        }
    );
    for (word, declared) in [
        ("verified-done", DeclaredOutcome::VerifiedDone),
        ("not-done", DeclaredOutcome::NotDone),
    ] {
        let parsed = parse(&args(&[
            "resolve",
            &j,
            "--admin",
            "a",
            "--outcome",
            word,
            "--evidence-digest",
            "abc",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            Command::Resolve {
                admin: "a".into(),
                job: j.clone(),
                declared,
                reason: None,
                digest: Some("abc".into())
            }
        );
    }
}

#[test]
fn parse_refuses_everything_else() {
    let j = job();
    let bad: Vec<Vec<&str>> = vec![
        vec![],
        vec!["resolve"],
        vec!["resolve", "--admin", "a"],
        vec!["resolve", "short", "--admin", "a", "--outcome", "not-done"],
        vec!["resolve", &j, "--outcome", "not-done"],
        vec!["resolve", &j, "--admin", "a"],
        vec!["resolve", &j, "--admin", "a", "--outcome", "done"],
        vec!["resolve", &j, "--admin", "a", "--outcome", "not_done"],
        vec!["resolve", &j, "--admin", "a", "--outcome"],
        vec![
            "resolve",
            &j,
            "--admin",
            "a",
            "--admin",
            "b",
            "--outcome",
            "not-done",
        ],
        vec![
            "resolve",
            &j,
            "--admin",
            "a",
            "--outcome",
            "not-done",
            "--force",
            "1",
        ],
        vec!["show-resolution", &j],
        vec![
            "show-resolution",
            &j,
            "--admin",
            "a",
            "--outcome",
            "not-done",
        ],
        vec!["restart", &j, "--admin", "a"],
        // A flag where a value belongs is refused, never taken as the value.
        vec![
            "resolve",
            &j,
            "--admin",
            "a",
            "--outcome",
            "not-done",
            "--reason",
            "--version",
        ],
        vec!["resolve", &j, "--admin", "--help", "--outcome", "not-done"],
        vec!["resolve", &j, "--admin", "a", "--outcome", "-h"],
        vec![
            "resolve",
            &j,
            "--admin",
            "a",
            "--outcome",
            "not-done",
            "--help",
        ],
    ];
    for case in bad {
        assert!(parse(&args(&case)).is_err(), "{case:?}");
    }
}

#[test]
fn the_view_prints_plan_against_measurement_and_the_digest_to_bind() {
    let text = render_view(&view());
    for needle in [
        &format!("job: {}", job()),
        "state: needs_intervention",
        "kind: container restart",
        &format!("target: {}", "b".repeat(64)),
        "planned:  running yes started 2026-10-05T07:00:00Z",
        "measured: running yes started 2026-10-05T07:01:00Z",
        "unchanged from plan: no   intended state reached: yes",
        "outcomes available: verified-done, accepted-divergence",
        "executor outcome:",
        "outcome_unknown (unavailable)",
    ] {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
    assert!(text.contains(&format!(
        "evidence digest: {}",
        view().evidence_digest.unwrap()
    )));
    // A vanished container reads as such, not as empty fields.
    let mut gone = view();
    gone.targets[0].measured = ResolutionTarget {
        container: "b".repeat(64),
        running: false,
        started_at: String::new(),
        image: String::new(),
        matches_expected: false,
        intended_state_reached: false,
    };
    assert!(render_view(&gone).contains("container no longer exists"));
    let mut done = view();
    done.targets.clear();
    done.evidence_digest = None;
    done.resolution = Some(JobResolution {
        job: job(),
        resolved_by: "u".repeat(32),
        declared: DeclaredOutcome::NotDone,
        reason: "nothing happened".into(),
        evidence: view_evidence(),
        evidence_digest: "d".repeat(64),
        requested_at: 1,
        resolved_at: 2,
    });
    let text = render_view(&done);
    assert!(text.contains("resolved: not-done by"), "{text}");
    assert!(text.contains("reason: nothing happened"));
    assert!(!text.contains("evidence digest"));
}
fn view_evidence() -> ResolutionEvidence {
    ResolutionEvidence {
        targets: vec![view().targets[0].measured.clone()],
    }
}

#[test]
fn only_a_resolved_response_settles() {
    let response = |status, blocking: Vec<String>| ResolveJobResponse {
        status,
        message: "message".into(),
        resolution: None,
        view: (status == ResolveStatus::UnsupportedDeclaration).then(view),
        blocking,
    };
    let (text, ok) = render_response(&response(ResolveStatus::Resolved, vec![]));
    assert!(ok && text == "message\n");
    for status in [
        ResolveStatus::Pending,
        ResolveStatus::StaleEvidence,
        ResolveStatus::UnsupportedDeclaration,
    ] {
        assert!(!render_response(&response(status, vec![])).1);
    }
    let (text, _) = render_response(&response(
        ResolveStatus::UnsupportedDeclaration,
        vec!["b".repeat(64)],
    ));
    assert!(text.contains(&format!("blocking: {}", "b".repeat(64))));
    assert!(text.contains("unchanged from plan"));
}

#[test]
fn the_reason_comes_from_the_flag_or_stdin_and_is_never_empty() {
    let mut none: &[u8] = b"";
    assert_eq!(
        reason_from(Some("  from flag ".into()), &mut none).unwrap(),
        "from flag"
    );
    let mut piped: &[u8] = b"from stdin\n";
    assert_eq!(reason_from(None, &mut piped).unwrap(), "from stdin");
    for empty in [b"" as &[u8], b" \n\n"] {
        let mut input = empty;
        assert!(reason_from(None, &mut input).is_err());
    }
    assert!(reason_from(Some("  ".into()), &mut none).is_err());
    let long = vec![b'x'; 5000];
    let mut input: &[u8] = &long;
    assert!(reason_from(None, &mut input).is_err());
}

/// A core that answers each connection in turn and records the requests.
fn fake_core(
    dir: &tempfile::TempDir,
    answers: Vec<CoreResponse>,
) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let path = dir.path().join("core.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        let listener = tokio::net::UnixListener::from_std(listener).unwrap();
        for answer in answers {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: serde_json::Value =
                limeos_contracts::read_frame(&mut stream).await.unwrap();
            log.lock().unwrap().push(request);
            limeos_contracts::write_frame(&mut stream, &answer)
                .await
                .unwrap();
        }
    });
    (path.to_str().unwrap().to_owned(), seen)
}
fn resolve(digest: Option<String>) -> Command {
    Command::Resolve {
        admin: "alice".into(),
        job: job(),
        declared: DeclaredOutcome::AcceptedDivergence,
        reason: None,
        digest,
    }
}
/// A terminal on which the operator types `answer`.
fn typed(answer: &'static str) -> impl FnMut() -> std::io::Result<String> {
    move || Ok(format!("{answer}\n"))
}
fn done() -> ResolveJobResponse {
    ResolveJobResponse {
        status: ResolveStatus::Resolved,
        message: "Resolved.".into(),
        resolution: None,
        view: None,
        blocking: vec![],
    }
}

#[tokio::test]
async fn resolve_shows_evidence_then_submits_exactly_its_digest_with_the_stdin_reason() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, seen) = fake_core(
        &dir,
        vec![
            CoreResponse::Resolution(view()),
            CoreResponse::Resolved(done()),
        ],
    );
    let mut stdin: &[u8] = b"Verified in the engine by hand.\n";
    execute(
        &socket,
        resolve(None),
        &mut stdin,
        Some(&mut typed("accepted-divergence")),
    )
    .await
    .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0]["request"], "read_resolution");
    assert_eq!(seen[0]["admin"], "alice");
    assert_eq!(seen[1]["request"], "resolve_job");
    assert_eq!(seen[1]["job"], job());
    assert_eq!(seen[1]["declared"], "accepted_divergence");
    assert_eq!(seen[1]["reason"], "Verified in the engine by hand.");
    assert_eq!(seen[1]["evidence_digest"], view().evidence_digest.unwrap());
    // No password, token or session ever crosses this socket.
    assert!(seen[1].get("password").is_none() && seen[1].get("token").is_none());
}

#[tokio::test]
async fn a_digest_from_an_earlier_look_must_still_match_before_anything_is_decided() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, seen) = fake_core(&dir, vec![CoreResponse::Resolution(view())]);
    let mut stdin: &[u8] = b"reason text";
    let error = execute(&socket, resolve(Some("0".repeat(64))), &mut stdin, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("state changed"), "{error}");
    assert_eq!(seen.lock().unwrap().len(), 1);
    // The matching digest goes through.
    let dir = tempfile::tempdir().unwrap();
    let (socket, seen) = fake_core(
        &dir,
        vec![
            CoreResponse::Resolution(view()),
            CoreResponse::Resolved(done()),
        ],
    );
    let mut stdin: &[u8] = b"reason text";
    execute(&socket, resolve(view().evidence_digest), &mut stdin, None)
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn no_reason_or_an_unsettled_answer_is_a_failure_with_nothing_sent_first() {
    let dir = tempfile::tempdir().unwrap();
    // No reason: refused before the socket is even used.
    let (socket, seen) = fake_core(&dir, vec![]);
    let mut stdin: &[u8] = b"";
    assert!(
        execute(
            &socket,
            resolve(None),
            &mut stdin,
            Some(&mut typed("accepted-divergence"))
        )
        .await
        .is_err()
    );
    assert!(seen.lock().unwrap().is_empty());
    // Held pending: non-zero exit, message shown.
    let dir = tempfile::tempdir().unwrap();
    let mut pending = done();
    pending.status = ResolveStatus::Pending;
    pending.message = "The request is recorded and the job stays held.".into();
    let (socket, _) = fake_core(
        &dir,
        vec![
            CoreResponse::Resolution(view()),
            CoreResponse::Resolved(pending),
        ],
    );
    let mut stdin: &[u8] = b"reason text";
    let error = execute(
        &socket,
        resolve(None),
        &mut stdin,
        Some(&mut typed("accepted-divergence")),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("stays held"));
    // A job that is not awaiting resolution has no digest to decide on.
    let dir = tempfile::tempdir().unwrap();
    let mut settled = view();
    settled.evidence_digest = None;
    settled.targets.clear();
    let (socket, seen) = fake_core(&dir, vec![CoreResponse::Resolution(settled)]);
    let mut stdin: &[u8] = b"reason text";
    assert!(
        execute(
            &socket,
            resolve(None),
            &mut stdin,
            Some(&mut typed("accepted-divergence"))
        )
        .await
        .is_err()
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn show_only_reads() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, seen) = fake_core(&dir, vec![CoreResponse::Resolution(view())]);
    let mut stdin: &[u8] = b"";
    execute(
        &socket,
        Command::Show {
            admin: "alice".into(),
            job: job(),
        },
        &mut stdin,
        None,
    )
    .await
    .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["request"], "read_resolution");
}

#[tokio::test]
async fn without_a_digest_or_a_terminal_nothing_is_read_or_decided() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, seen) = fake_core(&dir, vec![]);
    let mut stdin: &[u8] = b"a reason that is never read";
    let error = execute(&socket, resolve(None), &mut stdin, None)
        .await
        .unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("show-resolution") && text.contains("--evidence-digest"),
        "{text}"
    );
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(stdin, b"a reason that is never read");
}

#[tokio::test]
async fn on_a_terminal_anything_but_the_typed_outcome_cancels_after_the_evidence_is_shown() {
    for answer in [
        "yes",
        "y",
        "",
        "verified-done",
        "accepted-divergence please",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (socket, seen) = fake_core(&dir, vec![CoreResponse::Resolution(view())]);
        let mut stdin: &[u8] = b"reason text";
        let error = execute(&socket, resolve(None), &mut stdin, Some(&mut typed(answer)))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("not confirmed"),
            "{answer}: {error}"
        );
        // The evidence was read and shown; no decision was sent.
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "{answer}");
        assert_eq!(seen[0]["request"], "read_resolution");
    }
}

#[tokio::test]
async fn a_confirmed_decision_binds_to_the_digest_that_was_shown_and_asks_once() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, seen) = fake_core(
        &dir,
        vec![
            CoreResponse::Resolution(view()),
            CoreResponse::Resolved(done()),
        ],
    );
    let mut asked = 0;
    let mut confirm = || {
        asked += 1;
        Ok("  accepted-divergence \n".to_owned())
    };
    let mut stdin: &[u8] = b"reason text";
    execute(&socket, resolve(None), &mut stdin, Some(&mut confirm))
        .await
        .unwrap();
    assert_eq!(asked, 1);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1]["evidence_digest"], view().evidence_digest.unwrap());
    }
    // A digest from an earlier look is its own confirmation: no question asked.
    let dir = tempfile::tempdir().unwrap();
    let (socket, _) = fake_core(
        &dir,
        vec![
            CoreResponse::Resolution(view()),
            CoreResponse::Resolved(done()),
        ],
    );
    let mut never = || -> std::io::Result<String> { panic!("asked despite a digest") };
    let mut stdin: &[u8] = b"reason text";
    execute(
        &socket,
        resolve(view().evidence_digest),
        &mut stdin,
        Some(&mut never),
    )
    .await
    .unwrap();
}

mod admin_commands {
    use super::super::admin::{self, Cancel, List};
    use super::*;
    use limeos_domain::{
        CancelOutcome, CancelRefusal, JobFamily, OperatorCancel, OperatorJob, OperatorJobFilter,
        OperatorJobPage,
    };

    const UNSETTLED: [JobState; 7] = [
        JobState::Planned,
        JobState::WaitingApproval,
        JobState::Queued,
        JobState::Running,
        JobState::Verifying,
        JobState::OutcomeUnknown,
        JobState::NeedsIntervention,
    ];

    fn filter(states: &[JobState]) -> OperatorJobFilter {
        OperatorJobFilter {
            states: states.to_vec(),
            owner: None,
            family: None,
            job: None,
            after: None,
        }
    }
    fn listed(state: JobState) -> OperatorJob {
        OperatorJob {
            job: job(),
            owner: "bob".into(),
            family: JobFamily::Container,
            operation: "container_restart".into(),
            state,
            resource: format!("container:{}", "b".repeat(64)),
            claims: 1,
            created_at: 1_790_000_000,
            deadline: 1_790_000_300,
            resolution_open: false,
        }
    }
    fn page(jobs: Vec<OperatorJob>, matching: u32, remaining: u32) -> OperatorJobPage {
        OperatorJobPage {
            jobs,
            matching,
            remaining,
            next: (remaining > 0).then_some(41),
        }
    }

    #[test]
    fn list_parses_filters_limit_position_and_format_in_any_order() {
        assert_eq!(
            parse(&args(&["list"])).unwrap(),
            Command::List(List {
                filter: filter(&UNSETTLED),
                limit: 32,
                json: false
            })
        );
        assert_eq!(
            parse(&args(&[
                "list",
                "--json",
                "--after",
                "41",
                "--family",
                "storage-mount",
                "--owner",
                "bob",
                "--limit",
                "64",
                "--state",
                "needs-intervention,queued,queued"
            ]))
            .unwrap(),
            Command::List(List {
                filter: OperatorJobFilter {
                    owner: Some("bob".into()),
                    family: Some(JobFamily::StorageMount),
                    after: Some(41),
                    ..filter(&[JobState::Queued, JobState::NeedsIntervention])
                },
                limit: 64,
                json: true
            })
        );
        let Command::List(all) = parse(&args(&["list", "--state", "all"])).unwrap() else {
            panic!("list");
        };
        assert_eq!(all.filter.states.len(), 12);
        for bad in [
            vec!["list", "--state"],
            vec!["list", "--state", "--version"],
            vec!["list", "--state", "--help"],
            vec!["list", "--state", "frozen"],
            vec!["list", "--state", "queued,"],
            vec!["list", "--state", ""],
            vec!["list", "--limit", "0"],
            vec!["list", "--limit", "65"],
            vec!["list", "--limit", "-1"],
            vec!["list", "--limit", "ten"],
            vec!["list", "--after", "-1"],
            vec!["list", "--after", "x"],
            vec!["list", "--owner", "not a name"],
            vec!["list", "--owner", "-h"],
            vec!["list", "--family", "containers"],
            vec!["list", "--json", "--json"],
            vec!["list", "--limit", "1", "--limit", "2"],
            vec!["list", "--state", "queued", "--state", "running"],
            vec!["list", "--owner", "a", "--owner", "b"],
            vec!["list", "--family", "stack", "--family", "stack"],
            vec!["list", "--after", "1", "--after", "2"],
            vec!["list", "queued"],
            vec!["list", "--admin", "alice"],
        ] {
            assert!(parse(&args(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn cancel_takes_the_job_then_the_administrator_and_yes() {
        let j = job();
        let cancel = |yes| {
            Command::Cancel(Cancel {
                admin: "alice".into(),
                job: j.clone(),
                yes,
            })
        };
        assert_eq!(
            parse(&args(&["cancel", &j, "--admin", "alice"])).unwrap(),
            cancel(false)
        );
        assert_eq!(
            parse(&args(&["cancel", &j, "--yes", "--admin", "alice"])).unwrap(),
            cancel(true)
        );
        for bad in [
            vec!["cancel"],
            vec!["cancel", &j],
            vec!["cancel", &j, "--yes"],
            vec!["cancel", &j, "--admin"],
            vec!["cancel", &j, "--admin", "--version"],
            vec!["cancel", &j, "--admin", "-h"],
            vec!["cancel", &j, "--admin", "a", "--admin", "b"],
            vec!["cancel", &j, "--admin", "a", "--yes", "--yes"],
            vec!["cancel", "short", "--admin", "alice"],
            vec!["cancel", "--admin", "alice", &j],
            vec!["cancel", &j, "--admin", "alice", "--reason", "x"],
            vec!["cancel", &j, "--outcome", "not-done"],
        ] {
            assert!(parse(&args(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn utc_formats_the_epoch_and_leap_days() {
        assert_eq!(admin::utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(admin::utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(admin::utc(1_790_000_000), "2026-09-21T14:13:20Z");
        assert_eq!(admin::utc(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn the_table_lists_each_job_and_says_how_to_list_the_rest() {
        let mut held = listed(JobState::NeedsIntervention);
        held.resolution_open = true;
        held.created_at = 0;
        let shown = OperatorJobFilter {
            owner: Some("bob".into()),
            ..filter(&UNSETTLED)
        };
        let text = admin::render_table(
            &page(vec![listed(JobState::Queued), held], 5, 3),
            &shown,
            1_790_000_301,
        );
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("JOB "));
        assert!(lines[0].contains("RESOLUTION"));
        assert!(lines[1].starts_with(&job()));
        assert!(lines[1].contains("bob"));
        assert!(lines[1].contains("container_restart"));
        assert!(lines[1].contains("2026-09-21T14:13:20Z"));
        assert!(lines[1].contains("2026-09-21T14:18:20Z (passed)"));
        assert!(lines[2].contains("needs_intervention"));
        assert!(lines[2].contains(" open"));
        assert!(
            lines[3].starts_with("2 shown of 5 matching jobs"),
            "{}",
            lines[3]
        );
        assert!(lines[3].contains("owner: bob"));
        assert_eq!(lines[4], "3 more after these: list them with --after 41");
        assert_eq!(lines.len(), 5);
        // Nothing matches: no table, a plain line.
        let none = admin::render_table(&page(vec![], 0, 0), &filter(&[JobState::Queued]), 0);
        assert_eq!(none, "no jobs match (states: queued)\n");
    }

    #[test]
    fn json_lines_are_one_object_per_job_then_the_totals_with_the_next_position() {
        let text = admin::render_json(&page(vec![listed(JobState::Queued)], 3, 2)).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["job"], job());
        assert_eq!(lines[0]["family"], "container");
        assert_eq!(lines[0]["state"], "queued");
        assert_eq!(lines[0]["resolution_open"], false);
        assert_eq!(
            lines[1],
            serde_json::json!({"shown": 1, "matching": 3, "more": 2, "next": 41})
        );
        let last = admin::render_json(&page(vec![], 3, 0)).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(last.trim()).unwrap(),
            serde_json::json!({"shown": 0, "matching": 3, "more": 0, "next": null})
        );
    }

    fn answer(
        family: JobFamily,
        state: JobState,
        outcome: CancelOutcome,
        refusal: Option<CancelRefusal>,
    ) -> OperatorCancel {
        OperatorCancel {
            job: job(),
            family,
            state,
            outcome,
            refusal,
        }
    }

    #[test]
    fn each_cancel_answer_says_what_happened_and_what_to_do() {
        use CancelOutcome::*;
        let (text, ok) = admin::render_cancel(
            &answer(JobFamily::Stack, JobState::Canceled, Canceled, None),
            "alice",
        );
        assert!(ok && text.starts_with("canceled:") && text.contains("alice"));
        assert!(text.contains("never be dispatched"));
        let (text, ok) = admin::render_cancel(
            &answer(
                JobFamily::Container,
                JobState::Canceled,
                AlreadyCanceled,
                None,
            ),
            "alice",
        );
        assert!(ok && text.contains("already canceled"));
        let (text, ok) = admin::render_cancel(
            &answer(
                JobFamily::Container,
                JobState::NeedsIntervention,
                Refused,
                Some(CancelRefusal::NotQueued),
            ),
            "alice",
        );
        assert!(!ok && text.contains("not queued") && text.contains("job show-resolution"));
        let (text, ok) = admin::render_cancel(
            &answer(
                JobFamily::StorageMount,
                JobState::Running,
                Refused,
                Some(CancelRefusal::NotQueued),
            ),
            "alice",
        );
        assert!(!ok && text.contains("recovery") && !text.contains("show-resolution"));
        let (text, ok) = admin::render_cancel(
            &answer(
                JobFamily::Container,
                JobState::Succeeded,
                Refused,
                Some(CancelRefusal::NotQueued),
            ),
            "alice",
        );
        assert!(!ok && text.contains("succeeded"));
        let (text, ok) = admin::render_cancel(
            &answer(
                JobFamily::AutomationObservation,
                JobState::Queued,
                Refused,
                Some(CancelRefusal::PauseScheduleInstead),
            ),
            "alice",
        );
        assert!(!ok && text.contains("pausing its schedule"));
        assert!(text.contains("precondition_changed"));
    }

    #[test]
    fn the_subject_names_owner_operation_resource_and_queue_time() {
        let text = admin::render_subject(&listed(JobState::Queued));
        for expected in [
            format!("job: {}", job()),
            "owner: bob".into(),
            "operation: container_restart (container)".into(),
            format!("resource: container:{}", "b".repeat(64)),
            "state: queued".into(),
            "queued: 2026-09-21T14:13:20Z".into(),
        ] {
            assert!(text.contains(&expected), "{text}");
        }
        let mut unknown = listed(JobState::Queued);
        unknown.created_at = 0;
        assert!(admin::render_subject(&unknown).contains("queued: unknown"));
    }

    #[tokio::test]
    async fn list_sends_the_filter_and_limit_and_prints_what_core_returned() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, seen) = fake_core(
            &dir,
            vec![CoreResponse::Jobs(page(
                vec![listed(JobState::Queued)],
                1,
                0,
            ))],
        );
        let text = admin::list(
            &socket,
            List {
                filter: OperatorJobFilter {
                    owner: Some("bob".into()),
                    family: Some(JobFamily::Container),
                    after: Some(7),
                    ..filter(&[JobState::Queued])
                },
                limit: 7,
                json: true,
            },
            0,
        )
        .await
        .unwrap();
        assert_eq!(text.lines().count(), 2);
        assert_eq!(
            seen.lock().unwrap()[0],
            serde_json::json!({"request": "list_jobs", "version": VERSION, "filter": {
                "states": ["queued"], "owner": "bob", "family": "container", "job": null, "after": 7
            }, "limit": 7})
        );
    }

    fn found(state: JobState) -> CoreResponse {
        CoreResponse::Jobs(page(vec![listed(state)], 1, 0))
    }
    fn canceled() -> CoreResponse {
        CoreResponse::JobCanceled(answer(
            JobFamily::Container,
            JobState::Canceled,
            CancelOutcome::Canceled,
            None,
        ))
    }
    fn request(yes: bool) -> Cancel {
        Cancel {
            admin: "alice".into(),
            job: job(),
            yes,
        }
    }

    #[tokio::test]
    async fn cancel_with_yes_looks_the_job_up_then_sends_only_the_job_and_administrator() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, seen) = fake_core(
            &dir,
            vec![
                found(JobState::Queued),
                canceled(),
                found(JobState::Running),
                CoreResponse::JobCanceled(answer(
                    JobFamily::Container,
                    JobState::Running,
                    CancelOutcome::Refused,
                    Some(CancelRefusal::NotQueued),
                )),
            ],
        );
        let text = admin::cancel(&socket, request(true), None).await.unwrap();
        assert!(text.starts_with("canceled:"));
        let error = admin::cancel(&socket, request(true), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not queued"));
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0]["request"], "list_jobs");
        assert_eq!(seen[0]["filter"]["job"], job());
        assert_eq!(seen[0]["filter"]["states"].as_array().unwrap().len(), 12);
        assert_eq!(seen[0]["limit"], 1);
        assert_eq!(
            seen[1],
            serde_json::json!({"request": "cancel_job", "version": VERSION, "admin": "alice", "job": job()})
        );
    }

    #[tokio::test]
    async fn without_yes_or_a_terminal_nothing_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, seen) = fake_core(&dir, vec![]);
        let error = admin::cancel(&socket, request(false), None)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("needs a terminal") && error.contains("--yes"),
            "{error}"
        );
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn on_a_terminal_only_typing_cancel_sends_the_cancellation() {
        for answer_typed in ["", "yes", "y", "Cancel", "cancel it"] {
            let dir = tempfile::tempdir().unwrap();
            let (socket, seen) = fake_core(&dir, vec![found(JobState::Queued)]);
            let error = admin::cancel(&socket, request(false), Some(&mut typed(answer_typed)))
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains("not confirmed"),
                "{answer_typed:?}"
            );
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 1, "{answer_typed:?}");
            assert_eq!(seen[0]["request"], "list_jobs");
        }
        let dir = tempfile::tempdir().unwrap();
        let (socket, seen) = fake_core(&dir, vec![found(JobState::Queued), canceled()]);
        let mut asked = 0;
        let mut confirm = || {
            asked += 1;
            Ok("cancel\n".to_owned())
        };
        admin::cancel(&socket, request(false), Some(&mut confirm))
            .await
            .unwrap();
        assert_eq!(asked, 1);
        assert_eq!(seen.lock().unwrap()[1]["request"], "cancel_job");
    }

    #[tokio::test]
    async fn an_unknown_job_is_reported_before_anything_is_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, seen) = fake_core(&dir, vec![CoreResponse::Jobs(page(vec![], 0, 0))]);
        let error = admin::cancel(&socket, request(true), None)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("no job has that ID"), "{error}");
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn with_core_down_the_error_says_so_and_names_the_read_only_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("absent.sock");
        let error = admin::list(
            socket.to_str().unwrap(),
            List {
                filter: filter(&UNSETTLED),
                limit: 32,
                json: false,
            },
            0,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("cannot reach core"), "{error}");
        assert!(error.contains("nothing was sent"), "{error}");
        assert!(error.contains("read-only listing"), "{error}");
        let error = admin::cancel(socket.to_str().unwrap(), request(true), None)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("nothing can cancel a job"), "{error}");
    }

    #[tokio::test]
    async fn core_errors_are_explained_for_these_commands() {
        let dir = tempfile::tempdir().unwrap();
        let envelope = |code| {
            CoreResponse::Error(limeos_contracts::ErrorEnvelope {
                code,
                message: "core said no".into(),
                retry: false,
                audit_id: "rpc-1".into(),
            })
        };
        let (socket, _) = fake_core(
            &dir,
            vec![
                envelope(limeos_domain::ErrorCode::InvalidInput),
                found(JobState::Queued),
                envelope(limeos_domain::ErrorCode::Forbidden),
                found(JobState::Queued),
                envelope(limeos_domain::ErrorCode::NotFound),
            ],
        );
        for expected in [
            "older than this limeosctl",
            "current administrator",
            "No job has that ID",
        ] {
            let error = admin::cancel(&socket, request(true), None)
                .await
                .unwrap_err()
                .to_string();
            assert!(error.starts_with("core said no"), "{error}");
            assert!(error.contains(expected), "{error}");
        }
    }
}
