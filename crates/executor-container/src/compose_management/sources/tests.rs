//! Private scratch roots/UID substitutions exercise genuine retained owners.
use super::*;
use crate::compose_management::tests::{Inputs, LocalBinding, configuration, local_files, refusal};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt};
use tempfile::TempDir;

pub(crate) const OPERATOR: &[u8] =
    include_bytes!("../../../../../tests/fixtures/compose-source/operator.json");
pub(crate) const MANAGED: &[u8] =
    include_bytes!("../../../../../tests/fixtures/compose-override/merge-delta.override.json");
pub(crate) fn inputs(
    operator: &[u8],
    managed: Option<&[u8]>,
    change: impl FnOnce(&mut Value),
) -> Result<Inputs> {
    let tree = tempfile::tempdir().unwrap();
    for name in ["config", "opt", "opt/stacks", "opt/stacks/media"] {
        let path = tree.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut config = configuration();
    config["stacks"][0]["services"] = json!(["api", "worker"]);
    config["stacks"][0]["operator"]["sha256"] = json!(hex::encode(Sha256::digest(operator)));
    config["stacks"][0]["managed"] = managed
        .map(|bytes| json!({"state":"present","sha256":hex::encode(Sha256::digest(bytes))}))
        .unwrap_or(json!({"state":"absent"}));
    change(&mut config);
    fs::write(
        tree.path().join("config/catalog.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let catalog = catalog::local_catalog(tree.path(), Path::new("/config/catalog.json"))?;
    let selected = catalog.catalog().selected("media-stack")?;
    let selection = catalog.catalog().stacks[selected].selection()?;
    fs::write(
        tree.path()
            .join("opt/stacks/media")
            .join(compose_files::base_name(selection.operator())),
        operator,
    )
    .unwrap();
    if let Some(raw) = managed {
        fs::write(
            tree.path().join("opt/stacks/media/limeos.override.json"),
            raw,
        )
        .unwrap();
    }
    let files = local_files(tree.path(), &selection)?;
    Ok(Inputs {
        tree,
        catalog,
        files,
    })
}
pub(crate) struct LocalSources {
    pub(crate) owner: BoundComposeJsonEvidence,
    pub(crate) tree: TempDir,
}
pub(crate) fn from_binding(binding: LocalBinding) -> Result<LocalSources> {
    let LocalBinding { owner, tree } = binding;
    let sources = owner.files.admit_json_local(tree.path(), &|| false)?;
    let owner = assemble_sources(owner, sources)?;
    Ok(LocalSources { owner, tree })
}
pub(crate) fn assemble_bound(
    binding: ComposeBindingEvidence,
    sources: ComposeJsonSources,
) -> Result<BoundComposeJsonEvidence> {
    assemble_sources(binding, sources)
}
pub(crate) fn counted_report(snapshot: &BoundComposeJsonSnapshot<'_>, limit: usize) -> Result<()> {
    check_source_report(snapshot, limit)
}
impl LocalSources {
    pub(crate) async fn check_binding_only(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.owner
            .binding
            .validate_with(
                cancelled,
                || {
                    self.owner
                        .binding
                        .catalog
                        .revalidate_local(self.tree.path(), cancelled)
                },
                || {
                    self.owner.binding.files.checked(|| {
                        self.owner.binding.files.validate_from(
                            &catalog::local_root(self.tree.path()),
                            compose_files::now()?,
                            Instant::now() + Duration::from_secs(2),
                            cancelled,
                        )
                    })
                },
            )
            .await
    }
    pub(crate) async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.owner
            .validate_with(
                cancelled,
                || {
                    self.owner
                        .binding
                        .catalog
                        .revalidate_local(self.tree.path(), cancelled)
                },
                || {
                    self.owner.binding.files.checked(|| {
                        self.owner.binding.files.validate_from(
                            &catalog::local_root(self.tree.path()),
                            compose_files::now()?,
                            Instant::now() + Duration::from_secs(2),
                            cancelled,
                        )
                    })
                },
                || {
                    self.owner
                        .binding
                        .files
                        .admit_json_local(self.tree.path(), cancelled)
                },
            )
            .await
    }
    pub(crate) fn expire(&mut self, remaining: Duration) {
        self.owner.binding.window.expires = Instant::now() + remaining;
    }
    pub(crate) fn set_wall_time(&mut self, at: i64) {
        self.owner.binding.window.observed_at = at;
    }
}

#[test]
fn genuine_file_bytes_are_required_even_when_pure_source_data_is_valid() {
    let input = inputs(OPERATOR, Some(MANAGED), |_| {}).unwrap();
    let source = input
        .files
        .admit_json_local(input.tree.path(), &|| false)
        .unwrap();
    assert_eq!(
        source.summary().operator_fingerprint.sha256,
        input.files.snapshot().files[0].sha256
    );
    assert_eq!(
        source.summary().managed_fingerprint.unwrap().bytes as u64,
        input.files.snapshot().files[1].identity.size
    );
    assert_eq!(
        source.summary().current.services[0].user.unwrap().gid,
        u32::MAX
    );
}

#[test]
fn unsupported_protected_source_poisoning_preserves_original_file_metadata() {
    let input = inputs(b"services: {}\n", None, |_| {}).unwrap();
    let original = serde_json::to_vec(input.files.snapshot()).unwrap();
    refusal(
        input.files.admit_json_local(input.tree.path(), &|| false),
        ErrorCode::InvalidInput,
    );
    refusal(
        input.files.admit_json_local(input.tree.path(), &|| false),
        ErrorCode::Conflict,
    );
    assert_eq!(
        serde_json::to_vec(input.files.snapshot()).unwrap(),
        original
    );
}

#[test]
fn pure_source_snapshot_cannot_substitute_different_raw_operator_or_managed_bytes() {
    let input = inputs(OPERATOR, Some(MANAGED), |_| {}).unwrap();
    let genuine = input
        .files
        .admit_json_local(input.tree.path(), &|| false)
        .unwrap();
    let mut spaced = b" ".to_vec();
    spaced.extend_from_slice(OPERATOR);
    let forged = crate::compose_source::admit_json_sources(&spaced, Some(MANAGED)).unwrap();
    refusal(
        same_sources(&genuine.summary(), &forged.summary()),
        ErrorCode::Conflict,
    );
    let forged = crate::compose_source::admit_json_sources(OPERATOR, None).unwrap();
    refusal(
        same_sources(&genuine.summary(), &forged.summary()),
        ErrorCode::Conflict,
    );
}

#[test]
fn managed_absence_change_refuses_without_turning_parsed_data_into_fresh_evidence() {
    let input = inputs(OPERATOR, None, |_| {}).unwrap();
    let source = input
        .files
        .admit_json_local(input.tree.path(), &|| false)
        .unwrap();
    fs::write(
        input
            .tree
            .path()
            .join("opt/stacks/media/limeos.override.json"),
        MANAGED,
    )
    .unwrap();
    refusal(
        input.files.admit_json_local(input.tree.path(), &|| false),
        ErrorCode::Conflict,
    );
    refusal(
        input.files.admit_json_local(input.tree.path(), &|| false),
        ErrorCode::Conflict,
    );
    assert!(source.summary().managed_fingerprint.is_none());
}
