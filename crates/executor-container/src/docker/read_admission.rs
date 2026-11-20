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
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
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
    /// Engine-wide Compose membership for stack inventory. Admits a whole Engine
    /// of up to `limeos_domain::deployment::discovery::MAX_ENGINE_CONTAINERS` containers. The per-stack
    /// member bound (`MAX_STACK_MEMBERS`) is enforced separately and unchanged.
    EngineWideV1,
}

/// Closed numeric limits of one profile. Never derived from caller input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Limits {
    pub(super) consumers: usize,
    pub(super) running: usize,
    /// Selected (non-tmpfs) mounts across the whole Engine.
    pub(super) mounts_total: usize,
    /// Selected mounts of one container on revalidation.
    pub(super) mounts_each: usize,
    pub(super) response: usize,
    pub(super) list_response: usize,
    /// Byte cap of the selective Engine listing that orders truncation.
    pub(super) listing: usize,
    /// Rows that listing may name; beyond `consumers` they are truncated.
    pub(super) listed: usize,
    pub(super) body: usize,
    pub(super) live: usize,
    pub(super) nodes: usize,
    pub(super) strings: usize,
    /// Streamed (never allocated) size ceiling of the retained evidence report.
    pub(super) report: usize,
}
impl EngineReadProfile {
    pub(super) const fn limits(self) -> Limits {
        match self {
            Self::CombinedV1 => Limits {
                consumers: CONSUMERS,
                running: RUNNING,
                mounts_total: MOUNTS,
                mounts_each: MOUNTS,
                response: RESPONSE,
                list_response: RESPONSE,
                listing: RESPONSE,
                listed: CONSUMERS,
                body: BODY_BYTES,
                live: LIVE_BYTES,
                nodes: NODES,
                strings: STRINGS,
                report: 48 * 1024,
            },
            // Sized for 64 containers: one list row is at most a few KiB, so the
            // list may use 256 KiB; the 7x scratch model then charges 1.8 MiB
            // of a 4 MiB live ledger. Every inspect keeps the 64 KiB response cap
            // and 128 inspect bodies (two passes) fit the 4 MiB body allowance at
            // 32 KiB average. This projection scans no container processes, so
            // running containers share the consumer bound.
            Self::EngineWideV1 => Limits {
                consumers: limeos_domain::deployment::discovery::MAX_ENGINE_CONTAINERS,
                running: limeos_domain::deployment::discovery::MAX_ENGINE_CONTAINERS,
                mounts_total: limeos_domain::CONTAINER_STORAGE_MAX_TOTAL_MOUNTS,
                mounts_each: limeos_domain::CONTAINER_STORAGE_MAX_MOUNTS,
                response: RESPONSE,
                list_response: 256 * 1024,
                // More containers than `consumers` are listed selectively
                // (identity, state and name only), ordered and truncated. The
                // listing is charged 4x its byte cap, 2 MiB of the live ledger.
                listing: 512 * 1024,
                listed: 1024,
                // Bytes read, not held: two listings and two inspect passes
                // for the collection and again for its revalidation.
                body: 8 * 1024 * 1024,
                live: 4 * 1024 * 1024,
                nodes: 8192,
                strings: 256 * 1024,
                // 64 containers x (selected facts + Compose identity) plus up to
                // 256 selected mounts, counted by a streaming writer.
                report: 256 * 1024,
            },
        }
    }
}

/// Retained selected projections cannot recover after a failed or abandoned
/// comparison. Concurrent entry poisons the first comparison as well.
pub(super) struct ValidationGate(AtomicU8);
impl ValidationGate {
    pub(super) fn new() -> Self {
        Self(AtomicU8::new(0))
    }
    pub(super) fn begin(&self) -> Result<Validation<'_>> {
        if self
            .0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.0.store(2, Ordering::Release);
            return Err(Error(ErrorCode::Conflict));
        }
        Ok(Validation {
            gate: self,
            finished: false,
        })
    }
}
pub(super) struct Validation<'a> {
    gate: &'a ValidationGate,
    finished: bool,
}
impl Validation<'_> {
    pub(super) fn finish(mut self, result: Result<()>) -> Result<()> {
        let result = match result {
            Ok(())
                if self
                    .gate
                    .0
                    .compare_exchange(1, 0, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok() =>
            {
                Ok(())
            }
            other => {
                self.gate.0.store(2, Ordering::Release);
                other.and(Err(Error(ErrorCode::Conflict)))
            }
        };
        self.finished = true;
        result
    }
}
impl Drop for Validation<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.gate.0.store(2, Ordering::Release);
        }
    }
}

fn refused() -> Error {
    Error(ErrorCode::Unavailable)
}

#[derive(Clone)]
pub(super) struct Admission(Arc<State>);
struct State {
    limits: Limits,
    body: AtomicUsize,
    live: AtomicUsize,
    collecting: AtomicBool,
}
impl Admission {
    #[cfg(test)]
    pub(super) fn remaining_live(&self) -> usize {
        self.0.live.load(Ordering::Relaxed)
    }
    pub(super) fn new(profile: EngineReadProfile) -> Self {
        let limits = profile.limits();
        Self(Arc::new(State {
            limits,
            body: AtomicUsize::new(limits.body),
            live: AtomicUsize::new(limits.live),
            collecting: AtomicBool::new(false),
        }))
    }
    pub(super) fn limits(&self) -> Limits {
        self.0.limits
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
    pub(super) fn limits(&self) -> Limits {
        self.admission.limits()
    }
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
    pub(super) fn decode(bytes: &[u8], scope: Scope, list: bool) -> Result<Self> {
        Self::decode_mode(bytes, scope, list, false)
    }
    pub(super) fn decode_unique(bytes: &[u8], scope: Scope, list: bool) -> Result<Self> {
        Self::decode_mode(bytes, scope, list, true)
    }
    fn decode_mode(bytes: &[u8], mut scope: Scope, list: bool, unique: bool) -> Result<Self> {
        let limits = scope.admission.limits();
        let mut decoder = serde_json::Deserializer::from_slice(bytes);
        let mut state = Decode {
            limits,
            scope: &mut scope,
            nodes: 0,
            strings: 0,
            unique,
        };
        let value = Seed {
            state: &mut state,
            depth: 0,
            array_limit: if list { limits.consumers } else { ITEMS },
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
    limits: Limits,
    scope: &'a mut Scope,
    nodes: usize,
    strings: usize,
    unique: bool,
}
impl Decode<'_> {
    fn node(&mut self, depth: usize) -> std::result::Result<(), &'static str> {
        if depth > DEPTH || self.nodes >= self.limits.nodes {
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
        if text.len() > STRING || total > self.limits.strings {
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
            if self.state.unique && values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON key"));
            }
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
