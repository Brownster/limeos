//! Validated values: one rule per concept, held by a type whose field is
//! private, so a value of the type is valid by construction.
//!
//! Each string type serialises, deserialises, generates its JSON schema and
//! TypeScript type exactly as a bare `String` does, so no wire, stored or
//! generated contract changes. Deserialising validates (`TryFrom<String>`):
//! an invalid value is a decode error, never a held value.
//!
//! Context rules (a `/mnt/` prefix, a shorter bound, extra excluded
//! characters) stay with their context and are applied on top of the rule
//! here, never instead of it. Executor crates may depend on this module
//! (`limeos-domain` is the shared lower layer).
use crate::{Error, ErrorCode};

fn invalid() -> Error {
    Error(ErrorCode::InvalidInput)
}

/// Implements the shared surface of a validated string newtype. `$valid` is
/// the type's one rule.
macro_rules! text_newtype {
    ($name:ident, $valid:path, $what:literal) => {
        impl $name {
            /// Validates and takes the value.
            pub fn parse(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                if $valid(&value) {
                    Ok(Self(value))
                } else {
                    Err(invalid())
                }
            }
            /// The rule, for callers that only need a verdict.
            pub fn valid(value: &str) -> bool {
                $valid(value)
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
            pub fn into_string(self) -> String {
                self.0
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }
        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        // Debug prints as the bare string did, so no diagnostic output changes.
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Debug::fmt(&self.0, f)
            }
        }
        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }
        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }
        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool {
                &self.0 == other
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = Error;
            fn try_from(value: String) -> Result<Self> {
                Self::parse(value)
            }
        }
        impl TryFrom<&str> for $name {
            type Error = Error;
            fn try_from(value: &str) -> Result<Self> {
                Self::parse(value)
            }
        }
        impl std::str::FromStr for $name {
            type Err = Error;
            fn from_str(value: &str) -> Result<Self> {
                Self::parse(value)
            }
        }
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(|_| serde::de::Error::custom(concat!("invalid ", $what)))
            }
        }
        // The schema and the TypeScript type are those of `String`, so the
        // generated contracts are unchanged.
        impl schemars::JsonSchema for $name {
            fn inline_schema() -> bool {
                <String as schemars::JsonSchema>::inline_schema()
            }
            fn schema_name() -> std::borrow::Cow<'static, str> {
                <String as schemars::JsonSchema>::schema_name()
            }
            fn schema_id() -> std::borrow::Cow<'static, str> {
                <String as schemars::JsonSchema>::schema_id()
            }
            fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
                <String as schemars::JsonSchema>::json_schema(generator)
            }
        }
        impl ts_rs::TS for $name {
            type WithoutGenerics = Self;
            type OptionInnerType = Self;
            fn name() -> String {
                <String as ts_rs::TS>::name()
            }
            fn inline() -> String {
                <String as ts_rs::TS>::inline()
            }
            fn inline_flattened() -> String {
                <String as ts_rs::TS>::inline_flattened()
            }
            fn decl() -> String {
                <String as ts_rs::TS>::decl()
            }
            fn decl_concrete() -> String {
                <String as ts_rs::TS>::decl_concrete()
            }
        }
    };
}

mod lifetime;
mod path;
mod resource;
mod storage_identity;
pub use lifetime::*;
pub use path::*;
pub use resource::*;
pub use storage_identity::*;
