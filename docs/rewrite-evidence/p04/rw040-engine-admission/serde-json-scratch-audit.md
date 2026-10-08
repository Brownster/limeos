# RW-040 pinned JSON decoder scratch

Read-only audit for the Engine admission proposal and its independent API review.
No source, features, dependencies, tests, Git or host operations changed.
This is implementation evidence, not source approval or allocator qualification.

**Use a pre-decode counted scratch reservation of `4 * B + 64` bytes per active
decoder**, separate from its raw body, owned visitor output and comparison
copies. B is the admitted body cap. That is 32,832 bytes for the 8 KiB version
body and 262,208 bytes for a 64 KiB body. Keep the reservation until the fresh
`Deserializer::from_slice` is dropped, including refused/malformed inputs.
This is a conservative requested-buffer allowance under the pins below, not a
universal bound on allocator use, RSS or stack. The proposed `2 * B` allowance
does not cover old/new allocation overlap during growth.

## Pins and active paths

`Cargo.lock` pins serde_json **1.0.145**. Read-only
`cargo tree --locked --offline --workspace -e features -i serde_json` resolves
`alloc/default/raw_value/std`; `arbitrary_precision`, `float_roundtrip`,
`unbounded_depth` and `preserve_order` are absent. Local cached fingerprints
corroborate these enabled configurations. Rust is **1.88.0** in
`rust-toolchain.toml`, CI and `packaging/Containerfile`, matching local rustc.

The dependency source inspected was
`/home/marc/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_json-1.0.145`.

| Path | Behavior before/around the visitor | Consequence |
|---|---|---|
| `read.rs:494` `SliceRead::parse_str_bytes` | Unescaped strings borrow the input. Escaped strings append runs and decoded escapes to the reusable scratch Vec before `visit_str`. Keys take the same path. | Visitor string limits alone cannot bound scratch allocation. Unknown keys/values need the same pre-decode reservation. |
| `read.rs:874`, `978` escape/Unicode helpers | Ordinary escapes push one byte. Unicode uses `reserve(4)` before writing <=4 UTF-8 bytes; valid escapes never expand beyond their raw representation. | Scratch length is bounded by the capped input; reserve headroom must be counted too. |
| `de.rs:33`, `1425`, `2208` | One scratch Vec per deserializer; strings clear its length, retaining capacity. Scratch and a visitor's owned output string can overlap. | Do not release a scratch reservation after one string or mistake clear for deallocation. Output copying has a separate reservation. |
| `de.rs:932`, `714`, `834` active number paths | `deserialize_any` uses u64/i64/f64 parsing; long integer/decimal tails scan input without a growing numeric buffer. | Long numbers still consume input/work, but no numeric scratch multiplier is needed for this active path. |
| `de.rs:674`, `937`, `373`, `403` inactive/different numeric paths | `float_roundtrip` builds digit scratch; `arbitrary_precision` creates a numeric String. Explicit i128/u128 deserialization also builds a String, even without those features. | Keep the constrained seed on `deserialize_any` with string keys. A new feature or typed numeric/key path requires another audit; `4B+64` is not a promise for arbitrary decoder entry points. |
| `de.rs:1100` ignored values | `IgnoredAny` iterates a nesting-frame stack in the same scratch Vec and skips strings. This path does not apply the ordinary recursion counter. | Do not skip unknown fields through IgnoredAny: that bypasses the proposed visitor node/depth accounting. The approved visitor must walk and charge them. |
| `de.rs:63`, `1370`, `1432` recursion | Default remaining depth starts at 128; array/map `deserialize_any` entry checks it. `unbounded_depth` is disabled. | Enforce the proposed depth32 before descending to the next child. The default limit does not prove stack-byte headroom for the visitor or target architecture. |

Enabled `raw_value` does not activate an additional raw-value buffer merely
because `deserialize_any` is used. Do not change the constrained seed to
RawValue, from_reader, a stream reused across different body caps, or a typed
numeric/key decoder without revisiting this reservation.

## Conditional requested-buffer calculation

In the pinned Rust 1.88 RawVec growth implementation, requested capacity is the
maximum of twice the old capacity, the required capacity and a small initial
capacity. Here scratch growth requires at most B+4 bytes. Thus final requested
capacity is at most 2(B+4), while a growth may temporarily hold old and new
requests totalling at most 3(B+4). `4B+64` leaves margin over that model for the
closed 8/64 KiB body caps. This reasoning concerns requested buffer storage,
not allocator internals. [Pinned RawVec source](https://raw.githubusercontent.com/rust-lang/rust/1.88.0/library/alloc/src/raw_vec/mod.rs)
(local read-only copy: `/tmp/limeos-rust-1.88-raw-vec.rs`, `grow_amortized`).

The overlap is reachable in the source model: put B-16 ordinary ASCII bytes
before a late `\n` escape in one string. Copying that prefix can request B-16
bytes; pushing the decoded newline then doubles that buffer. Old plus new is
3(B-16), exceeding 2B for both closed body caps. No probe was run to measure
allocator behavior for this example.

Vec's stable contract does not fix its growth policy, and allocators may provide
more storage than requested. A Rust/dependency/feature or allocator change
therefore needs re-audit and measurement; the policy cannot claim universal
exact heap accounting. [Official Vec guarantees](https://doc.rust-lang.org/std/vec/struct.Vec.html#guarantees)

## Boundaries the scratch allowance does not cover

The raw body's allocation/capacity, visitor Vec/String/BTreeMap nodes, replaced
duplicate keys, retained selected snapshots, simultaneous fresh comparisons,
decoder/visitor error objects and recursive stack frames are separate costs.
Keep refusal messages static/bounded so a visitor error does not copy an
oversized key/string while reporting its rejected size. Serde_json's internal
`push`/`extend`/`reserve` are infallible allocation paths: reservation admission
does not turn their allocator failure into a recoverable typed error.

Overlapping decodes need one reservation each, acquired through the shared live
budget before decoder construction; a concurrent call cannot reuse another
call's reservation. Release live reservations after their actual values/scratch
drop, while retaining the cumulative HTTP-body spend. An oversized escaped
string can allocate scratch and then be refused by the visitor, so that error
case must stay within the same reservation.

The implementer's focused evidence should cover a large escaped key/string
refused after scratch decode, Unicode growth near a capacity boundary, multiple
escaped strings separated by clear, a long default numeric token, depth32/33,
unknown fields and overlapping comparison decodes. No probes/tests were run
for this audit. A smaller raw cap reduces the same conditional allowance;
no second semantic parser or new package/feature is needed.
