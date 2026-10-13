# Current Pi run stopped by the memory safeguard

**Qualification is incomplete.** The current host-owned deadline proof passed,
but both native builds hit the host swap safeguard. No current candidate package
was built on the Pi. Install, genuine schema-8 upgrade, interruption/recovery and
footprint tests did not run. [Machine-readable status](qualification-status.json)
lists the reached and untested gates.

The last post-test health check at **2026-10-13 00:17:48 Europe/London** showed the
original 14 containers running, the frozen dashboard/helper, Docker and Samba
with unchanged PIDs/restart counts, both HTTP checks returning 200, no new OOM
kill and no QEMU or guest-supervisor process. All three owned guests and their
protected control directories were removed before the 07:00 cutoff. The approved
QEMU/firmware prerequisites and verified base image remain installed and idle.
No Rust package was installed on the host, and no production Docker mutation,
service restart, mount or media-write command was issued.

## Inputs and authorization

| Input | Identity |
| --- | --- |
| Attempted runtime and guest fixtures | `016ba49a27b228576db5d4ca28dcd122c60a6278` |
| Attempted package/schema | `0.4.8+arm64.2` / 9 |
| Workstation orchestrator | Main `af94f19ac21f9da14b207439808ffb33c87e8e10`; unchanged tracked runner |
| Bundle SHA-256 | `fc746b8e564c015ce7bf6757ad60d833b5fd6cb88ff028c376bef3109c9414a3` |
| Manifest SHA-256 | `bee6bf0742de9b7e55fd3b9bcaeb82ac62c5cac33250360e53b88cade18cfd4c` |
| Corrected SSH target | `holly@192.168.0.45` (`wybie`, Pi 5, 8 GiB RAM, 16 KiB host pages) |
| Authorized window | October 12 22:44:28–October 13 07:00 Europe/London; 21:44:28–06:00 UTC |
| Supervised long-guest deadline | 06:30 BST SIGTERM; 06:32 BST SIGKILL if necessary |
| Genuine previous artifact | Source `8acac403e32d3692b028a036a11f4fead2050580`, `0.4.7+arm64.1`, schema 8; unchanged SHA `6c86199b253e155c94d96e1395c069c4188e11a2379f14eff97eacab0203b898` |

The initial `.46` connection failed with no route before executing any remote
command. The operator corrected the address and separately approved the reviewed
QEMU prerequisites, then allowed retaining them dormant for future testing.
[Authorization and decision records](inputs/authorization-pi45.json) preserve
that scope. The package delta is exactly 41 additions and zero changes to the
717 preexisting entries. [Before](inputs/preparation-before.json),
[after](inputs/preparation-after.json) and [install transcript](inputs/preparation-install.txt)
retain the actual simulation, package versions and manual selections. The image
matches Debian's downloaded [SHA512SUMS](inputs/debian-SHA512SUMS), including the
same SHA-512 rechecked on the host before boot.

## Deadline and build attempts

The detached root-owned supervisor had PID 1 as its parent and owned its exact
foreground QEMU child, dropped to UID 1000. Protected launcher hashes, markers,
UIDs and kernel start ticks were retained. At 23:55:00 BST it sent SIGTERM; the
guest exited approximately 33 ms later. Both owner and child disappeared, and
the unrelated frozen application's PID/start ticks were unchanged. SIGKILL was
not needed. [Proof](inputs/deadline-proof-result.json) and
[supervisor log](deadline-proof/supervisor.log) establish the current native
deadline path independently of the historical run and local mocks.

| Attempt | Guest allocation | Reached | Stop and post-stop health |
| --- | --- | --- | --- |
| 1 | 2 CPUs, 1536 MiB; 512 MiB swap on the guest's disk | Guest prerequisites passed; subsequent setup interrupted | 498 MiB host swap exceeded the 450 MiB guard. Stopped at 23:59:28 BST; afterwards 5928 MiB available RAM and 437 MiB swap |
| 2 | 1 CPU, 512 MiB; 2048 MiB swap on the guest's disk | Prerequisites, Rust 1.88, locked fetch, fmt and strict Clippy passed; workspace tests incomplete | 496 MiB host swap exceeded the unchanged guard. Stopped at 00:17:46 BST; afterwards 6005 MiB available RAM and 449 MiB swap |

The guest swap files never changed host swap configuration. Both allocations
were below the runbook ceilings, at configured nice 19 and idle I/O priority.
The coordinator also checked minimum available RAM, temperature, OOM count,
production services, original running containers and HTTP health every minute.
Neither run reached its scheduled deadline; both were stopped by the guard.
The final [host sample](attempt-2/host-samples/014.json) verifies teardown health;
the runner's files named `host-after.json` are sampled **before** orderly stop.

Swap occupancy is the observed trigger, not proof that the Pi exhausted RAM.
Available RAM remained above 4 GiB, temperatures stayed below 59°C, and the
recorded production health requests succeeded. The kernel lacks PSI memory
counters, and `/dev/vcio` was unavailable; no claim of comprehensive stall or
undervoltage measurement is made. The precise reclaim cause is unproven.

At baseline there was already about 300 MiB in swap, one failed
`docker-compose-start.service`, and four stopped containers: `airsonic-advanced`,
`limeos-mattermost-db`, `limeos-alertd` and `limeos-mattermost`. Those conditions
were not repaired or counted as test failures. Swap was still higher than
baseline after the tests; it was not cleared by restarting services or changing
host memory policy.

## Evidence limits and expiry

Safety teardown removed each overlay before the collection stage. Detailed
guest test/build files and partial `build-result.json` were not pulled. The
complete available coordinator stdout, host samples, console, launcher metadata
and supervisor logs are retained, including failures. The first local proof
recorder tried to parse a command transcript with a multiline command prefix;
the corrected recorder reads the structured host-command ledger instead. It
validates the same original observations without changing the launcher.

A delayed progress-snapshot invocation started at 08:49:30 BST, after the window.
The existing guard refused it before SSH. It neither resumed a guest nor accessed
the host. No current daytime host-health claim is made from the earlier sample.
Any new host dispatch requires its own authorized window.

The successful native-run recorder was deliberately not used: its complete-build
prerequisite did not pass. Private SSH keys, seed images and guest login material
are excluded from this packet. [Evidence hashes](evidence-sha256.json) cover every
packet file except the hash index itself.

## Next inputs prepared off the Pi

[Main CI](https://github.com/Brownster/limeos/actions/runs/38302065106) passed native
ARM64/AMD64 source and package checks plus the disposable AMD64 suites. The
initial AMD64 writer-lock conflict and passing retry remain separate CI history;
its root cause is unproven. These results do not qualify installed Pi behavior.

The original native ARM64 artifact was downloaded locally and its GitHub ZIP
digest, package controls, four AArch64 ELF headers and binary hashes were checked.
[Original CI inputs](next-inputs/native-ci/verified-inputs.json) bind source
`af94f19`, job `114997846707` and artifact `11770788524`. All 290 source, fixture,
contract and script inputs in the earlier `016ba49` integration manifest match
that frozen CI source; the actual CI identity remains `af94f19`.

The existing CI labels `0.4.4` and `0.4.4+ci.1` are older than the genuine previous
`0.4.7+arm64.1`. They are not counted as historical artifacts or an upgrade.
Fresh standard/shadow **`0.4.8+arm64.3`** packages were assembled locally with the
frozen `af94f19` builder and unchanged native CI ELFs/UI. Controls, generated
contracts and every packaged ELF hash were verified. Neither original CI
packages nor the genuine previous package changed. The [candidate provenance](next-inputs/candidate/candidate-provenance.json)
records actual CI compilation separately from local packaging and pending guest
qualification. Packages remain on disk under
`.cache/arm64-qual/integrated-2026-10-12/ci-candidate-0.4.8-arm64.3/`.

Next, support this explicit CI provenance in the qualification fixtures and
prepare a fresh signed test repository. Preserve the native-guest proof path;
never manufacture its build steps to admit CI packages. Use a new window for
install/footprint, genuine upgrade and recovery guests, with heavy compilation
kept off the in-use Pi. P04, storage/restore defects and production cutover remain
open. The [basic cutover checklist](../../../plans/2026-10-12-basic-cutover-readiness.md)
records the operator's preference for a smaller first release.
