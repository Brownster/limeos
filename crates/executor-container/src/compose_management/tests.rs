//! Private local root/UID substitutions; production has no such selectors.
use super::*;
use crate::compose_files::{Budget, ComposeFileSelection};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, sync::atomic::AtomicU64};
use tempfile::TempDir;
pub(crate) const OPERATOR: &[u8] =
    include_bytes!("../../../../tests/fixtures/compose-files/operator.yaml");
pub(crate) const MANAGED: &[u8] =
    include_bytes!("../../../../tests/fixtures/compose-files/limeos.override.json");
pub(crate) fn configuration() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/compose-management/catalog.json"
    ))
    .unwrap()
}
pub(crate) fn refusal<T>(result: Result<T>, code: ErrorCode) {
    match result {
        Err(error) => assert_eq!(error.0, code),
        Ok(_) => panic!("expected {code:?}"),
    }
}
pub(crate) struct Inputs {
    pub tree: TempDir,
    pub catalog: ManagementCatalogEvidence,
    pub files: ComposeFileEvidence,
}
pub(crate) fn inputs_with(value: Value) -> Result<Inputs> {
    let tree = tempfile::tempdir().unwrap();
    for name in ["config", "opt", "opt/stacks", "opt/stacks/media"] {
        let path = tree.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::write(
        tree.path().join("config/catalog.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let catalog = catalog::local_catalog(tree.path(), Path::new("/config/catalog.json"))?;
    let selected = catalog.catalog().selected("media-stack")?;
    let selection = catalog.catalog().stacks[selected].selection()?;
    fs::write(
        tree.path()
            .join("opt/stacks/media")
            .join(compose_files::base_name(selection.operator())),
        OPERATOR,
    )
    .unwrap();
    if selection.includes_managed() {
        fs::write(
            tree.path().join("opt/stacks/media/limeos.override.json"),
            MANAGED,
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
pub(crate) fn local_files(
    root: &Path,
    selection: &ComposeFileSelection,
) -> Result<ComposeFileEvidence> {
    let files = compose_files::collect(
        catalog::local_root(root),
        selection,
        rustix::process::geteuid().as_raw(),
        compose_files::now()?,
        Budget {
            spent: AtomicU64::new(0),
            expires: Instant::now() + LIFETIME,
        },
        Instant::now() + Duration::from_secs(2),
        &|| false,
    )?;
    validate_files(&files, root, &|| false)?;
    Ok(files)
}
fn validate_files(
    files: &ComposeFileEvidence,
    root: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    files.checked(|| {
        files.validate_from(
            &catalog::local_root(root),
            compose_files::now()?,
            Instant::now() + Duration::from_secs(2),
            cancelled,
        )
    })
}
pub(crate) struct LocalBinding {
    pub owner: ComposeBindingEvidence,
    pub tree: TempDir,
}
pub(crate) fn bind(inputs: Inputs, engine: EngineComposeEvidence) -> Result<LocalBinding> {
    let Inputs {
        tree,
        catalog,
        files,
    } = inputs;
    let selected = catalog.catalog().selected("media-stack")?;
    let owner = assemble(catalog, selected, files, engine, Window::new()?)?;
    Ok(LocalBinding { owner, tree })
}
impl LocalBinding {
    pub(crate) async fn revalidate(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.owner
            .validate_with(
                cancelled,
                || {
                    self.owner
                        .catalog
                        .revalidate_local(self.tree.path(), cancelled)
                },
                || validate_files(&self.owner.files, self.tree.path(), cancelled),
            )
            .await
    }
    pub(crate) fn expire(&mut self, remaining: Duration) {
        self.owner.window.expires = Instant::now() + remaining;
    }
    pub(crate) fn set_wall_time(&mut self, observed_at: i64) {
        self.owner.window.observed_at = observed_at;
    }
}

#[test]
fn catalog_selects_literal_identity_and_preserves_meaningful_order() {
    let input = inputs_with(configuration()).unwrap();
    let c = input.catalog.catalog();
    c.validate().unwrap();
    assert_eq!(c.selected("media-stack").unwrap(), 0);
    refusal(c.selected("missing"), ErrorCode::NotFound);
    refusal(c.selected("../media-stack"), ErrorCode::InvalidInput);
    assert_eq!(c.stacks[0].services, ["media", "transcoder"]);
    assert!(c.stacks[0].selection().unwrap().includes_managed());
    assert_eq!(
        input.catalog.snapshot().identity.size,
        serde_json::to_vec(&configuration()).unwrap().len() as u64
    );
}
#[test]
fn catalog_configuration_has_no_p03_approval_or_secret_fields() {
    let input = inputs_with(configuration()).unwrap();
    let report = serde_json::to_string(input.catalog.snapshot()).unwrap();
    for field in [
        "services",
        "engine_id",
        "approval",
        "template",
        "SYNTHETIC_SECRET",
        "principal",
    ] {
        assert!(!report.contains(field));
    }
    assert!(report.contains("sha256"));
    let p03: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/compose-catalog.json"
    ))
    .unwrap();
    assert!(serde_json::from_value::<catalog::ManagementCatalog>(p03).is_err());
}
#[test]
fn component_overlap_is_literal_and_symmetric() {
    for (a, b, expected) in [
        ("/a", "/a", true),
        ("/a", "/a/b", true),
        ("/a/b", "/a", true),
        ("/a", "/ab", false),
        ("/A", "/a", false),
        ("/a b", "/a b/c", true),
    ] {
        assert_eq!(catalog::overlap(a, b), expected);
        assert_eq!(catalog::overlap(b, a), expected);
    }
}
#[test]
fn names_only_selection_does_not_accept_unregistered_overrides() {
    let mut value = configuration();
    value["stacks"][0]["operator"]["name"] = json!("override.yaml");
    refusal(inputs_with(value), ErrorCode::InvalidInput);
    let mut value = configuration();
    value["stacks"][0]["approval"] = json!("fake-token");
    refusal(inputs_with(value), ErrorCode::InvalidInput);
}
