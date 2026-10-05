use super::*;
fn setup() -> (
    tempfile::TempDir,
    Store,
    Principal,
    StorageInventory,
    StorageSetupInput,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    store.issue_bootstrap("bootstrap", 100).unwrap();
    store
        .enroll("bootstrap", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let principal = store.login_record("alice").unwrap().unwrap().principal;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    let contract: limeos_domain::StorageContract =
        serde_json::from_value(fixture["cases"][0]["contract"].clone()).unwrap();
    let inventory = StorageInventory {
        host_root_mount_id: 1,
        topology_digest: "a".repeat(64),
        mounts_digest: "b".repeat(64),
        fstab_digest: "c".repeat(64),
        fstab_entries: vec![],
        devices: vec![limeos_domain::StorageObservedDevice {
            device: limeos_domain::StorageBlockDevice { major: 8, minor: 1 },
            filesystem_uuid: contract.devices[0].filesystem_uuid.clone(),
            filesystem: "ext4".into(),
            serial: None,
            boot_backing: false,
            in_use_as_swap: false,
            mounts: vec![],
        }],
    };
    let input = StorageSetupInput {
        contract,
        inventory_digest: limeos_identity::digest(&json(&inventory).unwrap()),
        timeout_seconds: 10,
    };
    (dir, store, principal, inventory, input)
}
#[test]
fn restart_preserves_preview_approval_read_is_pure_and_no_host_job_exists() {
    let (dir, mut store, p, inventory, input) = setup();
    let proposal = store.plan_storage(&p, &input, &inventory, 200).unwrap();
    let approval = store
        .approve_storage(&p, &proposal.plan.id, &proposal.digest, &inventory, 201)
        .unwrap();
    let hashed: String = store
        .conn
        .query_row("SELECT approval_digest FROM storage_plans", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(hashed, limeos_identity::digest(&approval.token));
    assert_ne!(hashed, approval.token);
    drop(store);
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let before = store.conn.total_changes();
    assert_eq!(
        store.storage_plan(&p, &proposal.plan.id, 202).unwrap(),
        proposal
    );
    assert_eq!(store.conn.total_changes(), before);
    assert_eq!(
        store
            .conn
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    store.cancel_storage(&p, &proposal.plan.id, 203).unwrap();
    assert_eq!(
        store
            .storage_plan(&p, &proposal.plan.id, 204)
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    assert_eq!(
        store
            .conn
            .query_row("SELECT approval_digest FROM storage_plans", [], |r| r
                .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
}
#[test]
fn changed_inventory_replaced_files_fstab_edits_grants_expiry_and_digest_block_approval() {
    let (_dir, mut store, p, inventory, input) = setup();
    let proposal = store.plan_storage(&p, &input, &inventory, 200).unwrap();
    for field in [
        "fstab",
        "topology",
        "mounts",
        "namespace",
        "serial",
        "uuid",
        "device",
    ] {
        let mut changed = inventory.clone();
        match field {
            "fstab" => changed.fstab_digest = "d".repeat(64),
            "topology" => changed.topology_digest = "d".repeat(64),
            "mounts" => changed.mounts_digest = "d".repeat(64),
            "namespace" => changed.host_root_mount_id += 1,
            "serial" => changed.devices[0].serial = Some("replacement".into()),
            "uuid" => changed.devices[0].filesystem_uuid = "replaced".into(),
            _ => changed.devices[0].device.minor += 1,
        }
        assert_eq!(
            store
                .approve_storage(&p, &proposal.plan.id, &proposal.digest, &changed, 201)
                .err()
                .unwrap()
                .0,
            ErrorCode::Conflict,
            "{field}"
        );
        assert_eq!(
            store.plan_storage(&p, &input, &changed, 201).unwrap_err().0,
            ErrorCode::Conflict
        );
    }
    assert_eq!(
        store
            .approve_storage(&p, &proposal.plan.id, &"a".repeat(64), &inventory, 201)
            .err()
            .unwrap()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(
        store
            .approve_storage(&p, &proposal.plan.id, &proposal.digest, &inventory, 500)
            .err()
            .unwrap()
            .0,
        ErrorCode::Expired
    );
    store
        .revise_grants(&p.id, Role::Administrator, &[], 201)
        .unwrap();
    assert_eq!(
        store
            .approve_storage(&p, &proposal.plan.id, &proposal.digest, &inventory, 202)
            .err()
            .unwrap()
            .0,
        ErrorCode::Expired
    );
}
#[test]
fn storage_requires_current_admin_grant_owner_and_durable_audit_space() {
    let (_dir, mut store, p, inventory, input) = setup();
    let proposal = store.plan_storage(&p, &input, &inventory, 200).unwrap();
    let other = Principal {
        id: "other".into(),
        ..p.clone()
    };
    assert!(store.storage_plan(&other, &proposal.plan.id, 201).is_err());
    store
        .conn
        .execute("UPDATE meta SET audit_bytes=?", [AUDIT_LIMIT])
        .unwrap();
    assert_eq!(
        store
            .plan_storage(&p, &input, &inventory, 202)
            .unwrap_err()
            .0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(
        store
            .approve_storage(&p, &proposal.plan.id, &proposal.digest, &inventory, 202)
            .err()
            .unwrap()
            .0,
        ErrorCode::StateNotDurable
    );
    for role in [Role::Viewer, Role::MediaRequester, Role::Operator] {
        let (_dir, mut store, p, inventory, input) = setup();
        store
            .revise_grants(&p.id, role, &[storage_scope()], 201)
            .unwrap();
        let current = store.login_record("alice").unwrap().unwrap().principal;
        assert_eq!(
            store
                .plan_storage(&current, &input, &inventory, 202)
                .unwrap_err()
                .0,
            ErrorCode::Forbidden
        );
    }
}
#[test]
fn v5_migration_is_atomic_preserves_users_and_does_not_mutate_failed_v5() {
    for fail in [false, true] {
        let (dir, store, _p, _inventory, _input) = setup();
        store
            .conn
            .execute_batch("DROP TABLE storage_plans; PRAGMA user_version=5;")
            .unwrap();
        if fail {
            store
                .conn
                .execute_batch("CREATE INDEX storage_plans_expiry ON users(username);")
                .unwrap();
        }
        let previous = store
            .conn
            .query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))
            .unwrap();
        drop(store);
        let path = dir.path().join("core.sqlite");
        let reopened = Store::open(&path);
        if fail {
            assert_eq!(reopened.err().unwrap().0, ErrorCode::StateNotDurable);
        }
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            if fail { 5 } else { 6 }
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            previous
        );
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='storage_plans'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            if fail { 0 } else { 1 }
        );
    }
}
