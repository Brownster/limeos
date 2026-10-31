//! Bounded past container facts. Authentication and original freshness stay with Core.
use super::*;
use crate::{Error, ErrorCode, Result};
use serde::{
    Deserialize, Serialize,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor, value::MapAccessDeserializer},
};
use std::{fmt, io::Write, marker::PhantomData};

pub const CONTAINER_HEALTH_MEMBERS: usize = 16;
pub const CONTAINER_HEALTH_BYTES: usize = 32 * 1024;
pub const CONTAINER_HEALTH_SECONDS: i64 = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ContainerHealthMember {
    #[schemars(length(min = 74, max = 74))]
    pub resource: String,
    pub signal: Option<AlertSignal>,
}
/// Plain selected observations. Deserialization restores no Engine or poll lease.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ContainerHealthCohort {
    version: u16,
    #[schemars(length(min = 64, max = 64))]
    nonce: String,
    #[schemars(length(min = 1, max = 128))]
    engine_id: String,
    #[ts(type = "number")]
    sampled_at: i64,
    complete: bool,
    #[schemars(length(max = 16))]
    members: Vec<ContainerHealthMember>,
}
fn invalid() -> Error {
    Error(ErrorCode::InvalidInput)
}
impl ContainerHealthCohort {
    pub fn new(
        nonce: String,
        engine_id: String,
        sampled_at: i64,
        members: Vec<ContainerHealthMember>,
    ) -> Result<Self> {
        if !crate::opaque_id(&nonce)
            || !super::text(&engine_id, 128)
            || !valid_time(sampled_at)
            || members.len() > CONTAINER_HEALTH_MEMBERS
        {
            return Err(invalid());
        }
        let mut previous = None;
        for member in &members {
            let id = member
                .resource
                .strip_prefix("container:")
                .ok_or_else(invalid)?;
            if !crate::opaque_id(id)
                || previous.is_some_and(|p: &str| p >= member.resource.as_str())
            {
                return Err(invalid());
            }
            previous = Some(member.resource.as_str());
            if let Some(signal) = &member.signal {
                signal.validate()?;
                if signal.kind != AlertKind::Container || signal.key != member.resource {
                    return Err(invalid());
                }
            }
        }
        let complete = members.iter().all(|m| {
            m.signal
                .as_ref()
                .is_none_or(|s| s.status != SignalStatus::Unavailable)
        });
        let value = Self {
            version: 1,
            nonce,
            engine_id,
            sampled_at,
            complete,
            members,
        };
        value.check_size()?;
        Ok(value)
    }
    pub fn nonce(&self) -> &str {
        &self.nonce
    }
    pub fn engine_id(&self) -> &str {
        &self.engine_id
    }
    pub fn sampled_at(&self) -> i64 {
        self.sampled_at
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub fn members(&self) -> &[ContainerHealthMember] {
        &self.members
    }
    pub fn check_time(&self, requested: i64, received: i64) -> Result<()> {
        if !valid_time(requested)
            || !valid_time(received)
            || self.sampled_at < requested
            || received < self.sampled_at
            || received - self.sampled_at > CONTAINER_HEALTH_SECONDS
        {
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > CONTAINER_HEALTH_BYTES {
            return Err(Error(ErrorCode::Overloaded));
        }
        serde_json::from_slice(bytes).map_err(|_| invalid())
    }
    fn check_size(&self) -> Result<()> {
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self
                    .0
                    .checked_add(bytes.len())
                    .filter(|n| *n <= CONTAINER_HEALTH_BYTES)
                    .ok_or_else(|| std::io::Error::other("health cohort limit"))?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(Count(0), self).map_err(|_| Error(ErrorCode::Overloaded))
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.check_size()?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(CONTAINER_HEALTH_BYTES)
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        serde_json::to_writer(&mut bytes, self).map_err(|_| invalid())?;
        Ok(bytes)
    }
}

struct Text<const N: usize>(String);
impl<'de, const N: usize> Deserialize<'de> for Text<N> {
    fn deserialize<D: de::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        struct Bound<const N: usize>;
        impl<const N: usize> Visitor<'_> for Bound<N> {
            type Value = Text<N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "at most {N} health text bytes")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
                if value.len() > N {
                    return Err(E::custom("health text limit"));
                }
                let mut text = String::new();
                text.try_reserve_exact(value.len())
                    .map_err(|_| E::custom("health allocation"))?;
                text.push_str(value);
                Ok(Text(text))
            }
        }
        decoder.deserialize_str(Bound::<N>)
    }
}
fn members<'de, D: de::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Vec<MemberWire>, D::Error> {
    struct Rows;
    struct Entry(bool);
    impl<'de> DeserializeSeed<'de> for Entry {
        type Value = MemberWire;
        fn deserialize<D: de::Deserializer<'de>>(
            self,
            decoder: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            if !self.0 {
                return Err(de::Error::custom("health member limit"));
            }
            object(decoder)
        }
    }
    impl<'de> Visitor<'de> for Rows {
        type Value = Vec<MemberWire>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("at most sixteen health members")
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut rows: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(row) =
                rows.next_element_seed(Entry(out.len() < CONTAINER_HEALTH_MEMBERS))?
            {
                out.try_reserve_exact(1)
                    .map_err(|_| de::Error::custom("health allocation"))?;
                out.push(row);
            }
            Ok(out)
        }
    }
    decoder.deserialize_seq(Rows)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    version: u16,
    nonce: Text<64>,
    engine_id: Text<128>,
    sampled_at: i64,
    complete: bool,
    #[serde(deserialize_with = "members")]
    members: Vec<MemberWire>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberWire {
    resource: Text<74>,
    #[serde(deserialize_with = "optional_signal")]
    signal: Option<SignalWire>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalWire {
    key: Text<74>,
    kind: AlertKind,
    status: SignalStatus,
    severity: Severity,
    summary: Text<512>,
}
impl<'de> Deserialize<'de> for ContainerHealthCohort {
    fn deserialize<D: de::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        let wire: Wire = object(decoder)?;
        if wire.version != 1 {
            return Err(de::Error::custom("health cohort version"));
        }
        let members = wire
            .members
            .into_iter()
            .map(|m| ContainerHealthMember {
                resource: m.resource.0,
                signal: m.signal.map(|s| AlertSignal {
                    key: s.key.0,
                    kind: s.kind,
                    status: s.status,
                    severity: s.severity,
                    summary: s.summary.0,
                }),
            })
            .collect();
        let value = Self::new(wire.nonce.0, wire.engine_id.0, wire.sampled_at, members)
            .map_err(|_| de::Error::custom("invalid health cohort"))?;
        if value.complete != wire.complete {
            return Err(de::Error::custom("health completeness mismatch"));
        }
        Ok(value)
    }
}

fn object<'de, D: de::Deserializer<'de>, T: Deserialize<'de>>(
    decoder: D,
) -> std::result::Result<T, D::Error> {
    struct Object<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Object<T> {
        type Value = T;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a health object")
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> std::result::Result<T, A::Error> {
            T::deserialize(MapAccessDeserializer::new(map))
        }
    }
    decoder.deserialize_map(Object(PhantomData))
}
fn optional_signal<'de, D: de::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<SignalWire>, D::Error> {
    struct Optional;
    impl<'de> Visitor<'de> for Optional {
        type Value = Option<SignalWire>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an explicit signal object or null")
        }
        fn visit_none<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: de::Deserializer<'de>>(
            self,
            decoder: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            object(decoder).map(Some)
        }
    }
    decoder.deserialize_option(Optional)
}

#[cfg(test)]
mod tests;
