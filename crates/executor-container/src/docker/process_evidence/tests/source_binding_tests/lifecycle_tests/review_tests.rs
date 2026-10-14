use super::*;
use crate::compose_management::sources::lifecycle::review::tests::{
    LocalReview, bind as review_bind, count, hash_material, material_value,
};
use crate::compose_management::{
    catalog,
    tests::{Inputs, local_files},
};
use limeos_domain::{ComposeFile, ComposePlan, ComposeSelection, Operation, PlannedCompose};

fn signed(preview: &mut PlannedCompose) {
    preview.digest = limeos_identity::digest(&serde_json::to_string(&preview.plan).unwrap());
}
fn preview(p: &LocalProposal) -> PlannedCompose {
    let s = p.owner.summary();
    let at = crate::compose_files::now().unwrap();
    let mut preview = PlannedCompose {
        digest: String::new(),
        plan: ComposePlan {
            id: "c".repeat(64),
            version: 1,
            principal: "owner".into(),
            grant_revision: 1,
            catalog_digest: "d".repeat(64),
            template_digest: limeos_identity::digest(
                &serde_json::to_string(s.proposal.desired).unwrap(),
            ),
            selection: ComposeSelection {
                stack: s.evidence.binding.registration.stack.clone(),
                template: "update".into(),
            },
            operator_files: vec![ComposeFile {
                name: s.evidence.binding.registration.operator.name,
                sha256: s.evidence.binding.files.files[0].sha256.clone(),
            }],
            managed_file: limeos_domain::COMPOSE_MANAGED_FILE.into(),
            before: s.proposal.before.clone(),
            desired: s.proposal.desired.clone(),
            impact: s.proposal.impact.clone(),
            created_at: at,
            expires_at: at + 300,
        },
    };
    signed(&mut preview);
    preview
}
async fn subject(f: &Fixture) -> LocalReview {
    let p = proposal(f, true).await;
    let input = preview(&p);
    let r = review_bind(p, &input).unwrap();
    r.revalidate(&|| false).await.unwrap();
    r
}
fn resign_template(p: &mut PlannedCompose) {
    p.plan.template_digest =
        limeos_identity::digest(&serde_json::to_string(&p.plan.desired).unwrap());
    signed(p);
}

#[tokio::test]
async fn preview_review_subject_binds_original_complete_facts_and_distinct_exact_scopes() {
    let f = fixture(Config {
        mounts: 1,
        ..config(true)
    })
    .await;
    let r = subject(&f).await;
    let s = r.owner.summary();
    assert_ne!(s.review_digest, s.preview.digest);
    assert_ne!(s.review_digest, s.lifecycle.artifact_sha256);
    assert_eq!(s.required_scopes.len(), 2);
    assert_eq!(s.required_scopes[0].operation, Operation::DeploymentManage);
    assert_eq!(
        s.required_scopes[0].resource,
        s.lifecycle.evidence.binding.stack_resource
    );
    assert_eq!(
        s.required_scopes[1].operation,
        Operation::DeploymentElevated
    );
    assert_eq!(
        s.required_scopes[1].resource,
        format!(
            "stack:media-stack:template:update:{}",
            s.preview.plan.template_digest
        )
    );
    assert_eq!(s.preview.plan.before, *s.lifecycle.proposal.before);
    assert_eq!(s.preview.plan.desired, *s.lifecycle.proposal.desired);
    assert_eq!(s.lifecycle.unresolved.len(), 8);
    let value = material_value(&r.owner);
    assert_eq!(value["domain"], "limeos.compose.review.v1");
    assert_eq!(value["engine_context"]["running"][0]["pid"], 1000);
    assert_eq!(
        value["engine_context"]["containers"][0]["mounts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(value["engine_context"]["socket"]["inode"].as_u64().unwrap() > 0);
    assert_eq!(
        r.owner.artifact().len(),
        s.lifecycle.proposal.artifact_bytes
    );
    assert!(r.owner.retained_descriptor_count() <= 38);
    let before = serde_json::to_vec(&s).unwrap();
    r.revalidate(&|| false).await.unwrap();
    assert_eq!(before, serde_json::to_vec(&r.owner.summary()).unwrap());
    let text = String::from_utf8(before).unwrap();
    for secret in ["private-source-value", "SYNTHETIC_SECRET", "approval_token"] {
        assert!(!text.contains(secret));
    }
    assert!(f.requests().iter().all(|v| v.starts_with("GET ")));
}

#[tokio::test]
async fn ordinary_desired_project_requires_only_exact_stack_manage_scope() {
    let f = fixture(Config {
        count: 0,
        ..config(false)
    })
    .await;
    let source = from_binding(
        super::super::bind(
            inputs(b"{\"services\":{}}", None, |c| {
                c["stacks"][0]["services"] = json!(["worker"]);
            })
            .unwrap(),
            engine(&f).await.unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let p = propose(
        source,
        &ComposeProject {
            services: vec![worker()],
        },
    )
    .unwrap();
    p.revalidate(&|| false).await.unwrap();
    let input = preview(&p);
    let r = review_bind(p, &input).unwrap();
    r.revalidate(&|| false).await.unwrap();
    assert_eq!(r.owner.summary().required_scopes.len(), 1);
}

#[tokio::test]
async fn modified_preview_preimage_desired_impact_stack_files_and_digests_refuse_before_cloning() {
    let f = fixture(config(true)).await;
    for problem in 0..10 {
        let p = proposal(&f, false).await;
        let mut input = preview(&p);
        match problem {
            0 => input.plan.before.services[0].user = Some(ComposeUser { uid: 1, gid: 1 }),
            1 => input.plan.desired.services[0].privileged = false,
            2 => input.plan.impact.services.clear(),
            3 => input.plan.selection.stack = "different".into(),
            4 => input.plan.operator_files[0].sha256 = "f".repeat(64),
            5 => input
                .plan
                .operator_files
                .push(input.plan.operator_files[0].clone()),
            6 => input.plan.template_digest = "e".repeat(64),
            7 => input.plan.desired.services = vec![worker(); 17],
            8 => input.plan.before.services[0].image = "x".repeat(100_000),
            _ => input.digest = "a".repeat(64),
        }
        if problem != 9 {
            signed(&mut input);
        }
        refusal(review_bind(p, &input), ErrorCode::Conflict);
    }
}

#[tokio::test]
async fn expired_or_malformed_preview_and_invalidated_proposal_cannot_create_subject() {
    let f = fixture(config(true)).await;
    for problem in 0..3 {
        let p = proposal(&f, false).await;
        let mut input = preview(&p);
        let expected = match problem {
            0 => {
                input.plan.created_at -= 301;
                input.plan.expires_at -= 301;
                ErrorCode::Expired
            }
            1 => {
                input.plan.grant_revision = 0;
                ErrorCode::InvalidInput
            }
            _ => {
                refusal(p.revalidate(&|| true).await, ErrorCode::Conflict);
                ErrorCode::Conflict
            }
        };
        signed(&mut input);
        refusal(review_bind(p, &input), expected);
    }
}

#[tokio::test]
async fn current_preview_owner_revision_selection_catalog_and_review_digest_cannot_change() {
    let f = fixture(config(true)).await;
    for problem in 0..8 {
        let r = subject(&f).await;
        let mut current = r.owner.summary().preview.clone();
        let mut expected = r.owner.summary().review_digest.to_owned();
        match problem {
            0 => current.plan.principal = "other".into(),
            1 => current.plan.grant_revision += 1,
            2 => current.plan.id = "e".repeat(64),
            3 => current.plan.catalog_digest = "e".repeat(64),
            4 => current.plan.selection.template = "other".into(),
            5 => current.plan.expires_at += 1,
            6 => expected = current.digest.clone(),
            _ => expected = "e".repeat(64),
        }
        if problem < 6 {
            signed(&mut current);
        }
        let requests = f.requests().len();
        refusal(
            r.check_current(&current, &expected, &|| false).await,
            ErrorCode::Conflict,
        );
        assert_eq!(requests, f.requests().len());
        r.check_proposal_only(&|| false).await.unwrap();
        refusal(r.revalidate(&|| false).await, ErrorCode::Conflict);
    }
}

#[tokio::test]
async fn copied_input_mutation_cannot_change_retained_review_and_approval_is_no_input() {
    let f = fixture(config(true)).await;
    let p = proposal(&f, false).await;
    let mut input = preview(&p);
    let r = review_bind(p, &input).unwrap();
    let original = serde_json::to_vec(&r.owner.summary()).unwrap();
    input.plan.principal = "later".into();
    resign_template(&mut input);
    r.revalidate(&|| false).await.unwrap();
    assert_eq!(original, serde_json::to_vec(&r.owner.summary()).unwrap());
    let value = serde_json::to_value(r.owner.summary()).unwrap();
    for key in [
        "approval",
        "approval_token",
        "receipt",
        "authorized",
        "grant",
        "locks",
    ] {
        assert!(value.get(key).is_none());
    }
}

#[tokio::test]
async fn review_hash_projection_excludes_collection_clocks_and_keeps_incarnation_and_file_times() {
    let f = fixture(config(true)).await;
    let r = subject(&f).await;
    let material = material_value(&r.owner);
    let text = serde_json::to_string(&material).unwrap();
    assert!(!text.contains("observed_at"));
    for key in [
        "created_at",
        "expires_at",
        "started_at",
        "modified_seconds",
        "changed_seconds",
    ] {
        assert!(text.contains(key));
    }
    assert_eq!(
        material["preview"]["digest"],
        r.owner.summary().preview.digest
    );
    assert_eq!(material["engine_members"].as_array().unwrap().len(), 2);
}

async fn recollect(f: &Fixture, r: LocalReview) -> LocalReview {
    let input = r.owner.summary().preview.clone();
    let LocalReview { owner, tree } = r;
    drop(owner);
    let catalog =
        catalog::local_catalog(tree.path(), std::path::Path::new("/config/catalog.json")).unwrap();
    let selection = catalog.catalog().stacks[0].selection().unwrap();
    let files = local_files(tree.path(), &selection).unwrap();
    let (_wall, time) = clock(100);
    let e = EngineComposeEvidence::from_storage(
        inspect_selected(
            &f.docker,
            f.policy,
            time,
            Some(Admission::new(EngineReadProfile::CombinedV1)),
            true,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    let source = from_binding(
        super::super::bind(
            Inputs {
                tree,
                catalog,
                files,
            },
            e,
        )
        .unwrap(),
    )
    .unwrap();
    let p = propose(source, &input.plan.desired).unwrap();
    p.revalidate(&|| false).await.unwrap();
    let r = review_bind(p, &input).unwrap();
    r.revalidate(&|| false).await.unwrap();
    r
}

#[tokio::test]
async fn genuine_recollection_of_identical_named_objects_matches_despite_new_engine_issue_time() {
    let f = fixture(Config {
        mounts: 0,
        ..config(true)
    })
    .await;
    let r = subject(&f).await;
    let digest = r.owner.summary().review_digest.to_owned();
    let engine_issue = r
        .owner
        .summary()
        .lifecycle
        .evidence
        .binding
        .engine
        .containers_observed_at;
    let r = recollect(&f, r).await;
    assert_ne!(
        engine_issue,
        r.owner
            .summary()
            .lifecycle
            .evidence
            .binding
            .engine
            .containers_observed_at
    );
    assert_eq!(digest, r.owner.summary().review_digest);
}

#[tokio::test]
async fn genuine_new_pid_or_declared_mounts_require_a_distinct_review_subject() {
    for pid in [true, false] {
        let f = fixture(Config {
            mounts: 0,
            ..config(true)
        })
        .await;
        let r = subject(&f).await;
        let old = r.owner.summary().review_digest.to_owned();
        if pid {
            f.set(|c| c.pid = Some(json!(2100)));
        } else {
            f.set(|c| c.mounts = 1);
        }
        let r = recollect(&f, r).await;
        assert_ne!(old, r.owner.summary().review_digest);
        let current = r.owner.summary().preview.clone();
        refusal(
            r.check_current(&current, &old, &|| false).await,
            ErrorCode::Conflict,
        );
    }
}

#[tokio::test]
async fn real_source_or_engine_changes_and_spent_original_reads_poison_review() {
    for change in 0..3 {
        let f = fixture(config(true)).await;
        let r = subject(&f).await;
        match change {
            0 => fs::write(
                r.tree.path().join("opt/stacks/media/limeos.override.json"),
                b"{}",
            )
            .unwrap(),
            1 => f.set(|c| c.changed = Some(Change::Pid)),
            _ => r.exhaust_reads(),
        }
        assert!(r.revalidate(&|| false).await.is_err());
        let requests = f.requests().len();
        refusal(r.revalidate(&|| false).await, ErrorCode::Conflict);
        assert_eq!(requests, f.requests().len());
    }
}

#[tokio::test]
async fn cancelled_review_cannot_be_restored_by_successful_original_proposal() {
    let f = fixture(config(true)).await;
    let r = subject(&f).await;
    refusal(r.revalidate(&|| true).await, ErrorCode::Conflict);
    r.check_proposal_only(&|| false).await.unwrap();
    let requests = f.requests().len();
    refusal(r.revalidate(&|| false).await, ErrorCode::Conflict);
    assert_eq!(requests, f.requests().len());
}

#[tokio::test]
async fn dropped_and_original_timeout_review_checks_close_owned_engine_request_and_poison() {
    for timeout in [false, true] {
        let f = fixture(config(true)).await;
        let mut r = subject(&f).await;
        if timeout {
            r.expire(Duration::from_millis(40));
        }
        f.set(|c| c.hold = true);
        if timeout {
            refusal(r.revalidate(&|| false).await, ErrorCode::Expired);
        } else {
            let mut check = Box::pin(r.revalidate(&|| false));
            tokio::select! {result=&mut check=>panic!("expected held request: {result:?}"),()=f.held.notified()=>{}}
            drop(check);
        }
        tokio::time::timeout(Duration::from_secs(1), f.eof.notified())
            .await
            .unwrap();
        f.set(|c| c.hold = false);
        refusal(r.revalidate(&|| false).await, ErrorCode::Conflict);
    }
}

#[tokio::test]
async fn concurrent_reentrant_and_unwound_review_checks_cannot_restore_admission() {
    let f = fixture(config(true)).await;
    let r = subject(&f).await;
    let (a, b) = tokio::join!(r.revalidate(&|| false), r.revalidate(&|| false));
    refusal(a, ErrorCode::Conflict);
    refusal(b, ErrorCode::Conflict);
    let r = subject(&f).await;
    let called = AtomicBool::new(false);
    refusal(
        r.revalidate(&|| {
            if !called.swap(true, Ordering::SeqCst) {
                let mut nested = Box::pin(r.revalidate(&|| false));
                let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
                assert!(matches!(
                    std::future::Future::poll(nested.as_mut(), &mut cx),
                    std::task::Poll::Ready(Err(Error(ErrorCode::Conflict)))
                ));
            }
            false
        })
        .await,
        ErrorCode::Conflict,
    );
    let r = subject(&f).await;
    let cancelled = || panic!("owned review subject unwind");
    let mut check = Box::pin(r.revalidate(&cancelled));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            std::future::Future::poll(check.as_mut(), &mut cx)
        }))
        .is_err()
    );
    drop(check);
    refusal(r.revalidate(&|| false).await, ErrorCode::Conflict);
}

#[tokio::test]
async fn borrowed_review_summary_and_streaming_hash_count_exact_whole_bytes() {
    let f = fixture(config(true)).await;
    let r = subject(&f).await;
    let s = r.owner.summary();
    let n = serde_json::to_vec(&s).unwrap().len();
    count(&s, n).unwrap();
    refusal(count(&s, n - 1), ErrorCode::Unavailable);
    let material = material_value(&r.owner);
    let material_bytes = serde_json::to_vec(&material).unwrap().len();
    assert!(material_bytes < 256 * 1024);
    assert_eq!(
        hash_material(&r.owner, 256 * 1024).unwrap(),
        s.review_digest
    );
    assert_eq!(
        hash_material(&r.owner, material_bytes).unwrap(),
        s.review_digest
    );
    refusal(
        hash_material(&r.owner, material_bytes - 1),
        ErrorCode::Unavailable,
    );
}

#[tokio::test]
async fn late_cancellation_after_engine_bytes_permanently_invalidates_review_subject() {
    let f = fixture(config(true)).await;
    let r = subject(&f).await;
    let requests = f.requests().len();
    refusal(
        r.revalidate(&|| f.requests().len() > requests).await,
        ErrorCode::Conflict,
    );
    refusal(r.revalidate(&|| false).await, ErrorCode::Conflict);
}

#[tokio::test]
async fn preview_expiry_during_genuine_child_validation_is_checked_before_success() {
    let f = fixture(config(true)).await;
    let p = proposal(&f, false).await;
    let mut input = preview(&p);
    input.plan.created_at = crate::compose_files::now().unwrap() - 298;
    input.plan.expires_at = input.plan.created_at + 300;
    signed(&mut input);
    let r = review_bind(p, &input).unwrap();
    r.revalidate(&|| false).await.unwrap();
    let requests = f.requests().len();
    refusal(r.expire_preview_during_child().await, ErrorCode::Expired);
    assert!(f.requests().len() > requests);
    refusal(r.revalidate(&|| false).await, ErrorCode::Conflict);
}
