use super::{ConfigurationLimits, Failure, Result};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;

/// Trusted-code selection; importing a wire value cannot select a profile.
/// ```compile_fail
/// use limeos_executor_storage::configuration::ConfigurationReadProfile;
/// let p: ConfigurationReadProfile = serde_json::from_str("\"combined_v1\"").unwrap();
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigurationReadProfile {
    CombinedV1,
}
impl ConfigurationReadProfile {
    pub fn limits(self) -> ConfigurationLimits {
        ConfigurationLimits {
            max_file_bytes: 16 * 1024,
            max_total_bytes: 32 * 1024,
            max_retained_fds: 33,
            max_report_bytes: 12 * 1024,
        }
    }
}

// One shared preflight for all protected inputs. It materializes no JSON tree.
pub(super) struct Shape {
    nodes: usize,
    strings: usize,
    refused: bool,
}
impl Shape {
    pub(super) fn new() -> Self {
        Self {
            nodes: 0,
            strings: 0,
            refused: false,
        }
    }
    fn deny<T>(&mut self) -> std::result::Result<T, &'static str> {
        self.refused = true;
        Err("configuration shape admission")
    }
    fn node(&mut self, depth: usize) -> std::result::Result<(), &'static str> {
        if depth > 16 || self.nodes == 2048 {
            return self.deny();
        }
        self.nodes += 1;
        Ok(())
    }
    fn string(&mut self, text: &str) -> std::result::Result<(), &'static str> {
        if text.len() > 4096 || text.len() > 64 * 1024 - self.strings {
            return self.deny();
        }
        self.strings += text.len();
        Ok(())
    }
    pub(super) fn inspect(&mut self, bytes: &[u8]) -> Result<()> {
        if self.refused {
            return Err(Failure::Unavailable);
        }
        let mut decoder = serde_json::Deserializer::from_slice(bytes);
        let result = Value {
            shape: self,
            depth: 0,
        }
        .deserialize(&mut decoder)
        .and_then(|()| decoder.end());
        result.map_err(|_| {
            if self.refused {
                Failure::Unavailable
            } else {
                Failure::InvalidPlan
            }
        })
    }
}

struct Value<'a> {
    shape: &'a mut Shape,
    depth: usize,
}
impl<'de> DeserializeSeed<'de> for Value<'_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        self.shape.node(self.depth).map_err(de::Error::custom)?;
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Value<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded configuration JSON")
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: de::Error>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_str<E: de::Error>(self, text: &str) -> std::result::Result<(), E> {
        self.shape.string(text).map_err(E::custom)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        for _ in 0..128 {
            if seq
                .next_element_seed(Value {
                    shape: self.shape,
                    depth: self.depth + 1,
                })?
                .is_none()
            {
                return Ok(());
            }
        }
        seq.next_element_seed(Reject(self.shape))?;
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        for _ in 0..32 {
            if map
                .next_key_seed(Key {
                    shape: self.shape,
                    depth: self.depth + 1,
                })?
                .is_none()
            {
                return Ok(());
            }
            map.next_value_seed(Value {
                shape: self.shape,
                depth: self.depth + 1,
            })?;
        }
        map.next_key_seed(Reject(self.shape))?;
        Ok(())
    }
}
struct Key<'a> {
    shape: &'a mut Shape,
    depth: usize,
}
impl<'de> DeserializeSeed<'de> for Key<'_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        self.shape.node(self.depth).map_err(de::Error::custom)?;
        d.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Key<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded configuration key")
    }
    fn visit_str<E: de::Error>(self, text: &str) -> std::result::Result<(), E> {
        self.shape.string(text).map_err(E::custom)
    }
}
struct Reject<'a>(&'a mut Shape);
impl<'de> DeserializeSeed<'de> for Reject<'_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, _: D) -> std::result::Result<(), D::Error> {
        self.0.refused = true;
        Err(de::Error::custom("configuration collection admission"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_collections_close_and_excess_tokens_refuse_before_decoding() {
        let items = std::iter::repeat_n("0", 128).collect::<Vec<_>>().join(",");
        Shape::new()
            .inspect(format!("[{items}]").as_bytes())
            .unwrap();
        assert_eq!(
            Shape::new().inspect(format!("[{items},\"\\q\"]").as_bytes()),
            Err(Failure::Unavailable)
        );
        let entries = (0..32)
            .map(|i| format!("\"{i}\":0"))
            .collect::<Vec<_>>()
            .join(",");
        Shape::new()
            .inspect(format!("{{{entries}}}").as_bytes())
            .unwrap();
        assert_eq!(
            Shape::new().inspect(format!("{{{entries},\"\\q\":0}}").as_bytes()),
            Err(Failure::Unavailable)
        );
    }
    #[test]
    fn complete_input_depth_and_trailing_syntax_are_checked() {
        let nested = |n| format!("{}0{}", "[".repeat(n), "]".repeat(n));
        Shape::new().inspect(nested(16).as_bytes()).unwrap();
        assert_eq!(
            Shape::new().inspect(nested(17).as_bytes()),
            Err(Failure::Unavailable)
        );
        for invalid in [b"{}{}".as_slice(), b"{", b"\"\\q\"", b"[0,]"] {
            assert_eq!(Shape::new().inspect(invalid), Err(Failure::InvalidPlan));
        }
    }
    #[test]
    fn nodes_and_strings_are_shared_across_inputs_and_exhaustion_latches() {
        let chunk = |n| {
            format!(
                "[{}]",
                std::iter::repeat_n("0", n).collect::<Vec<_>>().join(",")
            )
        };
        let mut chunks = vec![chunk(128); 15];
        chunks.push(chunk(111));
        let mut shape = Shape::new();
        shape
            .inspect(format!("[{}]", chunks.join(",")).as_bytes())
            .unwrap();
        assert_eq!(shape.nodes, 2048);
        assert_eq!(shape.inspect(b"0"), Err(Failure::Unavailable));
        assert_eq!(shape.inspect(b"{}"), Err(Failure::Unavailable));
        let mut shape = Shape::new();
        let text = serde_json::to_vec(&"x".repeat(4096)).unwrap();
        for _ in 0..16 {
            shape.inspect(&text).unwrap();
        }
        assert_eq!(shape.strings, 64 * 1024);
        assert_eq!(shape.inspect(b"\"x\""), Err(Failure::Unavailable));
        assert_eq!(
            Shape::new().inspect(&serde_json::to_vec(&"x".repeat(4097)).unwrap()),
            Err(Failure::Unavailable)
        );
    }
    #[test]
    fn pinned_typed_configuration_cells_fit_the_modeled_size() {
        use limeos_domain::*;
        for size in [
            size_of::<StorageContract>(),
            size_of::<StorageDevice>(),
            size_of::<StorageLocations>(),
            size_of::<StorageMediaIdentity>(),
            size_of::<PoolConfig>(),
            size_of::<PoolOptions>(),
            size_of::<PoolsConfig>(),
            size_of::<SnapraidConfig>(),
            size_of::<SnapraidDrive>(),
            size_of::<SnapraidSettings>(),
            size_of::<SnapraidThresholds>(),
            size_of::<SnapraidScrub>(),
            size_of::<SnapraidSchedule>(),
            size_of::<shares::SambaSharesConfig>(),
            size_of::<shares::SambaShareConfig>(),
            size_of::<String>(),
        ] {
            assert!(size <= 1024, "configuration cell audit changed: {size}");
        }
    }
}
