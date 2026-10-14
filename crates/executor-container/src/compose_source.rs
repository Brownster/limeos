//! Pure admission of a closed Compose JSON subset, without host/file authority.
use limeos_domain::{
    ComposeDevice, ComposeMount, ComposePort, ComposeProject, ComposeRestart, ComposeService,
    ComposeUser, Error, ErrorCode, PortProtocol, Result, compose_name,
    compose_override::{ManagedOverridePlan, plan_managed_override},
};
use serde::{
    Deserialize, Serialize,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, io::Write, marker::PhantomData};

const SOURCE_BYTES: usize = 64 * 1024;
const SUMMARY_BYTES: usize = 192 * 1024;
fn invalid() -> Error {
    Error(ErrorCode::InvalidInput)
}
fn conflict() -> Error {
    Error(ErrorCode::Conflict)
}

struct Text<const N: usize>(String);
impl<'de, const N: usize> Deserialize<'de> for Text<N> {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct BoundedText<const N: usize>;
        impl<const N: usize> Visitor<'_> for BoundedText<N> {
            type Value = Text<N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a string of at most {N} bytes")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
                if value.len() > N {
                    return Err(E::custom("Compose string limit"));
                }
                let mut owned = String::new();
                owned
                    .try_reserve_exact(value.len())
                    .map_err(|_| E::custom("Compose allocation"))?;
                owned.push_str(value);
                Ok(Text(owned))
            }
        }
        d.deserialize_str(BoundedText::<N>)
    }
}
struct Map<T, const N: usize>(BTreeMap<String, T>);
impl<T, const N: usize> Default for Map<T, N> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Map<T, N> {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct BoundedMap<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for BoundedMap<T, N> {
            type Value = Map<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a distinct-name map of at most {N} entries")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut access: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut map = BTreeMap::new();
                while let Some(key) = access.next_key::<Text<48>>()? {
                    if map.len() == N {
                        return Err(de::Error::custom("Compose map limit"));
                    }
                    if map.contains_key(&key.0) {
                        return Err(de::Error::custom("duplicate Compose name"));
                    }
                    map.insert(key.0, access.next_value()?);
                }
                Ok(Map(map))
            }
        }
        d.deserialize_map(BoundedMap::<T, N>(PhantomData))
    }
}
struct List<T, const N: usize>(Vec<T>);
impl<T, const N: usize> Default for List<T, N> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for List<T, N> {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct BoundedList<T, const N: usize>(PhantomData<T>);
        struct Entry<T> {
            allowed: bool,
            marker: PhantomData<T>,
        }
        impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Entry<T> {
            type Value = T;
            fn deserialize<D: de::Deserializer<'de>>(
                self,
                d: D,
            ) -> std::result::Result<T, D::Error> {
                if !self.allowed {
                    return Err(de::Error::custom("Compose array limit"));
                }
                T::deserialize(d)
            }
        }
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for BoundedList<T, N> {
            type Value = List<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "an array of at most {N} entries")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut access: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut list = Vec::new();
                while let Some(entry) = access.next_element_seed(Entry {
                    allowed: list.len() < N,
                    marker: PhantomData,
                })? {
                    list.try_reserve_exact(1)
                        .map_err(|_| de::Error::custom("Compose allocation"))?;
                    list.push(entry);
                }
                Ok(List(list))
            }
        }
        d.deserialize_seq(BoundedList::<T, N>(PhantomData))
    }
}
// Missing is inheritance/default. Explicit null is always a refusal.
fn present<'de, D: de::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    services: Map<Service, 16>,
    #[serde(default)]
    volumes: Map<Empty, 128>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Service {
    image: Text<264>,
    #[serde(default, deserialize_with = "present")]
    user: Option<Text<21>>,
    #[serde(default, deserialize_with = "present")]
    restart: Option<ComposeRestart>,
    #[serde(default, deserialize_with = "present")]
    privileged: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    network_mode: Option<Host>,
    #[serde(default)]
    ports: List<Port, 8>,
    #[serde(default)]
    volumes: List<Mount, 8>,
    #[serde(default)]
    devices: List<Text<517>, 8>,
}
#[derive(Deserialize)]
enum Host {
    #[serde(rename = "host")]
    Host,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Port {
    host_ip: Text<45>,
    published: Text<5>,
    target: u16,
    protocol: PortProtocol,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Volume,
    Bind,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mount {
    #[serde(rename = "type")]
    kind: Kind,
    source: Text<256>,
    target: Text<256>,
    read_only: bool,
    #[serde(default, deserialize_with = "present")]
    bind: Option<Bind>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bind {
    create_host_path: bool,
}

#[derive(Serialize)]
pub struct ComposeSourceFingerprint {
    pub bytes: usize,
    pub sha256: String,
}
/// Checked data without retained files, issue times, permissions or approval.
///
/// ```compile_fail
/// use limeos_executor_container::compose_source::ComposeJsonSources;
/// let imported: ComposeJsonSources = serde_json::from_str("{}").unwrap();
/// ```
pub struct ComposeJsonSources {
    operator: ComposeProject,
    current: ComposeProject,
    operator_volumes: Vec<String>,
    declared_volumes: Vec<String>,
    operator_fingerprint: ComposeSourceFingerprint,
    managed_fingerprint: Option<ComposeSourceFingerprint>,
}
/// A borrowed source summary cannot reconstruct the checked source value.
///
/// ```compile_fail
/// use limeos_executor_container::compose_source::ComposeJsonSourceSummary;
/// let imported: ComposeJsonSourceSummary<'_> = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Serialize)]
pub struct ComposeJsonSourceSummary<'a> {
    pub version: u16,
    pub format: &'static str,
    pub operator: &'a ComposeProject,
    pub current: &'a ComposeProject,
    pub declared_volumes: &'a [String],
    pub operator_fingerprint: &'a ComposeSourceFingerprint,
    pub managed_fingerprint: Option<&'a ComposeSourceFingerprint>,
}
impl ComposeJsonSources {
    pub fn summary(&self) -> ComposeJsonSourceSummary<'_> {
        ComposeJsonSourceSummary {
            version: 1,
            format: "strict_json",
            operator: &self.operator,
            current: &self.current,
            declared_volumes: &self.declared_volumes,
            operator_fingerprint: &self.operator_fingerprint,
            managed_fingerprint: self.managed_fingerprint.as_ref(),
        }
    }
    /// Re-admit the proposed artifact and verify its expected combination.
    /// The returned proposal retains every unresolved execution gate.
    pub fn plan(&self, desired: &ComposeProject) -> Result<ManagedOverridePlan> {
        let plan = plan_managed_override(&self.operator, &self.current, desired)?;
        let artifact = decode(plan.artifact())?;
        let (combined, _) = combine(&self.operator, &self.operator_volumes, &artifact)?;
        if &combined != plan.summary().desired {
            return Err(conflict());
        }
        Ok(plan)
    }
}

/// Admit actual JSON bytes without reading paths or using ambient environment.
pub fn admit_json_sources(operator: &[u8], managed: Option<&[u8]>) -> Result<ComposeJsonSources> {
    let document = decode(operator)?;
    let operator_project = project(&document)?;
    let operator_volumes = volume_names(&document)?;
    references(&operator_project, &operator_volumes)?;
    let (current, declared_volumes) = if let Some(raw) = managed {
        combine(&operator_project, &operator_volumes, &decode(raw)?)?
    } else {
        (operator_project.clone(), operator_volumes.clone())
    };
    let owner = ComposeJsonSources {
        operator: operator_project,
        current,
        operator_volumes,
        declared_volumes,
        operator_fingerprint: fingerprint(operator),
        managed_fingerprint: managed.map(fingerprint),
    };
    serde_json::to_writer(Count(0), &owner.summary()).map_err(|_| invalid())?;
    Ok(owner)
}
fn decode(raw: &[u8]) -> Result<Document> {
    if raw.is_empty() || raw.len() > SOURCE_BYTES {
        return Err(invalid());
    }
    serde_json::from_slice(raw).map_err(|_| invalid())
}
fn fingerprint(raw: &[u8]) -> ComposeSourceFingerprint {
    ComposeSourceFingerprint {
        bytes: raw.len(),
        sha256: hex::encode(Sha256::digest(raw)),
    }
}
fn volume_names(document: &Document) -> Result<Vec<String>> {
    if document.volumes.0.keys().any(|name| !compose_name(name)) {
        return Err(invalid());
    }
    Ok(document.volumes.0.keys().cloned().collect())
}
fn numeric(value: &str) -> Result<u32> {
    let parsed: u32 = value.parse().map_err(|_| invalid())?;
    if parsed.to_string() != value {
        return Err(invalid());
    }
    Ok(parsed)
}
fn project(document: &Document) -> Result<ComposeProject> {
    let mut services = Vec::new();
    for (name, service) in &document.services.0 {
        let user = service
            .user
            .as_ref()
            .map(|u| -> Result<ComposeUser> {
                let (uid, gid) = u.0.split_once(':').ok_or_else(invalid)?;
                Ok(ComposeUser {
                    uid: numeric(uid)?,
                    gid: numeric(gid)?,
                })
            })
            .transpose()?;
        let mut ports = Vec::new();
        for p in &service.ports.0 {
            let published = numeric(&p.published.0)?.try_into().map_err(|_| invalid())?;
            ports.push(ComposePort {
                host_ip: p.host_ip.0.clone(),
                published,
                target: p.target,
                protocol: p.protocol,
            });
        }
        let mut mounts = Vec::new();
        for m in &service.volumes.0 {
            mounts.push(match m.kind {
                Kind::Volume if m.bind.is_none() && compose_name(&m.source.0) => {
                    ComposeMount::Volume {
                        name: m.source.0.clone(),
                        target: m.target.0.clone(),
                        read_only: m.read_only,
                    }
                }
                Kind::Bind if m.bind.as_ref().is_some_and(|b| !b.create_host_path) => {
                    ComposeMount::Bind {
                        source: m.source.0.clone(),
                        target: m.target.0.clone(),
                        read_only: m.read_only,
                    }
                }
                _ => return Err(invalid()),
            });
        }
        let mut devices = Vec::new();
        for d in &service.devices.0 {
            let mut parts = d.0.split(':');
            let source = parts.next().ok_or_else(invalid)?;
            let target = parts.next().ok_or_else(invalid)?;
            if parts.next() != Some("rwm") || parts.next().is_some() {
                return Err(invalid());
            }
            devices.push(ComposeDevice {
                source: source.into(),
                target: target.into(),
            });
        }
        services.push(ComposeService {
            name: name.clone(),
            image: service.image.0.clone(),
            user,
            restart: service.restart.unwrap_or(ComposeRestart::No),
            privileged: service.privileged.unwrap_or(false),
            host_network: service.network_mode.is_some(),
            ports,
            mounts,
            devices,
        });
    }
    let mut project = ComposeProject { services };
    project.normalize()?;
    Ok(project)
}
fn target(m: &ComposeMount) -> &str {
    match m {
        ComposeMount::Volume { target, .. } | ComposeMount::Bind { target, .. } => target,
    }
}
fn references(project: &ComposeProject, volumes: &[String]) -> Result<()> {
    if project
        .services
        .iter()
        .flat_map(|s| &s.mounts)
        .any(|m| matches!(m, ComposeMount::Volume { name, .. } if !volumes.contains(name)))
    {
        return Err(invalid());
    }
    Ok(())
}
fn combine(
    operator: &ComposeProject,
    operator_volumes: &[String],
    managed: &Document,
) -> Result<(ComposeProject, Vec<String>)> {
    let additions = project(managed)?;
    let mut volumes = operator_volumes.to_vec();
    for name in volume_names(managed)? {
        if !volumes.contains(&name) {
            if volumes.len() == 128 {
                return Err(invalid());
            }
            volumes.push(name);
        }
    }
    volumes.sort();
    let mut combined = operator.clone();
    for addition in additions.services {
        let Some(base) = combined
            .services
            .iter_mut()
            .find(|s| s.name == addition.name)
        else {
            if combined.services.len() == 16 {
                return Err(invalid());
            }
            combined.services.push(addition);
            continue;
        };
        let fields = &managed.services.0[&addition.name];
        base.image = addition.image;
        if fields.user.is_some() {
            base.user = addition.user;
        }
        if fields.restart.is_some() {
            base.restart = addition.restart;
        }
        if fields.privileged.is_some() {
            base.privileged = addition.privileged;
        }
        if fields.network_mode.is_some() {
            base.host_network = true;
        }
        for port in addition.ports {
            if !base.ports.contains(&port) {
                if base.ports.len() == 8 {
                    return Err(invalid());
                }
                base.ports.push(port);
            }
        }
        for mount in addition.mounts {
            if let Some(old) = base.mounts.iter_mut().find(|m| target(m) == target(&mount)) {
                if std::mem::discriminant(old) != std::mem::discriminant(&mount) {
                    return Err(conflict());
                }
                *old = mount;
            } else {
                if base.mounts.len() == 8 {
                    return Err(invalid());
                }
                base.mounts.push(mount);
            }
        }
        for device in addition.devices {
            if base.devices.iter().any(|d| d.target == device.target) {
                return Err(conflict());
            }
            if base.devices.len() == 8 {
                return Err(invalid());
            }
            base.devices.push(device);
        }
    }
    combined.normalize()?;
    references(&combined, &volumes)?;
    Ok((combined, volumes))
}
struct Count(usize);
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > SUMMARY_BYTES.saturating_sub(self.0) {
            return Err(std::io::Error::other("Compose source summary limit"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
