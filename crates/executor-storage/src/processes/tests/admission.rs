use super::*;
use limeos_domain::{ContainerStorageMount, ContainerStorageSource};

const COMBINED: ProcessReadProfile = ProcessReadProfile::CombinedV1;

impl Fake {
    fn combined(&self, d: &ContainerStorageInventory) -> Result<Evidence<Self>> {
        let b = bindings(d);
        COMBINED.admit(d, b.len())?;
        let lease = Lease::new(d, self.clock()?)?;
        let sorted = validate_bindings(d, &b)?;
        collect_with_profile(self.clone(), d, &sorted, lease, COMBINED)
    }
}
fn parse(text: &str) -> Result<Vec<ContainerProcessMount>> {
    parse_mounts_with_profile(text, &|| Ok(()), COMBINED, &mut MountPayload::new(COMBINED))
}

#[test]
fn complete_membership_admission_precedes_all_kernel_opens() {
    let mut d = declaration(16, 100);
    for c in &mut d.containers[8..] {
        c.container.running = false;
    }
    let f = Fake::new(8);
    let e = f.combined(&d).unwrap();
    assert_eq!(e.snapshot.processes.len(), 8);
    assert_eq!(e.declarations.containers.len(), 16);
    assert_eq!(f.counts.active.get(), 37);
    assert_eq!(f.counts.peak.get(), 40); // Fake lacks the temporary Linux fdinfo reader.
    drop(e);
    assert_eq!(f.counts.active.get(), 0);
    for overflow in [declaration(17, 100), declaration(9, 100)] {
        let f = Fake::new(17);
        assert!(matches!(f.combined(&overflow), Err(Failure::Unavailable)));
        assert!(f.state.borrow().calls.is_empty());
        assert_eq!(f.counts.peak.get(), 0);
    }
    assert_eq!(
        COMBINED.admit(&declaration(1, 100), 9),
        Err(Failure::Unavailable)
    );
}

#[test]
fn stopped_declared_mounts_consume_the_same_whole_request_budget() {
    let mut d = declaration(2, 100);
    d.containers[1].container.running = false;
    for i in 0..32 {
        d.containers[1].mounts.push(ContainerStorageMount {
            source: ContainerStorageSource::Bind {
                path: format!("/data/{i}"),
            },
            destination: format!("/data/{i}"),
            writable: true,
        });
    }
    let f = Fake::new(1);
    let e = f.combined(&d).unwrap();
    assert_eq!(e.declarations.containers[1].mounts.len(), 32);
    drop(e);
    d.containers[1].mounts.push(ContainerStorageMount {
        source: ContainerStorageSource::Bind {
            path: "/extra".into(),
        },
        destination: "/extra".into(),
        writable: false,
    });
    let f = Fake::new(1);
    assert!(matches!(f.combined(&d), Err(Failure::Unavailable)));
    assert!(f.state.borrow().calls.is_empty());
}

#[test]
fn exact_tables_and_rows_are_complete_and_the_next_unit_refuses() {
    assert_eq!(parse(&rows(512, 1)).unwrap().len(), 512);
    assert_eq!(parse(&rows(513, 1)), Err(Failure::Unavailable));
    let base = "30 20 8:1 / / rw - ext4  rw\n";
    let table = base.replacen(
        "ext4  ",
        &format!("ext4 {} ", "x".repeat(COMBINED.table_bytes() - base.len())),
        1,
    );
    assert_eq!(table.len(), COMBINED.table_bytes());
    assert_eq!(parse(&table).unwrap().len(), 1);
    assert_eq!(
        parse(&table.replace("ext4 x", "ext4 xx")),
        Err(Failure::Unavailable)
    );
    let checked = Cell::new(0);
    assert!(
        parse_mounts_with_profile(
            &rows(513, 1),
            &|| {
                checked.set(checked.get() + 1);
                Ok(())
            },
            COMBINED,
            &mut MountPayload::new(COMBINED)
        )
        .is_err()
    );
    assert_eq!(checked.get(), 1); // Before any per-row parse/allocation.
}

#[test]
fn decoded_paths_filesystems_and_optional_tokens_refuse_whole_tables() {
    let table = format!(
        "30 20 8:1 / /{} rw - {} x rw\n",
        "\\040".repeat(511),
        "x".repeat(64)
    );
    let mounts = parse(&table).unwrap();
    assert_eq!(mounts[0].mountpoint.len(), 512);
    assert_eq!(mounts[0].filesystem.len(), 64);
    for table in [
        table.replace(" rw - ", "\\040 rw - "),
        table.replace(" x rw", "x x rw"),
        format!("30 20 8:1 / / rw {} - ext4 x rw\n", vec!["a"; 33].join(" ")),
    ] {
        assert_eq!(parse(&table), Err(Failure::Unavailable));
    }
    assert!(
        parse(&format!(
            "30 20 8:1 / / rw {} - ext4 x rw\n",
            vec!["a"; 32].join(" ")
        ))
        .is_ok()
    );
}

#[test]
fn raw_pass_budget_charges_duplicate_namespaces_and_limits_the_next_read() {
    let f = Fake::new(0);
    let d = declaration(0, 100);
    let lease = Lease::new(&d, f.clock().unwrap()).unwrap();
    let budget = Budget::with_profile(&f, &lease, COMBINED).unwrap();
    for pass in 0..3 {
        for _ in 0..4 {
            budget.table(64 * 1024, pass).unwrap();
        }
        assert_eq!(budget.table_limit(pass).unwrap(), 0);
        assert_eq!(budget.table(1, pass), Err(Failure::Unavailable));
        assert_eq!(budget.table_bytes[pass].get(), 256 * 1024);
    }
}

#[test]
fn aggregate_and_shared_parsed_payload_refusals_drop_all_partial_handles() {
    for (table, expected_opens) in [
        (rows(1, COMBINED.table_bytes() - 32), 5),
        (
            rows(512, 1).replace(" / rw", &format!(" /{} rw", "x".repeat(25))),
            8,
        ),
    ] {
        if expected_opens == 8 {
            assert!(table.len() * 8 <= COMBINED.aggregate_bytes());
        } else {
            assert!(table.len() <= COMBINED.table_bytes());
            assert!(table.len() * 4 <= COMBINED.aggregate_bytes());
            assert!(table.len() * 5 > COMBINED.aggregate_bytes());
        }
        let f = Fake::new(8);
        for p in f.state.borrow_mut().processes.values_mut() {
            p.table = table.clone();
        }
        let d = declaration(8, 100);
        assert!(matches!(f.combined(&d), Err(Failure::Unavailable)));
        assert_eq!(f.counts.active.get(), 0);
        assert!(f.counts.peak.get() > 5);
        assert_eq!(f.state.borrow().calls["pidfd"], expected_opens);
    }
}

#[test]
fn retained_profile_is_used_again_and_failure_cannot_widen_it() {
    for pass in 1..=2 {
        let f = Fake::new(1);
        let d = declaration(1, 100);
        let e = f.combined(&d).unwrap();
        assert_eq!(e.profile, COMBINED);
        let next = f.state.borrow().calls["table"] + pass;
        f.hook("table", next, Change::Table(rows(1, 64 * 1024)));
        assert_eq!(e.revalidate(), Err(Failure::Unavailable));
        f.state.borrow_mut().processes.get_mut(&200).unwrap().table = TABLE.into();
        assert_eq!(e.revalidate(), Err(Failure::Conflict));
        drop(e);
        assert_eq!(f.counts.active.get(), 0);
    }
}

#[test]
fn original_lease_survives_revalidation_and_expiry_drops_no_membership() {
    let f = Fake::new(8);
    let d = declaration(8, 100);
    let e = f.combined(&d).unwrap();
    for second in 101..105 {
        f.state.borrow_mut().wall = Duration::from_secs(second);
        assert_eq!(e.revalidate(), Ok(()));
        assert_eq!(e.snapshot.declarations_observed_at, 100);
        assert_eq!(e.snapshot.processes.len(), 8);
    }
    f.state.borrow_mut().wall = Duration::from_secs(105);
    assert_eq!(e.revalidate(), Err(Failure::Conflict));
}

#[test]
fn standard_profile_keeps_existing_counts_and_large_fields() {
    let f = Fake::new(9);
    let d = declaration(9, 100);
    assert!(f.inspect(&d, &bindings(&d)).is_ok());
    assert!(matches!(f.combined(&d), Err(Failure::Unavailable)));
    let table = format!(
        "30 20 8:1 / /{} rw - {} x rw\n",
        "x".repeat(512),
        "x".repeat(65)
    );
    assert!(parse_mounts(&table, &|| Ok(())).is_ok());
    assert!(parse(&table).is_err());
}

#[test]
fn combined_maximum_row_payload_measurement() {
    let f = Fake::new(8);
    let d = declaration(8, 100);
    let table = rows(512, 1);
    for p in f.state.borrow_mut().processes.values_mut() {
        p.table = table.clone();
    }
    let e = f.combined(&d).unwrap();
    let vector_bytes: usize = e
        .retained
        .iter()
        .map(|r| r.mounts.capacity() * size_of::<ContainerProcessMount>())
        .sum();
    let string_bytes: usize = e
        .retained
        .iter()
        .flat_map(|r| &r.mounts)
        .map(|m| m.filesystem_root.capacity() + m.mountpoint.capacity() + m.filesystem.capacity())
        .sum();
    assert!(vector_bytes + string_bytes <= 512 * 1024);
    assert_eq!(
        e.snapshot
            .processes
            .iter()
            .map(|p| p.mount_count)
            .sum::<usize>(),
        4096
    );
    assert_eq!(f.counts.active.get(), 37);
    assert_eq!(e.revalidate(), Ok(()));
    println!(
        "combined_payload_measurement processes=8 rows=4096 raw_table_bytes={} mount_vector_bytes={vector_bytes} mount_string_bytes={string_bytes} retained_descriptors=37 peak_modeled_descriptors={} authentication=private_fixture",
        table.len() * 8,
        f.counts.peak.get()
    );
    drop(e);
    assert_eq!(f.counts.active.get(), 0);
}

#[test]
fn combined_real_owned_children_descriptor_measurement() {
    const FLAG: &str = "LIMEOS_COMBINED_PROCESS_FD_PROBE";
    if std::env::var_os(FLAG).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "processes::tests::admission::combined_real_owned_children_descriptor_measurement",
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
        return;
    }
    let children: Vec<_> = (0..8).map(|_| OwnedChild::new()).collect();
    let baseline = fd_count();
    let kernel = LocalChildren::default();
    let now = kernel.clock().unwrap();
    let d = declaration(8, now.wall.as_secs() as i64);
    let mut b = bindings(&d);
    for (binding, child) in b.iter_mut().zip(&children) {
        binding.pid = child.0.id();
    }
    let sorted = validate_bindings(&d, &b).unwrap();
    let e =
        collect_with_profile(kernel, &d, &sorted, Lease::new(&d, now).unwrap(), COMBINED).unwrap();
    assert_eq!(fd_count(), baseline + 37);
    assert_eq!(e.revalidate(), Ok(()));
    assert!(e.kernel.peak_fds.get() <= baseline + 41);
    println!(
        "combined_real_fd_measurement processes=8 baseline={baseline} retained={} peak={} host_namespace_authentication=private_substitution",
        fd_count() - baseline,
        e.kernel.peak_fds.get() - baseline
    );
    drop(e);
    assert_eq!(fd_count(), baseline);
    drop(children);
    assert_eq!(fd_count(), baseline);
}
