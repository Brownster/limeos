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
    execute(&socket, resolve(None), &mut stdin).await.unwrap();
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
    let error = execute(&socket, resolve(Some("0".repeat(64))), &mut stdin)
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
    execute(&socket, resolve(view().evidence_digest), &mut stdin)
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
    assert!(execute(&socket, resolve(None), &mut stdin).await.is_err());
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
    let error = execute(&socket, resolve(None), &mut stdin)
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
    assert!(execute(&socket, resolve(None), &mut stdin).await.is_err());
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
    )
    .await
    .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["request"], "read_resolution");
}
