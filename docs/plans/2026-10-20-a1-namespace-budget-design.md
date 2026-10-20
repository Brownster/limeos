# A1 installed namespace admission

The installed A1 mount scanner refuses the ordinary disposable Debian host before
any mount effect. Hosted run `39429582703` on source `477b81f` reported
`scan.namespace.limit`, observed nine, limit eight. A local repetition used those
exact CI packages and the identical Debian image SHA512, adding only a bounded
post-refusal diagnostic census. It reproduced the same refusal. The later census
observed nine namespaces with no read/membership errors: the host namespace,
udevd, resolved, timesyncd, networkd, logind, and LimeOS storaged, containerd and
Core. The genuine Engine declared no containers. All existing service sandboxes
remained active. Census facts are diagnostics, never failed-scan or admission
authority. Inputs and raw failed output remain in `.cache/a1-namespace-review/`
and `.cache/main-consolidation/attempts/a1-namespace-baseline/`.

The eight-namespace profile cannot admit the default installed services, even
without containers. Increasing the service descriptor limit would change the
confinement contract. Stopping service sandboxes would qualify a different host.
Instead, specialize the configuration resource reservation to A1's actual fixed
paths, preserving its configuration content limits and every protected read.

## Resource contract

Add a trusted-code `InstalledMountV1` configuration profile with a 13-FD ceiling.
The shared root plus four `/etc/limeos/<file>` paths require exactly
`1 + 4 * 3 = 13` retained descriptors. The collector must reject deeper aggregate
path requirements before opening the host root or any configuration file. Keep
the existing 16-KiB file, 32-KiB aggregate and 12-KiB report limits. The general
`CombinedV1` profile remains at 33 FDs. Profiles and retained owners remain
non-deserializable; choosing a smaller resource bound grants no trust.

Use this profile only in A1's fixed selection. Raise its complete host namespace
capacity to sixteen. Export the scanner's static namespace bound so the central
reservation derives its cost from the same contract. Its retained cost is one
proc-root handle plus sixteen namespace handles. The total becomes:

`baseline16 + configuration13 + Engine1 + processes21 + physical75 + destinations16 + target8 + host-consumers17 + guardian14 + transient32 = 213`.

This fits the unchanged 226 admission model and 256 service ceiling. During host
revalidation the old namespace handles remain live while at most sixteen fresh
handles are retained; those fresh handles and per-task traversal use the existing
32-FD transient allowance. Whole-service 64-MiB/16-task limits, four-second lease,
four-container/sixteen-destination profile, all-task completeness, unknown-input
refusal, retained namespace identity, physical/configuration/Engine checks,
single-flight and poisoned recovery barriers remain in force. This does not
establish full-host parity or measured peak-resource acceptance.

## Implementation and qualification

Test actual installed-path preflight and retained descriptor counts, deeper-path
refusal, unchanged general profile/content limits and source-bound central
reservation consistency. Exercise the namespace insertion boundary using owned
handles: sixteen distinct handles retained, duplicate identity reuses its anchor,
and the seventeenth identity refuses before reading another mount table. Preserve
the existing mount/underlay alias and kernel-flag ABI regressions.

Compile and run focused native checks with one Cargo job and the 3-GiB available
memory guard. Build a Debian-compatible executor from the exact committed source
in the existing pinned local builder, with work and compiler scratch on disk.
Rerun the real installed API/Core/root-service fixture on disposable disks,
including its existing pre-effect refusals, review/approval, actual effect,
independent verification, replay and restart assertions. Preserve every failed
attempt and identify the next actual refusal if one remains. Do not substitute a
mock owner, clear authority databases, retry uncertain effects or weaken fixture
assertions to obtain success. Broader native ARM64, installed/resource/crash and
P04/P05/P06 cutover gates require their own evidence. No production Pi window or
push is authorized by this work.
