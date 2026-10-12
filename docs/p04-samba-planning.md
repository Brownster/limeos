# Samba configuration previews

RW-041 now has an independent typed Samba planning slice. It imports the saved
`samba.json` format of pi-health `80593b2` and previews generated sections and
configured storage relationships. It does not register a live share operation.

`limeos_observations::shares::import_legacy_samba(&[u8])` returns the existing
`StorageImportReport<SambaSharesConfig>` shape. A supported document produces
configuration plus explicit default findings. Any other finding blocks the whole
import; no partially imported configuration is returned. The function reads only
the supplied bytes. The caller must bound its own file read before passing them.

`limeos_domain::shares::SambaSharesConfig` provides `validate`, `render_sections`
and `plan(&StorageContract, &PoolsConfig)`. All eight frozen fields are retained:
name, path, comment, read_only, guest_ok, browseable, valid_users and enabled.
Config types reject unknown fields; their direct deserialization supplies no
defaults. The separate legacy importer applies these documented defaults:

| Missing legacy field | Default |
| --- | --- |
| shares | empty list |
| comment, valid_users | empty string |
| read_only, guest_ok | false |
| browseable, enabled | true |

Comments and user lists receive the same outer trimming as the frozen renderer.
Names and paths are preserved exactly or refused. Paths retain literal case,
UTF-8 and interior spaces. Configuration and generated section order follow the
input share order. Disabled sections are omitted from the generated text; their
configuration and dependencies remain visible. An empty or fully disabled list
produces only the owned begin/end markers, without claiming service success.

The checked subset permits bounded names made of ASCII letters, digits, `.`, `_`
and `-`, excluding `.`/`..` and reserved global/homes/printers sections. `$` names
and case-colliding names are refused. Paths must be canonical absolute paths
below `/`, with no control characters, non-space whitespace, substitutions,
backslashes, quote/config delimiters or surrounding component whitespace.
Comments are bounded literal lines; controls, backslashes, substitutions and
comment delimiters are refused. User lists permit up to 32 simple local usernames
separated by commas. Their order and separator spaces are retained; groups,
domain expressions, substitutions, duplicate/case-colliding users and embedded
lines are refused. Unsupported syntax is a migration finding, never a fallback
to a more permissive share.

Input is limited to 64 KiB and 64 shares. Names are at most 64 bytes, paths 512,
comments 240 and user lists 2048; each local username is at most 32 bytes.
The planner validates the existing contract and pool plan, including with no
shares. Unknown pool branches, inappropriate pool device roles, invalid or
recursive disabled configuration, nested device roots and pool/device root
overlaps refuse the whole request. Findings carry only fixed field paths/codes.

`SambaSharesPreview` is serialize-only and absent from generated wire contracts.
It retains the desired configuration, numeric media UID/GID, generated sections,
per-share device requirements and an explicit `unmapped_shares` list. Each
requirement carries the full configured `StorageDevice` and either `Direct` or
`Pool { pool_id, enabled, branches }`. Requirements sort by device ID and route;
branches on the same device remain in meaningful configured order.

A path beneath a device root maps directly. A path at or beneath a pool root
maps conservatively to every configured member. Ancestors map to all overlapping
configured device and pool roots. Both direct and pool routes remain distinct.
Disabled pools, disabled shares and read-only shares remain consumers. Expansion
has a global 512-requirement ceiling; an over-limit request returns no preview.
The four existing frozen layout fixtures demonstrate these relationships.

An unmapped `/srv/docs` share is preserved for later qualification. Neither it
nor a mapped path establishes actual directory existence, a physical mount,
UUID/backing identity, symlink/bind/namespace equivalence, Samba account access,
service availability or freshness. These declarations cannot authorize unmount
or runtime-loss shutdown. They must become one input to the integrator's complete
protected dependency collection, claims, approval, storage locks and recovery.

Saved JSON import and generated section preview are the delivered parity subset.
Direct `smb.conf` import remains open: the frozen parser ignores unsupported
directives, global inheritance, includes and duplicate sections. The imported
JSON fixture reproduces its existing test's output without executing that
parser. No host configuration file, account, service, package or share is changed.
Global Samba settings, inherited semantics, include ownership, status/health,
credentials, live apply/restart and return-path testing require separate work.
NFS is not counted as existing Samba parity.

The [design](plans/2026-10-12-samba-planning-design.md) records scope and ownership.
The [evidence packet](rewrite-evidence/p04/rw041-samba-planning/README.md) binds
source hashes and local checks to the reviewed code. DSK-001, MNT-001/MNT-002,
RT-001, UI-003, P04 and cutover remain open. Local planner tests do not close them
or improve the recorded inventory p95 failure of 61.401 ms against 20 ms.
