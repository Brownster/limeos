//! Bounded raw-byte consistency checks. No network, protected owner or authority.
use limeos_domain::{deployment::container_updates::Platform, opaque_id};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{IgnoredAny, SeqAccess, Visitor},
};
use sha2::{Digest, Sha256};
use std::{fmt, marker::PhantomData};

pub const MAX_DOCUMENT_BYTES: usize = 32 * 1024;
pub const MAX_TOTAL_BYTES: usize = 64 * 1024;
pub const MAX_INDEX_ENTRIES: usize = 16;
pub const MAX_LAYERS: usize = 64;
const OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const OCI_CONFIG: &str = "application/vnd.oci.image.config.v1+json";
const DOCKER_INDEX: &str = "application/vnd.docker.distribution.manifest.list.v2+json";
const DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";
const DOCKER_CONFIG: &str = "application/vnd.docker.container.image.v1+json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentError {
    Limit,
    Malformed,
    Unsupported,
    DigestMismatch,
    SizeMismatch,
    PlatformMismatch,
    AmbiguousPlatform,
    MissingPlatform,
    UnexpectedData,
}
type Result<T> = std::result::Result<T, ContentError>;

/// Expected digest and bytes are untrusted data; equality does not establish origin.
pub struct ImageContent<'a> {
    pub expected_root_digest: &'a str,
    pub root: &'a [u8],
    pub manifest: Option<&'a [u8]>,
    pub config: &'a [u8],
    pub platform: Platform,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RootKind {
    Manifest,
    Index,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentGate {
    SourceOriginAndTagBinding,
    CompleteLayersAndRuntimeAcceptance,
    ProtectedCollectionAndFreshness,
    ExistingAuthorityClaimsAndRecovery,
}
const GATES: &[ContentGate] = &[
    ContentGate::SourceOriginAndTagBinding,
    ContentGate::CompleteLayersAndRuntimeAcceptance,
    ContentGate::ProtectedCollectionAndFreshness,
    ContentGate::ExistingAuthorityClaimsAndRecovery,
];
/// Cannot be deserialized or constructed from a report. Retains no payload bytes.
pub struct AdmittedImage {
    root_kind: RootKind,
    root_digest: String,
    manifest_digest: String,
    config_digest: String,
    platform: Platform,
    layer_count: u16,
    declared_layer_bytes: u64,
    root_bytes: usize,
    manifest_bytes: usize,
    config_bytes: usize,
}
impl fmt::Debug for AdmittedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmittedImage")
            .field("root_kind", &self.root_kind)
            .field("platform", &self.platform)
            .field("layer_count", &self.layer_count)
            .finish_non_exhaustive()
    }
}
#[derive(Serialize)]
pub struct ContentSummary<'a> {
    pub version: u16,
    pub root_kind: RootKind,
    pub root_digest: &'a str,
    pub manifest_digest: &'a str,
    pub config_digest: &'a str,
    pub platform: Platform,
    pub layer_count: u16,
    /// Declared compressed/uncompressed descriptor sizes, not measured disk usage.
    pub declared_layer_bytes: u64,
    pub root_bytes: usize,
    pub manifest_bytes: usize,
    pub config_bytes: usize,
    pub unresolved: &'static [ContentGate],
}
impl AdmittedImage {
    pub fn summary(&self) -> ContentSummary<'_> {
        ContentSummary {
            version: 1,
            root_kind: self.root_kind,
            root_digest: &self.root_digest,
            manifest_digest: &self.manifest_digest,
            config_digest: &self.config_digest,
            platform: self.platform,
            layer_count: self.layer_count,
            declared_layer_bytes: self.declared_layer_bytes,
            root_bytes: self.root_bytes,
            manifest_bytes: self.manifest_bytes,
            config_bytes: self.config_bytes,
            unresolved: GATES,
        }
    }
}
struct BoundedVec<T, const N: usize>(Vec<T>);
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for BoundedVec<T, N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct BoundedVisitor<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for BoundedVisitor<T, N> {
            type Value = BoundedVec<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("bounded image sequence")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                if seq.size_hint().is_some_and(|n| n > N) {
                    return Err(serde::de::Error::custom("image sequence bound"));
                }
                let mut values = Vec::new();
                while values.len() < N {
                    match seq.next_element()? {
                        Some(value) => values.push(value),
                        None => return Ok(BoundedVec(values)),
                    }
                }
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("image sequence bound"));
                }
                Ok(BoundedVec(values))
            }
        }
        deserializer.deserialize_seq(BoundedVisitor::<T, N>(PhantomData))
    }
}
#[derive(Deserialize)]
struct Header {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "mediaType")]
    media_type: String,
}
#[derive(Deserialize)]
struct PlatformClaim {
    os: String,
    architecture: String,
    variant: Option<String>,
    #[serde(rename = "os.version")]
    os_version: Option<IgnoredAny>,
    #[serde(rename = "os.features")]
    os_features: Option<IgnoredAny>,
    features: Option<IgnoredAny>,
}
#[derive(Deserialize)]
struct Descriptor {
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
    size: u64,
    urls: Option<IgnoredAny>,
    data: Option<IgnoredAny>,
    #[serde(rename = "artifactType")]
    artifact_type: Option<IgnoredAny>,
    platform: Option<PlatformClaim>,
}
#[derive(Deserialize)]
struct Index {
    manifests: BoundedVec<Descriptor, MAX_INDEX_ENTRIES>,
    #[serde(rename = "artifactType")]
    artifact_type: Option<IgnoredAny>,
    subject: Option<IgnoredAny>,
    config: Option<IgnoredAny>,
    layers: Option<IgnoredAny>,
}
#[derive(Deserialize)]
struct Manifest {
    config: Descriptor,
    layers: BoundedVec<Descriptor, MAX_LAYERS>,
    #[serde(rename = "artifactType")]
    artifact_type: Option<IgnoredAny>,
    subject: Option<IgnoredAny>,
    manifests: Option<IgnoredAny>,
}
#[derive(Deserialize)]
struct RootFs {
    #[serde(rename = "type")]
    kind: String,
    diff_ids: BoundedVec<String, MAX_LAYERS>,
}
#[derive(Deserialize)]
struct Config {
    os: String,
    architecture: String,
    variant: Option<String>,
    #[serde(rename = "os.version")]
    os_version: Option<IgnoredAny>,
    #[serde(rename = "os.features")]
    os_features: Option<IgnoredAny>,
    features: Option<IgnoredAny>,
    rootfs: RootFs,
    // Do not retain environment, credentials, commands, history or annotations.
    #[serde(rename = "config")]
    _runtime: Option<IgnoredAny>,
}
fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| ContentError::Malformed)
}
fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(opaque_id)
}
fn hash(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}
fn manifest_type(media_type: &str) -> bool {
    matches!(media_type, OCI_MANIFEST | DOCKER_MANIFEST)
}
fn index_type(media_type: &str) -> bool {
    matches!(media_type, OCI_INDEX | DOCKER_INDEX)
}
fn header(bytes: &[u8]) -> Result<Header> {
    let header: Header = parse(bytes)?;
    if header.schema_version != 2 {
        return Err(ContentError::Unsupported);
    }
    if header.media_type.len() > 128 {
        return Err(ContentError::Limit);
    }
    Ok(header)
}
fn descriptor(descriptor: &Descriptor, selected: bool) -> Result<()> {
    if descriptor.media_type.is_empty()
        || descriptor.media_type.len() > 128
        || !digest(&descriptor.digest)
    {
        return Err(ContentError::Malformed);
    }
    if selected
        && (descriptor.urls.is_some()
            || descriptor.data.is_some()
            || descriptor.artifact_type.is_some())
    {
        return Err(ContentError::Unsupported);
    }
    Ok(())
}
fn matches_platform(
    os: &str,
    architecture: &str,
    variant: Option<&str>,
    qualified: bool,
    expected: Platform,
) -> Result<bool> {
    if os.len() > 64 || architecture.len() > 64 || variant.is_some_and(|v| v.len() > 64) {
        return Err(ContentError::Limit);
    }
    let supported = os == "linux"
        && match expected {
            Platform::LinuxAmd64 => architecture == "amd64" && matches!(variant, None | Some("v1")),
            Platform::LinuxArm64V8 => {
                architecture == "arm64" && matches!(variant, None | Some("v8"))
            }
        };
    if supported && qualified {
        return Err(ContentError::Unsupported);
    }
    Ok(supported)
}
fn platform(claim: &PlatformClaim, expected: Platform) -> Result<bool> {
    matches_platform(
        &claim.os,
        &claim.architecture,
        claim.variant.as_deref(),
        claim.os_version.is_some() || claim.os_features.is_some() || claim.features.is_some(),
        expected,
    )
}
fn matches_blob(descriptor: &Descriptor, bytes: &[u8]) -> Result<String> {
    if descriptor.size != bytes.len() as u64 {
        return Err(ContentError::SizeMismatch);
    }
    let actual = hash(bytes);
    if actual != descriptor.digest {
        return Err(ContentError::DigestMismatch);
    }
    Ok(actual)
}
fn preflight(input: &ImageContent<'_>) -> Result<()> {
    if !digest(input.expected_root_digest) {
        return Err(ContentError::Malformed);
    }
    let mut total = 0usize;
    for bytes in [Some(input.root), input.manifest, Some(input.config)]
        .into_iter()
        .flatten()
    {
        if bytes.is_empty() {
            return Err(ContentError::Malformed);
        }
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(ContentError::Limit);
        }
        total = total.checked_add(bytes.len()).ok_or(ContentError::Limit)?;
    }
    if total > MAX_TOTAL_BYTES {
        return Err(ContentError::Limit);
    }
    Ok(())
}
/// Verify raw-byte references and the supported platform subset only.
pub fn admit_image_content(input: ImageContent<'_>) -> Result<AdmittedImage> {
    preflight(&input)?;
    let root_digest = hash(input.root);
    if root_digest != input.expected_root_digest {
        return Err(ContentError::DigestMismatch);
    }
    let root_header = header(input.root)?;
    let (root_kind, manifest_bytes, manifest_digest, manifest_header) =
        if manifest_type(&root_header.media_type) {
            if input.manifest.is_some() {
                return Err(ContentError::UnexpectedData);
            }
            (
                RootKind::Manifest,
                input.root,
                root_digest.clone(),
                root_header,
            )
        } else if index_type(&root_header.media_type) {
            let index: Index = parse(input.root)?;
            if index.artifact_type.is_some() || index.subject.is_some() {
                return Err(ContentError::Unsupported);
            }
            if index.config.is_some() || index.layers.is_some() {
                return Err(ContentError::Malformed);
            }
            let mut selected = None;
            for entry in &index.manifests.0 {
                descriptor(entry, false)?;
                if index_type(&entry.media_type) {
                    return Err(ContentError::Unsupported);
                }
                if let Some(claim) = &entry.platform
                    && platform(claim, input.platform)?
                {
                    if selected.is_some() {
                        return Err(ContentError::AmbiguousPlatform);
                    }
                    descriptor(entry, true)?;
                    if !manifest_type(&entry.media_type) {
                        return Err(ContentError::Unsupported);
                    }
                    selected = Some(entry);
                }
            }
            let selected = selected.ok_or(ContentError::MissingPlatform)?;
            let bytes = input.manifest.ok_or(ContentError::UnexpectedData)?;
            let actual = matches_blob(selected, bytes)?;
            let child_header = header(bytes)?;
            if child_header.media_type != selected.media_type {
                return Err(ContentError::Unsupported);
            }
            (RootKind::Index, bytes, actual, child_header)
        } else {
            return Err(ContentError::Unsupported);
        };
    let manifest: Manifest = parse(manifest_bytes)?;
    if manifest.artifact_type.is_some() || manifest.subject.is_some() {
        return Err(ContentError::Unsupported);
    }
    if manifest.manifests.is_some() {
        return Err(ContentError::Malformed);
    }
    descriptor(&manifest.config, true)?;
    if manifest.config.platform.is_some() {
        return Err(ContentError::Unsupported);
    }
    let expected_config = if manifest_header.media_type == OCI_MANIFEST {
        OCI_CONFIG
    } else {
        DOCKER_CONFIG
    };
    if manifest.config.media_type != expected_config {
        return Err(ContentError::Unsupported);
    }
    let config_digest = matches_blob(&manifest.config, input.config)?;
    let config: Config = parse(input.config)?;
    if !matches_platform(
        &config.os,
        &config.architecture,
        config.variant.as_deref(),
        config.os_version.is_some() || config.os_features.is_some() || config.features.is_some(),
        input.platform,
    )? {
        return Err(ContentError::PlatformMismatch);
    }
    if config.rootfs.kind != "layers" {
        return Err(ContentError::Unsupported);
    }
    if config.rootfs.diff_ids.0.len() != manifest.layers.0.len()
        || config.rootfs.diff_ids.0.iter().any(|id| !digest(id))
    {
        return Err(ContentError::Malformed);
    }
    let mut declared_layer_bytes = 0u64;
    for (layer, diff_id) in manifest.layers.0.iter().zip(&config.rootfs.diff_ids.0) {
        descriptor(layer, true)?;
        if layer.platform.is_some()
            || !matches!(
                layer.media_type.as_str(),
                "application/vnd.oci.image.layer.v1.tar"
                    | "application/vnd.oci.image.layer.v1.tar+gzip"
                    | "application/vnd.oci.image.layer.v1.tar+zstd"
                    | "application/vnd.docker.image.rootfs.diff.tar.gzip"
            )
        {
            return Err(ContentError::Unsupported);
        }
        // A plain tar descriptor and its DiffID hash the same raw bytes.
        // Compressed descriptors deliberately do not use this equality rule.
        if layer.media_type == "application/vnd.oci.image.layer.v1.tar" && layer.digest != *diff_id
        {
            return Err(ContentError::Malformed);
        }
        declared_layer_bytes = declared_layer_bytes
            .checked_add(layer.size)
            .ok_or(ContentError::Limit)?;
    }
    Ok(AdmittedImage {
        root_kind,
        root_digest,
        manifest_digest,
        config_digest,
        platform: input.platform,
        layer_count: manifest.layers.0.len() as u16,
        declared_layer_bytes,
        root_bytes: input.root.len(),
        manifest_bytes: manifest_bytes.len(),
        config_bytes: input.config.len(),
    })
}

#[cfg(test)]
mod tests;
