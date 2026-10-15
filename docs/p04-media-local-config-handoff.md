# Local media configuration planning handoff

Engineer B, 15 October 2026. Branch `engineer/p04-media-local-config`, base
`25c9cbe`. Tested product source `ee83fefc99468000afd509d79b9e769fd187535a`.
This continues the media library/activity/catalog namespace lineage; other B
deliveries remain separate. No A or central runtime dependency was needed.

## Delivered

`domain::deployment::local_config::plan` accepts an explicit application dialect,
typed complete/optional-incomplete container paths and optional supplied source
bytes. It produces a private `ConfigDraft` or fixed `ConfigError`. There is no
file access, service/config discovery, credential read, command, HTTP or effect.

- **Transmission legacy kebab-case JSON:** change only `download-dir`,
  `incomplete-dir` and `incomplete-dir-enabled`. Check the JSON structure and
  selected types, root-key duplication (including escaped keys), root count and
  bounded nested structure. Patch selected value spans or insert missing fields;
  never reserialize unrelated data. Preserve secret values, unknown nested data,
  numeric spelling, key escaping, whitespace and original line endings.
- **SABnzbd targeted ConfigObj subset:** change only top-level `[misc]`
  `complete_dir`/`download_dir`. Preserve the bare version preamble, comments,
  inline comments, BOM/CRLF and nested server sections. Insert missing keys before
  nested sections; append a missing `[misc]` as a top-level section. Quote new
  paths. Refuse duplicate sections/selected aliases, duplicate keys in `[misc]`,
  malformed headers, quoted section/key constructs where interpreted, multiline
  constructs, selected list values, and generated scalar/section name collisions.
- Missing source is `Unavailable`, rather than an assumed empty file. An existing
  empty SABnzbd file is distinct and can receive a draft; empty/invalid JSON
  refuses. This preserves the frozen directory-setting rules, without turning a
  failed or absent read into a writable default configuration.
- Omitting the incomplete path leaves existing incomplete settings untouched;
  it does not disable or delete them. A no-op returns no artifact. Replanning
  changed output with the same desired directories is a no-op in covered cases.

Bounds apply before source copying: 64 KiB input/output, 512 UTF-8 bytes per
desired path, 128 root JSON fields / `[misc]` scalar fields, 2048 INI lines and
32 levels of nesting. Generated fields/lines count toward admission, keeping
output admissible on rerun. Desired paths preserve case/interior spaces and must
be absolute/literal, without traversal, template syntax or equal/ancestor overlap.
Double quotes/backslashes and unsupported whitespace in desired paths refuse.

`ConfigDraft` has neither Serialize nor Deserialize. Its Debug shows application,
changed field names and artifact length only. `summary()` serializes desired
paths, changed field names, byte count and unresolved gates, without source values
or artifacts. **`artifact()` can contain existing passwords/API keys** and is only
input for a future protected writer; do not log it or return it in diagnostic APIs.
No zeroization or protected custody is claimed for these ordinary supplied bytes.

## Integration and remaining parity

1. Export/register the leaf after the preceding B lineage. Existing serde and
   serde_json dependencies suffice; no shared crate root/manifest/lock, Store,
   schema, protocol, API, UI, package or activation file changed here. The local
   catalog harness exposes the dormant leaf only for these tests.
2. Bind the exact protected config file/preimage and metadata to its current
   Engine/project/service/container instance and approved dialect. Derive actual
   container paths from the existing storage contract, resolved mounts and numeric
   identity. Literal strings and matching defaults prove no mounted state,
   writability, ownership or hardlink behavior. Do not trust caller-supplied file
   paths, byte arrays or DTOs as protected source authority.
3. `TransmissionLegacy` is explicit: selected snake-case aliases refuse instead
   of adding ignored/conflicting legacy keys. Their absence does not establish
   the installed version/save format. A modern-format adapter remains pending.
   SABnzbd support is a conservative targeted subset, not complete ConfigObj
   validation. Unknown JSON values are structurally skipped and preserved, not
   interpreted; unknown INI values outside selected scalars are not validated.
   Actual application/format compatibility needs installed qualification.
4. All six gates remain unresolved even for a no-op: protected file/current
   instance, verified storage/numeric identity, verified service quiescence,
   current authority/claims/locks, atomic before-image/recovery and verified loaded
   service result. Serialize with deployment and backup through the shared path.
   Stop and verify the owning service before applying its local configuration;
   Transmission may overwrite edits when it shuts down. Obtain a durable backup/
   before-image and preserve file owner/mode. Apply atomically, reconcile uncertain
   writes/restarts without blind replay, and independently verify loaded settings.
5. Register authenticated review/job/status API/UI centrally. GET must perform no
   file or service effect. No write, receipt, fresh approval or recovery path is
   supplied by this draft. Download-client/indexer application registration,
   management/naming/quality settings, complete quickstart and full gateway/source
   support remain open; no feature is retired. B1 still awaits central durable
   Store/transaction registration. RW-042 and P04 remain incomplete.

## Evidence

[Packet](rewrite-evidence/p04/2026-10-15-media-local-config/README.md): **109 passing
domain tests** — 24 new local-config, 37 catalog/namespace, 21 media library,
15 activity and 12 discovery. Strict Clippy across all harness targets and
Rustfmt pass. The product was committed after those checks without intervening
product edits. The initial 107-case run preceded two added regression tests;
it is not an extra 107 cases to add to the final total.

Review reproduced two ConfigObj namespace collisions before the fix: a directory
scalar could collide with a direct child section, and a generated `[misc]` section
could collide with a preamble scalar. Both failed cases, their source snapshots
and the final passing run are retained. Fixtures port the frozen local-directory
tests with fake secret sentinels and added byte-preservation regressions. Frozen
Python is read/hashed only, never imported or executed.

No decoder/API/frontend/browser, actual ConfigObj/application process, root-positive
file custody/metadata, service stop/write/restart/recovery, package/ARM64/Pi,
worker footprint, performance or soak qualification ran. Existing packets retain
their own source/results. Main and production hosts remain unchanged; this branch
has not been merged or pushed.

Primary format references: [Transmission 4.0.6 configuration
editing](https://github.com/transmission/transmission/blob/4.0.6/docs/Editing-Configuration-Files.md),
[current Transmission format transition](https://github.com/transmission/transmission/blob/main/docs/Editing-Configuration-Files.md),
and [ConfigObj format/reference](https://configobj.readthedocs.io/en/latest/configobj.html).
