use super::*;

#[derive(Default)]
struct CountCreation {
    attempts: usize,
}
impl CaptureIo for CountCreation {
    fn checkpoint(&mut self, point: Point, _: &File) -> Result<(), Failure> {
        if point == Point::AttemptCreated {
            self.attempts += 1;
        }
        Ok(())
    }
}

#[test]
fn preflight_sizes_real_sources_without_copying_or_moving_offsets() {
    let f = Fixture::new(CHUNK + 1);
    let mut file = &f.file;
    file.seek(SeekFrom::Start(1)).unwrap();
    let sources = f.sources();
    let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
    let required = planned.requirements();
    assert_eq!(required.objects, 2);
    assert_eq!(required.files, 1);
    assert_eq!(required.directories, 1);
    assert_eq!(required.logical_file_bytes, (CHUNK + 1) as u64);
    assert_eq!(required.largest_file_bytes, (CHUNK + 1) as u64);
    assert_eq!(required.retained_descriptors, 4);
    assert_eq!(required.transient_descriptors, 1);
    assert_eq!(required.streaming_buffer_bytes, 65536);
    assert!(required.bound_metadata_capacity_bytes > 0);
    assert_eq!(required.record_byte_limit, limits().max_record_bytes);
    assert_eq!(planned.limits(), limits());
    assert_eq!(file.stream_position().unwrap(), 1);
    assert!(!format!("{planned:?}").contains("application"));
    assert!(!format!("{planned:?}").contains("subdir"));
    assert!(!format!("{planned:?}").contains(f.private().to_str().unwrap()));
    planned.revalidate(&|| false).unwrap();
    f.assert_clean();
}

#[test]
fn declined_preflight_needs_no_cleanup_and_creates_no_name() {
    let f = Fixture::new(17);
    let sources = f.sources();
    let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
    drop(planned);
    f.assert_clean();
    assert!(fs::read_dir(f.private()).unwrap().next().is_none());
}

#[test]
fn consumed_preflight_preserves_canonical_record_and_verified_bytes() {
    let f = Fixture::new(17);
    let sources = f.sources();
    let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
    let owner = capture_planned(planned, &|| false).unwrap();
    let mut bytes = Vec::new();
    owner
        .entries()
        .next()
        .unwrap()
        .reader()
        .unwrap()
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, f.expected);
    let value: serde_json::Value = serde_json::from_slice(owner.record()).unwrap();
    assert_eq!(value["format_version"], 1);
    assert_eq!(value["limits"], serde_json::to_value(limits()).unwrap());
    assert_eq!(value["objects"].as_array().unwrap().len(), 2);
    let mut digest = Sha256::new();
    digest.update(b"limeos.backup-before-images.record.v1\0");
    digest.update(owner.record());
    assert_eq!(owner.record_sha256(), hex::encode(digest.finalize()));
    let compatible = capture(&sources, &f.root, limits(), &|| false).unwrap();
    assert_eq!(owner.record(), compatible.record());
    assert_eq!(owner.record_sha256(), compatible.record_sha256());
    compatible.discard().unwrap();
    owner.discard().unwrap();
    f.assert_clean();
}

#[test]
fn grown_source_refuses_before_attempt_creation_without_replanning() {
    let f = Fixture::new(17);
    let sources = f.sources();
    let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
    fs::write(
        f.tree.path().join("managed/config"),
        b"changed and larger than the original",
    )
    .unwrap();
    assert_eq!(planned.requirements().logical_file_bytes, 17);
    assert!(matches!(
        planned.revalidate(&|| false).unwrap_err().failure,
        Failure::ChangedSource
    ));
    let mut io = CountCreation::default();
    let error = capture_planned_with_io(planned, &|| false, &mut io).unwrap_err();
    assert!(matches!(error.failure, Failure::ChangedSource));
    assert!(error.cleanup.is_none());
    assert_eq!(io.attempts, 0);
    assert_eq!(fs::read_dir(f.private()).unwrap().count(), 0);
}

#[test]
fn replaced_name_never_redirects_the_original_preflight() {
    let f = Fixture::new(17);
    let sources = f.sources();
    let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
    fs::rename(
        f.tree.path().join("managed/config"),
        f.tree.path().join("managed/old-config"),
    )
    .unwrap();
    fs::write(f.tree.path().join("managed/config"), b"replacement").unwrap();
    let mut io = CountCreation::default();
    let error = capture_planned_with_io(planned, &|| false, &mut io).unwrap_err();
    assert!(matches!(error.failure, Failure::ChangedSource));
    assert!(error.cleanup.is_none());
    assert_eq!(io.attempts, 0);
    assert_eq!(fs::read_dir(f.private()).unwrap().count(), 0);
    assert_eq!(
        fs::read(f.tree.path().join("managed/config")).unwrap(),
        b"replacement"
    );
}

#[test]
fn changed_source_modes_and_parent_metadata_refuse_before_copy() {
    for parent in [false, true] {
        let f = Fixture::new(17);
        let sources = f.sources();
        let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
        if parent {
            fs::write(f.tree.path().join("managed/new-child"), b"unselected").unwrap();
        } else {
            fs::set_permissions(
                f.tree.path().join("managed/config"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        let mut io = CountCreation::default();
        let error = capture_planned_with_io(planned, &|| false, &mut io).unwrap_err();
        assert!(matches!(error.failure, Failure::ChangedSource));
        assert!(error.cleanup.is_none());
        assert_eq!(io.attempts, 0);
        f.assert_clean();
    }
}

#[test]
fn a_changed_last_source_prevents_even_the_first_private_copy() {
    let f = Fixture::new(17);
    let sources = f.sources();
    let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
    fs::write(
        f.tree.path().join("managed/subdir/new-child"),
        b"not selected",
    )
    .unwrap();
    let mut io = CountCreation::default();
    let error = capture_planned_with_io(planned, &|| false, &mut io).unwrap_err();
    assert!(matches!(error.failure, Failure::ChangedSource));
    assert!(error.cleanup.is_none());
    assert_eq!(io.attempts, 0);
    f.assert_clean();
}

#[test]
fn changed_root_protection_refuses_before_creation() {
    let f = Fixture::new(17);
    let sources = f.sources();
    let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
    fs::set_permissions(f.private(), fs::Permissions::from_mode(0o755)).unwrap();
    let mut io = CountCreation::default();
    let error = capture_planned_with_io(planned, &|| false, &mut io).unwrap_err();
    assert!(matches!(error.failure, Failure::UnsafeRoot));
    assert!(error.cleanup.is_none());
    assert_eq!(io.attempts, 0);
    fs::set_permissions(f.private(), fs::Permissions::from_mode(0o700)).unwrap();
    f.assert_clean();
}

#[test]
fn original_root_handle_survives_path_replacement_without_adoption() {
    let f = Fixture::new(17);
    let sources = f.sources();
    let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
    let moved = f.tree.path().join("original-private");
    fs::rename(f.private(), &moved).unwrap();
    fs::create_dir(f.private()).unwrap();
    fs::set_permissions(f.private(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(f.private().join("foreign"), b"untouched").unwrap();
    let owner = capture_planned(planned, &|| false).unwrap();
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 1);
    assert_eq!(fs::read_dir(f.private()).unwrap().count(), 1);
    owner.discard().unwrap();
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 0);
    assert_eq!(fs::read(f.private().join("foreign")).unwrap(), b"untouched");
}

#[test]
fn cancellation_before_planning_or_consumption_needs_no_cleanup() {
    let f = Fixture::new(17);
    let sources = f.sources();
    let error = plan(&sources, &f.root, limits(), &|| true).unwrap_err();
    assert!(matches!(error.failure, Failure::Cancelled));
    assert!(error.cleanup.is_none());
    let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
    let mut io = CountCreation::default();
    let error = capture_planned_with_io(planned, &|| true, &mut io).unwrap_err();
    assert!(matches!(error.failure, Failure::Cancelled));
    assert!(error.cleanup.is_none());
    assert_eq!(io.attempts, 0);
    f.assert_clean();
}

#[test]
fn preflight_uses_existing_selection_and_limit_admission() {
    let f = Fixture::new(17);
    let sources = f.sources();
    let error = plan(
        &sources,
        &f.root,
        CaptureLimits {
            max_file_bytes: 16,
            ..limits()
        },
        &|| false,
    )
    .unwrap_err();
    assert!(matches!(error.failure, Failure::Limit(Limit::FileBytes)));
    assert!(error.cleanup.is_none());
    let duplicate = [
        Source {
            resource: "application",
            relative_path: "config",
            name: "config",
            parent: &f.parent,
            object: &f.file,
        },
        Source {
            resource: "application",
            relative_path: "config",
            name: "config",
            parent: &f.parent,
            object: &f.file,
        },
    ];
    let error = plan(&duplicate, &f.root, limits(), &|| false).unwrap_err();
    assert!(matches!(error.failure, Failure::InvalidSelection));
    assert!(error.cleanup.is_none());
    f.assert_clean();
}

#[test]
fn dropping_a_plan_never_removes_an_existing_orphan() {
    let f = Fixture::new(17);
    fs::create_dir(f.private().join("orphan")).unwrap();
    fs::write(f.private().join("orphan/foreign"), b"preserve").unwrap();
    let sources = f.sources();
    let planned = plan(&sources, &f.root, limits(), &|| false).unwrap();
    drop(planned);
    assert_eq!(
        fs::read(f.private().join("orphan/foreign")).unwrap(),
        b"preserve"
    );
    assert_eq!(fs::read_dir(f.private()).unwrap().count(), 1);
}
