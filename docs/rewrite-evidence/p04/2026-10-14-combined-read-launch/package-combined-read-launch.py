# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind the committed local launch checks to lossless logs and source hashes."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/combined-read-launch'
logs = root / '.cache/rewrite-combined-read-launch/accepted-logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-14-combined-read-launch'
qualified = 'fbd89657fc960f52349a1c1876ddd98678c50e0d'
names = ('test-rest-accepted', 'storage-compile-on-disk', 'storage-fixtures',
         'launch-focused', 'clippy-accepted', 'fmt-accepted',
         'contracts-accepted', 'repository-accepted', 'arm64-harness-accepted')
checks, raws = [], {}
for name in names:
    check = json.loads((logs / (name + '.json')).read_text())
    assert check['exit'] == 0 and check['qualified_source'] == qualified
    compressed = (logs / check['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest() == check['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == check['compressed_sha256']
    for path, digest in check['runtime_hashes'].items():
        committed = subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
        assert hashlib.sha256(committed).hexdigest() == digest
        assert hashlib.sha256((work / path).read_bytes()).hexdigest() == digest
    checks.append(check)
    raws[name] = raw
accepted = ('test-rest-accepted', 'storage-fixtures')
rust = sum(c['rust_cases'] for c in checks if c['name'] in accepted)
doctests = sum(int(n) for name in accepted for n in re.findall(
    rb'Doc-tests[^\n]+\n.*?test result: ok\. (\d+) passed', raws[name], re.S))
assert rust == 690 and doctests == 21
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness-accepted'])[1]) == 61
assert raws['arm64-harness-accepted'].rstrip().endswith(b'OK')
assert next(c['rust_cases'] for c in checks if c['name'] == 'launch-focused') == 4
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-14-read-worker-supervision/source-manifest.json').read_text())
files = set(previous['files']) | {
    'bins/executor/src/read_launch.rs', 'bins/executor/src/read_launch/tests.rs',
    'bins/executor/tests/combined_read_launch.rs', 'bins/executor/tests/support/combined_read_probe.rs',
}
bindings = {}
for path in sorted(files):
    raw = (work / path).read_bytes()
    assert raw == subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
assert len(bindings) == 320
out.mkdir()
for check in checks:
    shutil.copy2(logs / check['log'], out / check['log'])
(out / 'checks.json').write_text(json.dumps({
    'qualified_source': qualified, 'rust_cases': rust,
    'unit_integration_cases': rust - doctests, 'compile_fail_doctests': doctests,
    'new_regression_cases': 15, 'opt_in_feature': 'limeos-executor/read-worker-probe',
    'focused_cases_already_in_workspace_total': True, 'arm64_harness_cases': 61,
    'process_and_socket_fixtures_execution': 'approved local nonroot tests outside sandbox',
    'checks': checks,
}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({
    'qualified_source': qualified, 'files': bindings,
}, indent=2) + '\n')
scope = {
    'qualified_source': qualified, 'parent_main': '84adb4e',
    'surface': 'unregistered fixed-image launch library and separate opt-in combined probe',
    'fixed_image': '/usr/lib/limeos/limeos-combined-read-probe',
    'protected_policy': 'root-owned nonwritable held/fresh ancestors and single-link regular native ELF',
    'public_image_uid_root_clock_cap_selectors': False,
    'sealed_image_max_bytes': 8388608, 'copy_scratch_bytes': 65536,
    'seals': ['WRITE', 'GROW', 'SHRINK', 'SEAL'],
    'seals_and_digest_verified_in_each_role': True,
    'executable_memory_policy_bypassed': False,
    'dynamic_loader_and_libraries_pinned': False,
    'original_ancestor_identity_permissions_rechecked': True,
    'configuration_roles': ['contract', 'pools', 'protection', 'shares'],
    'configuration_path_bytes': 512, 'configuration_path_components': 16,
    'selection_establishes_protected_contents': False,
    'inherited_launch_environment_cleared': True,
    'launch_variables': 'sealed digest and four bounded paths; guardian adds its own worker PID binding',
    'deadline_seconds': 4, 'preparation_deadline_preserved_in_monitor': True,
    'image_lease_owned_by_monitor_until_cleanup': True,
    'worker_exec_uses_live_guardian_descriptor': True,
    'event_bytes': 270336, 'report_bytes': 262144,
    'whole_final_event_and_successful_exit_required': True,
    'test_feature': 'read-worker-probe; disabled by default; never packaged',
    'legacy_standalone_combined_case': 'blocked pending external adoption of the separate seam',
    'real_local_facts': ['sealed owned ELF survives source mutation and replacement',
                         'write/grow/shrink/additional-seal attempts refuse',
                         'stripped owned helper guardian/worker reaches genuine nonroot configuration guard',
                         'caller image drop does not break supervised exec',
                         'original shorter preparation deadline wins over later unconfirmed child exit',
                         'on-disk internal roles with valid digest/paths refuse at the image seal guard',
                         'no-host-I/O contract produces one complete final event'],
    'private_unit_facts': ['owned-root policy without a public owner override',
                           'original directory permissions refuse changed but still-protected directories',
                           'symlink, writable, hardlinked, wrong-owner, malformed and oversized images refuse',
                           'closed launch variables and expired preparation',
                           'noncanonical, duplicate and amplified configuration names refuse'],
    'test_image_generation': 'owned debug ELF copy stripped by local /usr/bin/strip; no installed artifact qualified',
    'storage_authority_transferred': False, 'public_root_positive_launch': 'unqualified',
    'genuine_root_positive_collection': 'unqualified', 'installed_confinement': 'unqualified',
    'independent_review': 'pending',
    'pi_accessed': False, 'ssh_used': False, 'docker_accessed': False,
    'raw_block_probe_used': False, 'guest_launched': False,
    'reserved_backup_touched': False, 'external_qualification_paths_touched': False,
    'service_guards_changed': False, 'frozen_signed_arm64_candidate_contains_slice': False,
    'remaining': ['external adoption of the separate opt-in combined probe',
                  'root/capability/foreign-namespace positives',
                  'installed memfd/noexec, proc/fd and syscall compatibility',
                  'dynamic loader/library trust', 'blocked source preparation/exec termination',
                  'guardian SIGKILL cgroup teardown', 'unkillable-task cleanup latency',
                  'strict scheduler latency', 'whole-tree peak memory/FD/task headroom',
                  'complete physical consumers/claims', 'actual pool/share state', 'locks/effects/recovery'],
    'historical_inventory_p95_ms': 61.401, 'inventory_p95_target_ms': 20,
    'performance_gate_closed': False, 'p04_closed': False, 'defect_rows_closed': [],
}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
for name in ('run-combined-read-launch-check.py', 'package-combined-read-launch.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures')
(out / 'README.md').write_text(f"""# Protected combined-reader launch: local acceptance

Qualified runtime `{qualified}`, based on local main `84adb4e`.
The unregistered launcher selects one fixed protected image and four bounded
configuration names. A bounded native ELF copy is sealed in a memfd; the monitor
owns its lease through cleanup and preserves the original preparation deadline.
Each role verifies its own sealed executable. Worker exec uses the live guardian's
descriptor, independently of the manager's numeric PID. See the
[design](../../../plans/2026-10-14-combined-read-launch-design.md) and
[interface guide](../../../p04-combined-read-launch.md).

**690 Rust cases pass:** 669 unit/integration cases and 21 compile-fail doctests.
The 15 new cases and four-case focused rerun are already in that total.
Workspace tests enable `limeos-executor/read-worker-probe`; both fixture binaries
remain disabled by default and absent from the explicit packaging list.
**61 local ARM64 harness tests pass**, plus strict all-target Clippy, fmt,
contract and repository checks. [checks.json](checks.json) records commands,
environments, runtime hashes and complete merged stdout/stderr transcripts.
Documentation changes during validation altered no runtime source.

Real owned ELF tests prove active WRITE/GROW/SHRINK/SEAL seals, unchanged captured
bytes after source mutation/name replacement, native-format/size admission and
successful execution of the immutable copy. Private protected-walk fixtures
refuse wrong owner, writable or hardlinked files, symlink leaves/ancestors,
directory leaves and unprotected roots. Held and fresh directory checks compare
the original identity/permissions, including changed permissions that remain
nonwritable. These private owned-root tests qualify no public root-positive path.

A stripped owned copy of the opt-in helper passes sealed guardian and worker
startup, resource/bootstrap admission and the genuine public nonroot configuration
guard. An earlier image/bootstrap/budget error cannot pass that expected refusal.
Dropping the caller's image still permits exec because the monitor owns the lease.
An owned sleeper regression proves the original shorter preparation deadline
reaches the ticket/monitor without renewal and waits for fixture exit/lease drop.
On-disk internal roles with a valid digest and all four valid paths refuse at the
current-image seal guard. Contract mode emits exactly one
complete final blocked event without host I/O. Configuration names cannot be
missing, duplicated, noncanonical or amplified beyond their closed limits.

The separate supplied combined probe remains unregistered. Its final event is
built within 264 KiB before output, explicitly flushed, and requires successful
exit plus one complete final event. Successful observations require complete
supervised report transport and confirmed cleanup. Image metadata, selected names
and report JSON supply no storage-effect authority. The existing standalone
probe's combined case stays blocked pending external adoption of this new seam.
The implementing agent reviewed the slice; independent review remains pending.

The sealed copy pins executable bytes, not dynamic loader/library contents. Memfd
pages add up to 8 MiB per image to the whole tree; whole-tree peak headroom is
unqualified. Installed memfd/noexec, proc/fd, syscall, capability and namespace
compatibility remain open. No executable-memory policy is bypassed. Source
preparation and exec can block in kernel work; their termination/latency is not
qualified. Guardian SIGKILL and unkillable tasks retain the supervision limits.
Service guards remain 256 descriptors, 64 MiB, zero swap and 16 tasks. Historical
inventory p95 61.401 ms remains above the 20 ms target. No footprint/performance,
P04, defect or cutover gate closes. The frozen signed ARM64 candidate excludes
this slice; no installed artifact or positive root/Docker/storage composition
is qualified here.

Build/cache/general scratch stayed on disk, including debug ELF copies stripped
by local binutils for the owned launch fixture. Existing storage fixtures were
compiled there before small owned, auto-cleaned tmpfs runtime scratch. Their
50 ms samples observed lower-bound peaks of {peak['bytes']} bytes and
{peak['files']} files, not exact maxima. No Pi, SSH, Docker, raw block probe or
guest launch was used. Reserved backup and external qualification paths stayed
untouched.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source, fixture,
contract and script files to the committed runtime. Fixed-timestamp gzip preserves
full log bytes with raw/compressed hashes; [artifact-sha256.json](artifact-sha256.json)
binds the packet. Reproduce with fresh disk logs using the retained helpers;
compile storage fixtures before their tmpfs runtime run.

Next is independent review and external adoption/installed qualification of the
supplied combined seam. Complete physical consumers/claims, actual pool/share
state, locks, effects and recovery remain open.
""")
entries = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in sorted(out.rglob('*')) if p.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({
    'qualified_source': qualified, 'files': entries,
}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings),
                  'rust_cases': rust, 'harness_cases': 61, 'artifact_files': len(entries)}))
