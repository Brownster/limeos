# ARM64 runner safety review

Go for source `a7c73459c70b0555aff5a4f366e13c3ea063ae7f`, including the preceding expiry and identity fixes `0bceac3555123020be88aea9f544432df99330fb`, `e0bb3fa930cd40ea89d6935461ff2c817aed7971` and `1ca6e89e29e390ead864eef25ac6a25eccb7340f`. No remaining blocker found for the assigned workstation/SSH-loss, expiry-dispatch and unrelated-process safety requirements.

The review used the committed source, not the unfinished working diff. Three source hashes and the final harness/independent-boundary log hashes are recorded in `/tmp/limeos-arm64-runner-independent-review.json`. At capture, all three working source files matched that committed source.

## Launch ownership and failure paths

`/home/marc/Documents/github/lime-os/tests/qualification/arm64/native_vm.py:467` creates a private root-owned control directory, installs read-only launcher/config copies, and compares both copied hashes against the local inputs before launch. Root Bash uses an empty environment and no startup profiles. Python uses an empty environment and `-I`.

At `native_vm.py:529`, one detached host operation starts the owner first with `setsid`, `nohup`, closed stdin and file output. Only that owner can start QEMU. QEMU runs in the foreground; its argv contains no sudo wrapper or daemonize option. The supervisor captures its own process identity, then owns the child and retained pidfd from birth (`guest_supervisor.py:116`). No second workstation command is needed to bind the guest, monitor its deadline or clean it up.

The helper validates finite ordered deadlines inside the authorization and checks the host clock again after prelaunch logging/state I/O, immediately before `Popen` (`guest_supervisor.py:141`). It refuses both `-daemonize` and `--daemonize`; QEMU treats those spellings equivalently ([QEMU 7.2 primary source](https://raw.githubusercontent.com/qemu/qemu/v7.2.0/softmmu/vl.c), `lookup_opt`).

After spawning, pidfd failure, identity timeout, exclusive state-publication failure and logging exceptions all enter the owned-child `finally` block (`guest_supervisor.py:186`). That block kills and reaps only the retained child. If pidfd creation failed, the fallback child PID remains unreaped and cannot be reused. A TERM logging exception also forces local cleanup rather than abandoning the guest. Successful binding enters the local TERM/KILL monitor; the existing guest-deadline calculation still places KILL before the authorization closes.

The real local disconnect regression starts a detached synthetic owner/child, kills the fake SSH parent before guest identity publication, and checks that the owner later performs TERM/KILL, reaps the child and exits. This proves the process-ownership mechanism locally; it is not a new remote QEMU qualification.

## Access and stop safety

All six command/copy methods share `dispatch_options` (`native_vm.py:184`). It refuses dispatch after expiry or with less than one second remaining and caps workstation transport timeouts to one second before cutoff, preserving stricter caller limits. Boot uploads and stop downloads have no raw SCP bypass. Black-box regressions expire the clock after the preceding preparation/stop command and assert that no SCP dispatch occurs.

Stop requires root-private recorded PID, start ticks, real UID and marker. It checks the expected guest marker, binds a pidfd before reading numeric-PID identity, and sends signals only through that pidfd. Exact argv-element comparison rejects marker substrings; stale start time/UID and PID reuse refuse or safely observe the original process gone. The stop path has no numeric-PID fallback (`native_vm.py:589`). It waits for the separately recorded owner without signalling that owner or unrelated processes before cleanup.

## Verification and scope

The implementer's final log `/tmp/limeos-arm64-runner-safety-tests.log` records **33 tests passed in 10.921 seconds**. I read the full added tests and verified that final output.

Independent checks loaded `guest_supervisor.py` from the exact Git blob into an isolated Python module. Three mocked boundary cases passed: advancing the clock across the authorization during prelaunch I/O creates no child; each daemonize spelling refuses before launch. Raw output: `/tmp/limeos-arm64-runner-independent-boundaries.log`. No real process, SSH or QEMU was started by these independent checks.

No Cargo/full validation, source edits, commits, Pi/SSH commands, frozen Python runtime operations or installed-service tests were performed by this reviewer. The integration worker owns full validation and publishing. The hardened runner still needs a future isolated-host native qualification; the historical overnight run retains its original source identity. Expiry timeouts bound workstation transports; arbitrary detached `host-exec` commands require their own host-side lifetime enforcement.
