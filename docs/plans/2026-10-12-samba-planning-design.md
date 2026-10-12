# RW-041 Samba configuration planning

Base `1401c255d4a05b8621c2dcd497d6f7f0cbe91d56`; branch
`engineer/samba-planning`; disk-backed worktree `.worktrees/samba-planning`.
The frozen reference remains pi-health `80593b2`. Read Git objects only; do not
run its plugin, change its files, or contact the media host. Main runtime,
backup recovery, storage locks and the frozen ARM64 qualification stay with
their current owners. This slice adds no operation or service authority.

Estimate 12 human engineering hours, review at 18. P04 remains 320 hours with a
480-hour scope review. Actual human and agent effort remain unmeasured.

Use the existing domain/observations split. Add `domain::shares` with closed
Samba configuration fields, validation, deterministic generated section previews
and declared share-to-device/pool relationships. Add `observations::shares` with
a bounded read-only import of the frozen `samba.json` shape. Both module exports
are additive; no dependencies, generated wire types, schema or HTTP changes.

Preserve share order, literal case-sensitive paths (including interior spaces),
comments, user-list order, read-only/guest/browseable/enabled flags, numeric media
ownership and meaningful pool branch order. Import only the frozen defaults:
empty shares/comment/valid_users, read_only=false, guest_ok=false,
browseable=true, enabled=true. Report every default with a fixed field/code.
Match the frozen renderer's outer trimming of comments and user lists; refuse
name/path trimming rather than silently changing a filesystem location.

Prefer a checked JSON importer over porting the legacy smb.conf parser: that
parser discards unsupported settings, global inheritance, includes and duplicate
sections. A full Samba parser would need a separate semantic qualification.
This delivery explicitly leaves smb.conf import, service health, accounts,
package setup and live share lifecycle open. A shell/service wrapper would also
cross the integrator's ownership boundary without supplying storage authority.

Reject unknown/duplicate JSON fields, coercions, sequence-shaped structs,
malformed/oversized input, reserved or case-colliding section names, unsafe
paths/comments and unsupported user expressions. Permit only simple local
usernames separated by commas; groups, domains, substitutions and whitespace-only
user separation become explicit findings. Rendering accepts only validated
types and emits enabled sections; disabled configuration remains in dependency
previews. No global Samba settings or arbitrary directives can be supplied.

Bound input to 64 KiB, 64 shares, 64-byte share names, 512-byte paths, 240-byte
comments and 32 local users of at most 32 bytes each. Refuse more than 512 expanded
device/route requirements for the whole preview; never truncate. Validate the
storage contract and existing pool plan, including empty share lists. Refuse
nested device roots or pool/device root overlaps that make lexical ownership
ambiguous. A share beneath a device maps directly; a share at/beneath a pool
maps to every configured member. Ancestor shares conservatively include both
direct devices and pools. Sort requirements by device ID and route, preserve
branch order, and explicitly list shares with no configured relationship.

These are configuration relationships. They cannot establish directory existence,
symlink or mount aliases, physical UUID/backing, namespace identity, service
availability, account permissions or effect-time freshness. Empty requirements
do not authorize unmount. The integrator must combine this input with protected
evidence, complete dependency claims, approvals, locks and recovery before writes.

Test frozen sample/imported configurations and all four pool/protection layouts,
safe golden rendering, disabled/read-only consumers, order/case/component/ancestor
relationships, unknown paths, invalid configuration, injection and import limits,
and whole-request expansion boundaries. Run fmt, Rust workspace tests, strict
Clippy, contract drift and repository checks. Builds/logs/general scratch stay
on disk; only the existing executor-storage tests use their small auto-cleaned
tmpfs fixtures. Return clean code and separate documentation/evidence commits.
DSK-001, MNT-001/MNT-002, RT-001, UI-003, P04 and cutover remain open; no installed
or Pi qualification is claimed.
