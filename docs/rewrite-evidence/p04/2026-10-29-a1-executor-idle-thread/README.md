# Installed A1 refusal caused by an executor thread change

The fresh diagnostic candidate uses source
`c2246af4689ceafd8b4a42c9d3533b4497ccd100`, authority schema 12 and standard/shadow
fixture packages `0.4.7+scan.1`. The offline Debian build, current UI build and
signed repository preparation passed with unchanged source inputs. All 687
recorded product inputs and the installed binary/package hashes join this source.

This guest passed the initial census and all three negative pre-effect API
cases: extra path rejection, tampered approval digest rejection and cancelled
approved-plan rejection. It then created the positive human plan but refused
its approval with HTTP 409 at stage `human_plan_approval_queue`. No mount job
action was recorded and no approved mount acceptance is claimed.

The exact compared task vectors identify one removed task and no additions:
PID **7189**, TID **7190**, start **22303**, mount namespace
`(4, 4026531841)`. PID 7189 is the installed mount executor. Both complete
independent snapshots surrounding the earlier negative API cases contain this
exact identity in `/system.slice/limeos-storage-mounts.service`. The failed
comparison itself records its disappearance at `scan.recheck.user_tasks`.
The snapshots do not reconstruct the failed scan or prove the original October
27 initial-census refusal's cause.

The service starts at 20:40:31 and the refusal is logged at 20:40:41. Its actual
source explicitly starts a no-op Tokio blocking worker once at mount-service
bootstrap. The pinned Tokio 1.52.3 source uses a ten-second default blocking
worker idle timeout. This strongly supports retirement of that unnecessary
startup worker as this reproduction's cause. Confirm the bootstrap behavior
with a native regression before correcting it. Keep every host-task comparison
and the original four-second admission window.

The tool session disappeared before the outer guard wrote its terminal fields.
The runner continued and retained the guest failure packet and traceback, but
the original guard record has **no terminal exit code, ending timestamp or
final source snapshot**. Preserve it unchanged. A recovery watcher found the
recorded runner already absent; a separate native process observation also
found its recorded VM and SSH child absent. The disposable scratch directory
was removed. Source bytes were checked against the candidate again at archive
time. These observations cannot establish continuous memory supervision or
supply the missing terminal record, so this run is **diagnostic evidence only**.

`a1-failure.json` contains the bounded guest packet, including the exact service
identity, witnesses, completed negative cases and unit diagnostics. Raw guard
records/output, build records, package and fixture inputs, startup diagnostics,
the recovery observation and the source join are retained. All 31 guest modules
and both runner modules are preserved in `guest-and-runner-sources.tar.gz`.
`harness/` retains the exact builders, preparers, launcher, guard, watcher and
archiver. Private signing keys, binaries, packages and VM disks are excluded.
`SHA256SUMS` covers every other packet file.

The next candidate must qualify the runtime correction in a fresh guest with
continuous supervision. Genuine approved mount, independent verification,
replay/restart, installed UI, upgrade, ARM64 and broader A/B runtime acceptance
remain open. Holly's production Pi was untouched and nothing was pushed.
