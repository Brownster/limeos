use super::*;
use serde_json::{Value, json};

fn fixture(name: &str) -> &'static [u8] {
    match name {
        "oci-arm64-config.json" => {
            include_bytes!("../../../../../tests/fixtures/deployment/images/oci-arm64-config.json")
        }
        "oci-amd64-config.json" => {
            include_bytes!("../../../../../tests/fixtures/deployment/images/oci-amd64-config.json")
        }
        "oci-arm64-manifest.json" => include_bytes!(
            "../../../../../tests/fixtures/deployment/images/oci-arm64-manifest.json"
        ),
        "oci-amd64-manifest.json" => include_bytes!(
            "../../../../../tests/fixtures/deployment/images/oci-amd64-manifest.json"
        ),
        "oci-index.json" => {
            include_bytes!("../../../../../tests/fixtures/deployment/images/oci-index.json")
        }
        "docker-arm64-config.json" => include_bytes!(
            "../../../../../tests/fixtures/deployment/images/docker-arm64-config.json"
        ),
        "docker-amd64-config.json" => include_bytes!(
            "../../../../../tests/fixtures/deployment/images/docker-amd64-config.json"
        ),
        "docker-arm64-manifest.json" => include_bytes!(
            "../../../../../tests/fixtures/deployment/images/docker-arm64-manifest.json"
        ),
        "docker-amd64-manifest.json" => include_bytes!(
            "../../../../../tests/fixtures/deployment/images/docker-amd64-manifest.json"
        ),
        "docker-index.json" => {
            include_bytes!("../../../../../tests/fixtures/deployment/images/docker-index.json")
        }
        _ => panic!("unknown test fixture"),
    }
}
fn hashes() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../../../../tests/fixtures/deployment/images/hashes.json"
    ))
    .unwrap()
}
struct Chain {
    root: Vec<u8>,
    manifest: Option<Vec<u8>>,
    config: Vec<u8>,
    expected: String,
    platform: Platform,
}
impl Chain {
    fn fixture(family: &str, index: bool, architecture: &str) -> Self {
        let root_name = if index {
            format!("{family}-index.json")
        } else {
            format!("{family}-{architecture}-manifest.json")
        };
        Self {
            root: fixture(&root_name).to_vec(),
            manifest: index
                .then(|| fixture(&format!("{family}-{architecture}-manifest.json")).to_vec()),
            config: fixture(&format!("{family}-{architecture}-config.json")).to_vec(),
            expected: hashes()[root_name]["digest"].as_str().unwrap().into(),
            platform: if architecture == "arm64" {
                Platform::LinuxArm64V8
            } else {
                Platform::LinuxAmd64
            },
        }
    }
    fn admit(&self) -> Result<AdmittedImage> {
        admit_image_content(ImageContent {
            expected_root_digest: &self.expected,
            root: &self.root,
            manifest: self.manifest.as_deref(),
            config: &self.config,
            platform: self.platform,
        })
    }
    fn root(&mut self, change: impl FnOnce(&mut Value)) {
        let mut root: Value = serde_json::from_slice(&self.root).unwrap();
        change(&mut root);
        self.root = serde_json::to_vec(&root).unwrap();
        self.expected = hash(&self.root);
    }
    fn manifest(&mut self, change: impl FnOnce(&mut Value)) {
        if let Some(bytes) = &mut self.manifest {
            let mut value: Value = serde_json::from_slice(bytes).unwrap();
            change(&mut value);
            *bytes = serde_json::to_vec(&value).unwrap();
            let digest = hash(bytes);
            let size = bytes.len();
            let i = if self.platform == Platform::LinuxArm64V8 {
                0
            } else {
                1
            };
            self.root(|root| {
                root["manifests"][i]["digest"] = json!(digest);
                root["manifests"][i]["size"] = json!(size);
            });
        } else {
            self.root(change);
        }
    }
    fn config_raw(&mut self, bytes: Vec<u8>) {
        self.config = bytes;
        let digest = hash(&self.config);
        let size = self.config.len();
        self.manifest(|m| {
            m["config"]["digest"] = json!(digest);
            m["config"]["size"] = json!(size);
        });
    }
    fn config(&mut self, change: impl FnOnce(&mut Value)) {
        let mut config: Value = serde_json::from_slice(&self.config).unwrap();
        change(&mut config);
        self.config_raw(serde_json::to_vec(&config).unwrap());
    }
}

#[test]
fn independent_fixture_hashes_and_sizes_match_exact_raw_bytes() {
    for (name, metadata) in hashes().as_object().unwrap() {
        assert_eq!(hash(fixture(name)), metadata["digest"].as_str().unwrap());
        assert_eq!(
            fixture(name).len() as u64,
            metadata["size"].as_u64().unwrap()
        );
    }
}
#[test]
fn direct_oci_and_docker_manifests_admit_both_supported_platforms() {
    for family in ["oci", "docker"] {
        for architecture in ["arm64", "amd64"] {
            let chain = Chain::fixture(family, false, architecture);
            let admitted = chain.admit().unwrap();
            let summary = admitted.summary();
            assert_eq!(summary.root_kind, RootKind::Manifest);
            assert_eq!(summary.root_digest, summary.manifest_digest);
            assert_eq!(
                summary.config_digest,
                hashes()[format!("{family}-{architecture}-config.json")]["digest"]
                    .as_str()
                    .unwrap()
            );
            assert_eq!(summary.layer_count, 2);
            assert_eq!(summary.declared_layer_bytes, 3072);
        }
    }
}
#[test]
fn multi_platform_indexes_bind_child_and_config_instead_of_using_index_as_image_id() {
    for family in ["oci", "docker"] {
        for architecture in ["arm64", "amd64"] {
            let chain = Chain::fixture(family, true, architecture);
            let admitted = chain.admit().unwrap();
            let summary = admitted.summary();
            assert_eq!(summary.root_kind, RootKind::Index);
            assert_ne!(summary.root_digest, summary.manifest_digest);
            assert_ne!(summary.manifest_digest, summary.config_digest);
            assert_eq!(
                summary.manifest_digest,
                hashes()[format!("{family}-{architecture}-manifest.json")]["digest"]
                    .as_str()
                    .unwrap()
            );
        }
    }
}
#[test]
fn incorrect_expected_root_and_child_hashes_refuse() {
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.expected = "sha256:".to_owned() + &"0".repeat(64);
    assert_eq!(chain.admit().unwrap_err(), ContentError::DigestMismatch);
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.manifest.as_mut().unwrap()[0] = b' ';
    assert_eq!(chain.admit().unwrap_err(), ContentError::DigestMismatch);
}
#[test]
fn config_hashes_cover_original_bytes_without_normalizing_json() {
    let mut chain = Chain::fixture("oci", false, "arm64");
    let offset = chain.config.iter().position(|b| *b == b'v').unwrap();
    chain.config[offset] = b'V';
    assert_eq!(chain.admit().unwrap_err(), ContentError::DigestMismatch);
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.config.push(b' ');
    assert_eq!(chain.admit().unwrap_err(), ContentError::SizeMismatch);
}
#[test]
fn root_child_and_config_sizes_and_types_must_agree() {
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.root(|r| r["manifests"][0]["size"] = json!(1));
    assert_eq!(chain.admit().unwrap_err(), ContentError::SizeMismatch);
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.manifest(|m| m["config"]["size"] = json!(1));
    assert_eq!(chain.admit().unwrap_err(), ContentError::SizeMismatch);
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.root(|r| r["manifests"][0]["mediaType"] = json!(DOCKER_MANIFEST));
    assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.manifest(|m| m["config"]["mediaType"] = json!(DOCKER_CONFIG));
    assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
}
#[test]
fn wrong_or_missing_platform_never_selects_the_first_index_entry() {
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.platform = Platform::LinuxAmd64;
    assert_eq!(chain.admit().unwrap_err(), ContentError::PlatformMismatch);
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.root(|r| {
        r["manifests"].as_array_mut().unwrap().remove(0);
    });
    assert_eq!(chain.admit().unwrap_err(), ContentError::MissingPlatform);
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.config(|c| c["architecture"] = json!("amd64"));
    assert_eq!(chain.admit().unwrap_err(), ContentError::PlatformMismatch);
}
#[test]
fn duplicate_matching_platforms_are_ambiguous_even_with_equal_hashes() {
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.root(|r| {
        let duplicate = r["manifests"][0].clone();
        r["manifests"].as_array_mut().unwrap().push(duplicate);
    });
    assert_eq!(chain.admit().unwrap_err(), ContentError::AmbiguousPlatform);
}
#[test]
fn omitted_or_explicit_base_variants_are_supported_but_higher_variants_refuse() {
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.config(|c| {
        c.as_object_mut().unwrap().remove("variant");
    });
    chain.root(|r| {
        r["manifests"][0]["platform"]
            .as_object_mut()
            .unwrap()
            .remove("variant");
    });
    assert!(chain.admit().is_ok());
    let mut chain = Chain::fixture("oci", false, "amd64");
    chain.config(|c| c["variant"] = json!("v1"));
    assert!(chain.admit().is_ok());
    for (architecture, variant) in [("arm64", "v8.1"), ("amd64", "v2")] {
        let mut chain = Chain::fixture("oci", false, architecture);
        chain.config(|c| c["variant"] = json!(variant));
        assert_eq!(chain.admit().unwrap_err(), ContentError::PlatformMismatch);
    }
}
#[test]
fn unsupported_os_features_and_versions_cannot_be_discarded() {
    for field in ["os.version", "os.features", "features"] {
        let mut chain = Chain::fixture("oci", false, "arm64");
        chain.config(|c| c[field] = json!([]));
        assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
        let mut chain = Chain::fixture("oci", true, "arm64");
        chain.root(|r| r["manifests"][0]["platform"][field] = json!([]));
        assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
    }
}
#[test]
fn consumed_duplicate_header_descriptor_and_config_fields_are_malformed() {
    let mut chain = Chain::fixture("oci", false, "arm64");
    let text = String::from_utf8(chain.root).unwrap();
    chain.root = text.replacen('{', "{\"schemaVersion\":2,", 1).into_bytes();
    chain.expected = hash(&chain.root);
    assert_eq!(chain.admit().unwrap_err(), ContentError::Malformed);
    let mut chain = Chain::fixture("oci", false, "arm64");
    let text = String::from_utf8(chain.config.clone()).unwrap();
    chain.config_raw(
        text.replacen('{', "{\"architecture\":\"arm64\",", 1)
            .into_bytes(),
    );
    assert_eq!(chain.admit().unwrap_err(), ContentError::Malformed);
    let mut chain = Chain::fixture("oci", false, "arm64");
    let text = String::from_utf8(chain.root).unwrap();
    chain.root = text
        .replacen("\"config\":{", "\"config\":{\"size\":1,", 1)
        .into_bytes();
    chain.expected = hash(&chain.root);
    assert_eq!(chain.admit().unwrap_err(), ContentError::Malformed);
}
#[test]
fn artifacts_nested_indexes_and_unknown_matching_types_are_unsupported() {
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.root(|r| r["manifests"][0]["mediaType"] = json!(OCI_INDEX));
    assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.root(|r| r["manifests"][0]["mediaType"] = json!("application/example"));
    assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
    for field in ["artifactType", "subject"] {
        let mut chain = Chain::fixture("oci", false, "arm64");
        chain.manifest(|m| m[field] = json!("unsupported"));
        assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
    }
}
#[test]
fn selected_external_embedded_or_artifact_descriptors_are_refused() {
    for field in ["urls", "data", "artifactType"] {
        let mut chain = Chain::fixture("oci", true, "arm64");
        chain.root(|r| r["manifests"][0][field] = json!("untrusted"));
        assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
        let mut chain = Chain::fixture("oci", false, "arm64");
        chain.manifest(|m| m["config"][field] = json!([]));
        assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
        let mut chain = Chain::fixture("oci", false, "arm64");
        chain.manifest(|m| m["layers"][0][field] = json!([]));
        assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
    }
}
#[test]
fn unsupported_layers_rootfs_types_and_diffid_mismatches_refuse() {
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.manifest(|m| {
        m["layers"][0]["mediaType"] =
            json!("application/vnd.docker.image.rootfs.foreign.diff.tar.gzip")
    });
    assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.config(|c| c["rootfs"]["type"] = json!("other"));
    assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.config(|c| {
        c["rootfs"]["diff_ids"].as_array_mut().unwrap().pop();
    });
    assert_eq!(chain.admit().unwrap_err(), ContentError::Malformed);
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.config(|c| c["rootfs"]["diff_ids"][0] = json!("sha512:unknown"));
    assert_eq!(chain.admit().unwrap_err(), ContentError::Malformed);
}
#[test]
fn layer_descriptor_and_uncompressed_diffid_hashes_are_distinct() {
    let chain = Chain::fixture("oci", false, "arm64");
    let manifest: Value = serde_json::from_slice(&chain.root).unwrap();
    let config: Value = serde_json::from_slice(&chain.config).unwrap();
    assert_ne!(
        manifest["layers"][0]["digest"],
        config["rootfs"]["diff_ids"][0]
    );
    assert!(chain.admit().is_ok());
}
#[test]
fn raw_blob_and_aggregate_bounds_precede_hashing_or_parsing() {
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.root = vec![b' '; MAX_DOCUMENT_BYTES + 1];
    assert_eq!(chain.admit().unwrap_err(), ContentError::Limit);
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.root.resize(23000, b' ');
    chain.manifest.as_mut().unwrap().resize(23000, b' ');
    chain.config.resize(23000, b' ');
    assert_eq!(chain.admit().unwrap_err(), ContentError::Limit);
}
#[test]
fn index_layer_and_diffid_sequence_caps_are_enforced_during_decode() {
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.root(|r| {
        let d = r["manifests"][1].clone();
        r["manifests"] = Value::Array(vec![d; MAX_INDEX_ENTRIES + 1]);
    });
    assert_eq!(chain.admit().unwrap_err(), ContentError::Malformed);
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.manifest(|m| {
        let d = m["layers"][0].clone();
        m["layers"] = Value::Array(vec![d; MAX_LAYERS + 1]);
    });
    assert_eq!(chain.admit().unwrap_err(), ContentError::Malformed);
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.config(|c| {
        c["rootfs"]["diff_ids"] =
            json!(vec!["sha256:".to_owned() + &"e".repeat(64); MAX_LAYERS + 1])
    });
    assert_eq!(chain.admit().unwrap_err(), ContentError::Malformed);
}
#[test]
fn declared_size_sum_cannot_overflow() {
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.manifest(|m| {
        m["layers"][0]["size"] = json!(u64::MAX);
        m["layers"][1]["size"] = json!(1);
    });
    assert_eq!(chain.admit().unwrap_err(), ContentError::Limit);
}
#[test]
fn direct_roots_reject_unbound_child_bytes_and_indexes_require_the_selected_child() {
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.manifest = Some(chain.root.clone());
    assert_eq!(chain.admit().unwrap_err(), ContentError::UnexpectedData);
    let mut chain = Chain::fixture("oci", true, "arm64");
    chain.manifest = None;
    assert_eq!(chain.admit().unwrap_err(), ContentError::UnexpectedData);
}
#[test]
fn malformed_trailing_non_utf8_and_unsupported_schema_content_refuse_without_payload_errors() {
    for bytes in [b"not JSON".to_vec(), vec![255, b'\n'], b"{} {}".to_vec()] {
        let mut chain = Chain::fixture("oci", false, "arm64");
        chain.root = bytes;
        chain.expected = hash(&chain.root);
        assert_eq!(chain.admit().unwrap_err(), ContentError::Malformed);
    }
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.root(|r| r["schemaVersion"] = json!(1));
    assert_eq!(chain.admit().unwrap_err(), ContentError::Unsupported);
}
#[test]
fn ignored_runtime_values_never_enter_summaries_debug_or_errors() {
    let admitted = Chain::fixture("oci", true, "arm64").admit().unwrap();
    let summary = serde_json::to_string(&admitted.summary()).unwrap();
    assert!(!summary.contains("IMAGE_PRIVATE_VALUE"));
    assert!(!summary.contains("never-log-fixture"));
    assert!(!format!("{admitted:?}").contains("never-log-fixture"));
    assert!(summary.len() < 2048);
    assert_eq!(admitted.summary().unresolved.len(), 4);
}

#[test]
fn admitted_index_facts_feed_v2_policy_without_becoming_authority() {
    use limeos_domain::{
        ContainerSnapshot,
        deployment::{container_updates::*, discovery::*},
    };
    let admitted = Chain::fixture("oci", true, "arm64").admit().unwrap();
    let content = admitted.summary();
    let project = ProjectIdentity {
        name: "media".into(),
        working_directory: "/opt/stacks/Media".into(),
        config_files: vec!["/opt/stacks/Media/compose.yaml".into()],
    };
    let source = ImageSource::Digest {
        repository: "example/web".into(),
        root_digest: content.root_digest.into(),
    };
    let specs = [StackUpdateSpec {
        stack: "media".into(),
        project: project.clone(),
        services: vec![ServiceTrack {
            service: "web".into(),
            source: source.clone(),
            platform: content.platform,
            replicas: 1,
        }],
    }];
    let images = [ImageResolution {
        engine_id: "engine".into(),
        stack: "media".into(),
        project: project.clone(),
        service: "web".into(),
        source,
        platform: content.platform,
        root_digest: content.root_digest.into(),
        manifest_digest: content.manifest_digest.into(),
        config_digest: content.config_digest.into(),
        sampled_at: 100,
    }];
    let stacks = [Declaration {
        stack: "media".into(),
        project: project.name.clone(),
        working_directory: project.working_directory.clone(),
        config_files: project.config_files.clone(),
        services: vec!["web".into()],
        files: FileStatus::NotInspected,
    }];
    let members = [Membership::Compose {
        container: ContainerSnapshot {
            resource: format!("container:{}", "1".repeat(64)),
            image: format!("sha256:{}", "0".repeat(64)),
            started_at: "2026-10-15T00:00:00Z".into(),
            running: true,
        },
        project,
        service: "web".into(),
        one_off: false,
    }];
    let config = UpdateConfig::default();
    let selected = ["media".into()];
    let plan = plan_updates(UpdateInput {
        config: Some(&config),
        mode: RunMode::Manual,
        selected: &selected,
        platform: content.platform,
        specs: &specs,
        catalog: Some(CatalogInput {
            engine_id: "engine",
            sampled_at: 100,
            stacks: &stacks,
        }),
        engine: Some(EngineInput {
            engine_id: "engine",
            sampled_at: 100,
            members: &members,
        }),
        images: &images,
        now: 101,
    })
    .unwrap();
    assert_eq!(plan.summary().version, 2);
    assert_eq!(plan.summary().rows[0].decision, Decision::UpdateProposed);
    assert_eq!(
        plan.summary().rows[0].services[0].root_digest,
        content.root_digest
    );
    assert_eq!(plan.summary().unresolved.len(), 8);
    assert_eq!(content.unresolved.len(), 4);
}

#[test]
fn uncompressed_layer_descriptors_and_diffids_must_name_the_same_content() {
    let mut chain = Chain::fixture("oci", false, "arm64");
    chain.manifest(|m| {
        m["layers"][0]["mediaType"] = json!("application/vnd.oci.image.layer.v1.tar")
    });
    assert_eq!(chain.admit().unwrap_err(), ContentError::Malformed);
    let diff_id: Value =
        serde_json::from_slice::<Value>(&chain.config).unwrap()["rootfs"]["diff_ids"][0].clone();
    chain.manifest(|m| m["layers"][0]["digest"] = diff_id);
    assert!(chain.admit().is_ok());
}
