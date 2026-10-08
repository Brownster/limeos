//! Closed protected-read admission. Counts payload, not total allocator/cgroup heap.
use limeos_domain::{Error, ErrorCode, Result};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::{
    fmt,
    mem::size_of,
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

pub(super) const CONSUMERS: usize = 16;
pub(super) const RUNNING: usize = 8;
pub(super) const MOUNTS: usize = 32;
pub(super) const RESPONSE: usize = 64 * 1024;
const BODY_BYTES: usize = 2 * 1024 * 1024;
const LIVE_BYTES: usize = 2 * 1024 * 1024;
const NODES: usize = 2048;
const DEPTH: usize = 32;
const ITEMS: usize = 256;
const STRINGS: usize = 128 * 1024;
const STRING: usize = 16 * 1024;

/// Trusted in-process selection, never caller-selected UID or numeric limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineReadProfile {
    CombinedV1,
}

fn refused() -> Error {
    Error(ErrorCode::Unavailable)
}

#[derive(Clone)]
pub(super) struct Admission(Arc<State>);
struct State {
    body: AtomicUsize,
    live: AtomicUsize,
    collecting: AtomicBool,
}
impl Admission {
    #[cfg(test)]
    pub(super) fn remaining_live(&self) -> usize {
        self.0.live.load(Ordering::Relaxed)
    }
    pub(super) fn new(_: EngineReadProfile) -> Self {
        Self(Arc::new(State {
            body: AtomicUsize::new(BODY_BYTES),
            live: AtomicUsize::new(LIVE_BYTES),
            collecting: AtomicBool::new(false),
        }))
    }
    pub(super) fn consume(&self, bytes: usize) -> Result<()> {
        let previous = self
            .0
            .body
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                Some(left.saturating_sub(bytes))
            })
            .map_err(|_| refused())?;
        if bytes > previous {
            Err(refused())
        } else {
            Ok(())
        }
    }
    pub(super) fn ensure_body(&self) -> Result<()> {
        if self.0.body.load(Ordering::Relaxed) == 0 {
            Err(refused())
        } else {
            Ok(())
        }
    }
    pub(super) fn scope(&self) -> Scope {
        Scope {
            admission: self.clone(),
            bytes: 0,
        }
    }
    pub(super) fn collect(&self) -> Result<Collecting> {
        self.0
            .collecting
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| refused())?;
        Ok(Collecting(self.clone()))
    }
    pub(super) fn response_scope(&self, limit: usize) -> Result<Scope> {
        let mut scope = self.scope();
        // Body Vec old/new overlap, serde's reusable escaped-string Vec old/new
        // overlap (including reserve(4)), and bounded HTTP head/frame scratch.
        // These are conservative requested-byte allowances, not allocator overhead.
        scope.charge(3 * limit + 4 * limit + 64 + 64 * 1024)?;
        Ok(scope)
    }
}
pub(super) struct Collecting(Admission);
impl Drop for Collecting {
    fn drop(&mut self) {
        self.0.0.collecting.store(false, Ordering::Release);
    }
}

pub(super) struct Scope {
    admission: Admission,
    bytes: usize,
}
impl Scope {
    pub(super) fn charge(&mut self, bytes: usize) -> Result<()> {
        let total = self.bytes.checked_add(bytes).ok_or_else(refused)?;
        self.admission
            .0
            .live
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(bytes)
            })
            .map_err(|_| refused())?;
        self.bytes = total;
        Ok(())
    }
    fn release(&mut self, bytes: usize) {
        self.bytes -= bytes;
        self.admission.0.live.fetch_add(bytes, Ordering::Relaxed);
    }
    pub(super) fn string(&mut self, value: &str) -> Result<String> {
        self.charge(value.len())?;
        let mut out = String::new();
        out.try_reserve_exact(value.len()).map_err(|_| refused())?;
        // Count requested capacity; refuse a surprising reported capacity rather
        // than later refunding bytes that this ledger never reserved.
        if out.capacity() != value.len() {
            return Err(refused());
        }
        out.push_str(value);
        Ok(out)
    }
    pub(super) fn reserve<T>(&mut self, values: &mut Vec<T>, additional: usize) -> Result<()> {
        let needed = values.len().checked_add(additional).ok_or_else(refused)?;
        if needed <= values.capacity() {
            return Ok(());
        }
        let old = values
            .capacity()
            .checked_mul(size_of::<T>())
            .ok_or_else(refused)?;
        let new = needed.checked_mul(size_of::<T>()).ok_or_else(refused)?;
        // Keep the old allocation charged until replacement succeeds.
        self.charge(new)?;
        values
            .try_reserve_exact(additional)
            .map_err(|_| refused())?;
        if values.capacity() != needed {
            return Err(refused());
        }
        self.release(old);
        Ok(())
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        self.admission
            .0
            .live
            .fetch_add(self.bytes, Ordering::Relaxed);
    }
}

pub(super) struct Json {
    value: Value,
    _scope: Option<Scope>,
}
impl Deref for Json {
    type Target = Value;
    fn deref(&self) -> &Value {
        &self.value
    }
}
impl Json {
    pub(super) fn standard(value: Value) -> Self {
        Self {
            value,
            _scope: None,
        }
    }
    pub(super) fn decode(bytes: &[u8], mut scope: Scope, list: bool) -> Result<Self> {
        let mut decoder = serde_json::Deserializer::from_slice(bytes);
        let mut state = Decode {
            scope: &mut scope,
            nodes: 0,
            strings: 0,
        };
        let value = Seed {
            state: &mut state,
            depth: 0,
            array_limit: if list { CONSUMERS } else { ITEMS },
        }
        .deserialize(&mut decoder)
        .map_err(|_| refused())?;
        decoder.end().map_err(|_| refused())?;
        Ok(Self {
            value,
            _scope: Some(scope),
        })
    }
}
struct Decode<'a> {
    scope: &'a mut Scope,
    nodes: usize,
    strings: usize,
}
impl Decode<'_> {
    fn node(&mut self, depth: usize) -> std::result::Result<(), &'static str> {
        if depth > DEPTH || self.nodes >= NODES {
            return Err("JSON structural admission");
        }
        self.nodes += 1;
        Ok(())
    }
    fn string(&mut self, text: &str) -> std::result::Result<String, &'static str> {
        let total = self
            .strings
            .checked_add(text.len())
            .ok_or("JSON string admission")?;
        if text.len() > STRING || total > STRINGS {
            return Err("JSON string admission");
        }
        self.strings = total;
        self.scope
            .string(text)
            .map_err(|_| "JSON payload admission")
    }
}
struct Seed<'a, 'b> {
    state: &'a mut Decode<'b>,
    depth: usize,
    array_limit: usize,
}
impl<'de> DeserializeSeed<'de> for Seed<'_, '_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(
        self,
        decoder: D,
    ) -> std::result::Result<Value, D::Error> {
        self.state.node(self.depth).map_err(de::Error::custom)?;
        decoder.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_, '_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_unit<E: de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Value, E> {
        Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("nonfinite JSON"))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Value, E> {
        self.state.string(v).map(Value::String).map_err(E::custom)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> std::result::Result<Value, A::Error> {
        let mut values = Vec::new();
        loop {
            if values.len() == self.array_limit {
                if sequence.next_element_seed(Reject)?.is_some() {
                    return Err(de::Error::custom("JSON sequence admission"));
                }
                break;
            }
            let value = sequence.next_element_seed(Seed {
                state: self.state,
                depth: self.depth + 1,
                array_limit: ITEMS,
            })?;
            let Some(value) = value else {
                break;
            };
            self.state
                .scope
                .reserve(&mut values, 1)
                .map_err(|_| de::Error::custom("JSON payload admission"))?;
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> std::result::Result<Value, A::Error> {
        let mut values = Map::new();
        let mut entries = 0;
        loop {
            if entries == ITEMS {
                if object.next_key_seed(Reject)?.is_some() {
                    return Err(de::Error::custom("JSON map admission"));
                }
                break;
            }
            let Some(key) = object.next_key_seed(Key(self.state))? else {
                break;
            };
            entries += 1;
            let value = object.next_value_seed(Seed {
                state: self.state,
                depth: self.depth + 1,
                array_limit: ITEMS,
            })?;
            // BTreeMap has no fallible reserve. Bound/charge each entry before
            // insertion; allocator metadata/node splitting is not a heap proof.
            self.state
                .scope
                .charge(size_of::<(String, Value)>() + 4 * size_of::<usize>())
                .map_err(|_| de::Error::custom("JSON payload admission"))?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
struct Key<'a, 'b>(&'a mut Decode<'b>);
// Seq/MapAccess handles an exact closing delimiter before invoking this seed.
// An excess token is rejected without asking serde to traverse/decode it.
struct Reject;
impl<'de> DeserializeSeed<'de> for Reject {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, _: D) -> std::result::Result<(), D::Error> {
        Err(de::Error::custom("JSON collection admission"))
    }
}
impl<'de> DeserializeSeed<'de> for Key<'_, '_> {
    type Value = String;
    fn deserialize<D: de::Deserializer<'de>>(
        self,
        decoder: D,
    ) -> std::result::Result<String, D::Error> {
        self.0.node(0).map_err(de::Error::custom)?;
        decoder.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Key<'_, '_> {
    type Value = String;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded JSON key")
    }
    fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<String, E> {
        self.0.string(v).map_err(E::custom)
    }
}

#[cfg(test)]
mod tests;
