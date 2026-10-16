//! Supplied-response decoders only: no HTTP, credentials or source ownership.
use limeos_domain::{
    Error, ErrorCode, Result,
    deployment::media::{Library, MAX_PATHS, MAX_RECORDS, RootFolder},
};
use serde::{
    Deserialize, Deserializer,
    de::{IgnoredAny, MapAccess, SeqAccess, Visitor, value::MapAccessDeserializer},
};
use std::{fmt, marker::PhantomData};

pub const MAX_RESPONSE_BYTES: usize = 32 * 1024;
pub(super) struct Bounded<T, const N: usize>(pub(super) Vec<T>);
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Bounded<T, N> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        struct Items<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Items<T, N> {
            type Value = Bounded<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a bounded array")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                for _ in 0..N {
                    match seq.next_element()? {
                        Some(value) => values.push(value),
                        None => return Ok(Bounded(values)),
                    }
                }
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("record limit"));
                }
                Ok(Bounded(values))
            }
        }
        de.deserialize_seq(Items(PhantomData))
    }
}
pub(super) struct Object<T>(pub(super) T);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        struct Map<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Map<T> {
            type Value = Object<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a media metadata object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(Object)
            }
        }
        de.deserialize_map(Map(PhantomData))
    }
}
#[derive(Deserialize)]
struct RootWire {
    id: u32,
    path: String,
}
#[derive(Deserialize)]
struct LibraryWire {
    #[serde(rename = "Name", alias = "name")]
    name: String,
    #[serde(
        rename = "CollectionType",
        alias = "collectionType",
        alias = "collection_type"
    )]
    collection_type: String,
    #[serde(rename = "Locations", alias = "locations", alias = "paths")]
    paths: Bounded<String, MAX_PATHS>,
}
pub(super) fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(Error(ErrorCode::Unavailable));
    }
    serde_json::from_slice(bytes).map_err(|_| Error(ErrorCode::Unavailable))
}
/// Success means bounded selected bytes parsed, never endpoint/status/freshness proof.
pub fn root_folders(bytes: &[u8]) -> Result<Vec<RootFolder>> {
    let rows: Bounded<Object<RootWire>, MAX_RECORDS> = decode(bytes)?;
    let mut result = Vec::new();
    for Object(row) in rows.0 {
        let root = RootFolder {
            id: row.id,
            path: row.path,
        };
        root.validate().map_err(|_| Error(ErrorCode::Unavailable))?;
        result.push(root);
    }
    Ok(result)
}
/// Selected aliases match the frozen Jellyfin reader. Conflicting aliases refuse.
pub fn libraries(bytes: &[u8]) -> Result<Vec<Library>> {
    let rows: Bounded<Object<LibraryWire>, MAX_RECORDS> = decode(bytes)?;
    let mut result = Vec::new();
    for Object(row) in rows.0 {
        let library = Library {
            name: row.name,
            collection_type: row.collection_type,
            paths: row.paths.0,
        };
        library
            .validate()
            .map_err(|_| Error(ErrorCode::Unavailable))?;
        result.push(library);
    }
    Ok(result)
}
#[cfg(test)]
mod tests;
