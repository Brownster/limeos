//! Selective Engine inspect decoder. No collection, ownership or effect authority.
use limeos_domain::{
    Error, ErrorCode, Result,
    deployment::networks::{ComposeMember, Health, Node, full_id},
};
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor, value::MapAccessDeserializer},
};
use std::{fmt, marker::PhantomData};

pub const MAX_INSPECT_BYTES: usize = 32 * 1024;
pub const MAX_DEPENDENCY_LABEL_BYTES: usize = 512;

// Unknown fields are skipped without retaining environment, health logs or labels.
// These structs intentionally do not implement Debug/Serialize.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Inspect {
    id: String,
    name: String,
    #[serde(deserialize_with = "object")]
    host_config: HostConfig,
    #[serde(deserialize_with = "object")]
    state: State,
    #[serde(deserialize_with = "object")]
    config: Config,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct HostConfig {
    network_mode: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct State {
    running: bool,
    paused: bool,
    restarting: bool,
    dead: bool,
    #[serde(default, deserialize_with = "optional_object")]
    health: Option<HealthState>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct HealthState {
    status: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Config {
    #[serde(default, deserialize_with = "optional_object")]
    labels: Option<Labels>,
    #[serde(default, deserialize_with = "optional_object")]
    healthcheck: Option<Healthcheck>,
}
// Presence only. The command/test/options are skipped, never stored or run.
// A configured/disabled check without an observed result stays Unknown.
#[derive(Deserialize)]
struct Healthcheck {}
#[derive(Deserialize)]
struct Labels {
    #[serde(rename = "com.docker.compose.depends_on")]
    depends_on: Option<String>,
}

// Serde structs also accept JSON sequences by default. Engine objects must be
// maps, including empty healthcheck/labels objects; [] cannot mean absent state.
fn object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    de: D,
) -> std::result::Result<T, D::Error> {
    struct Object<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Object<T> {
        type Value = T;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an Engine object")
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> std::result::Result<T, A::Error> {
            T::deserialize(MapAccessDeserializer::new(map))
        }
    }
    de.deserialize_map(Object(PhantomData))
}
fn optional_object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    de: D,
) -> std::result::Result<Option<T>, D::Error> {
    struct Optional<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Optional<T> {
        type Value = Option<T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an Engine object or null")
        }
        fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: Deserializer<'de>>(
            self,
            de: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            object(de).map(Some)
        }
    }
    de.deserialize_option(Optional(PhantomData))
}

/// Parse one successful inspect body selected by the collector. `compose` must
/// come from the same B2 cohort. Expected ID/bytes/membership remain plain inputs,
/// not protected evidence; central collection must bind them and original time.
pub fn parse_inspect(
    expected_id: &str,
    bytes: &[u8],
    compose: Option<ComposeMember>,
) -> Result<Node> {
    let refused = || Error(ErrorCode::Unavailable);
    if !full_id(expected_id) || bytes.len() > MAX_INSPECT_BYTES {
        return Err(refused());
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let inspect: Inspect = object(&mut decoder).map_err(|_| refused())?;
    decoder.end().map_err(|_| refused())?;
    if inspect.id != expected_id {
        return Err(Error(ErrorCode::Conflict));
    }
    let name = inspect.name.strip_prefix('/').ok_or_else(refused)?;
    let dependency_hint = inspect.config.labels.and_then(|l| l.depends_on);
    if dependency_hint
        .as_ref()
        .is_some_and(|s| s.len() > MAX_DEPENDENCY_LABEL_BYTES)
    {
        return Err(refused());
    }
    let dependency_hint = dependency_hint.as_deref().and_then(single_dependency);
    let health = match inspect.state.health {
        None if inspect.config.healthcheck.is_some() => Health::Unknown,
        None => Health::NotConfigured,
        Some(health) => match health.status.as_str() {
            "healthy" => Health::Healthy,
            "unhealthy" => Health::Unhealthy,
            "starting" => Health::Starting,
            _ => Health::Unknown,
        },
    };
    let node = Node {
        id: inspect.id,
        name: name.into(),
        running: inspect.state.running
            && !inspect.state.paused
            && !inspect.state.restarting
            && !inspect.state.dead,
        health,
        network_mode: Some(inspect.host_config.network_mode),
        compose,
        dependency_hint,
    };
    node.validate().map_err(|_| refused())?;
    Ok(node)
}

// A single well-formed legacy dependency is only a display hint. Never select
// the first of multiple dependencies, or infer a reference from an arbitrary label.
fn single_dependency(label: &str) -> Option<String> {
    let mut fields = label.split(':');
    let service = fields.next()?;
    let condition = fields.next()?;
    let restart = fields.next()?;
    if fields.next().is_some()
        || !matches!(
            condition,
            "service_started" | "service_healthy" | "service_completed_successfully"
        )
        || !matches!(restart, "true" | "false")
        || service.is_empty()
        || service.len() > 64
        || !service.as_bytes()[0].is_ascii_alphanumeric()
        || !service
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return None;
    }
    Some(service.into())
}

#[cfg(test)]
mod tests;
