# P01: build, install and recover the Rust foundation

P01 provides authentication, policy and durable authority state. The two executors accept only a health request. Dashboard observations start in P02; container and host effects start in P03/P04. This package is a test-host release, not a replacement for the deployed Python application. See the [execution evidence](rewrite-evidence/p01/2026-10-04-execution-tracker.md) for qualification limits.

## Build and check

The production executables are Rust. React/TypeScript provides the browser interface. Python's standard library is used only to build packages and run development tests; Python is not a package dependency. The container below is a disposable build environment, not an application distribution.

```sh
npm --prefix frontend ci --ignore-scripts
npm --prefix frontend test
npm --prefix frontend run build

docker build -t limeos-builder -f packaging/Containerfile .
docker run --rm -v "$PWD:/build" limeos-builder cargo build --locked --release -p limeos-core -p limeos-executor -p limeosctl
docker run --rm -v "$PWD:/build" -e CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc limeos-builder cargo build --locked --release --target aarch64-unknown-linux-gnu -p limeos-core -p limeos-executor -p limeosctl
python3 packaging/build.py --arch amd64 --binaries target/release
python3 packaging/build.py --arch arm64 --binaries target/aarch64-unknown-linux-gnu/release
```

Use Rust 1.88.0 and the checked-in lockfile for development checks. The workspace forbids unsafe code in first-party Rust. Development builds optimize the password algorithms so real authentication deadlines remain testable.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
# Deferred code (docs/deferred-code.md) is checked separately:
cargo clippy --workspace --all-targets --locked --features deferred -- -D warnings
cargo test --workspace --locked --features deferred
cargo deny check
cargo audit
shellcheck packaging/debian/postinst packaging/debian/prerm packaging/debian/postrm packaging/repository.sh
python3 scripts/check_repository.py
python3 scripts/check_contracts.py
python3 scripts/test_gates.py
npm --prefix frontend audit --audit-level=high
```

CI runs these checks on native x86-64 and ARM64 runners and validates the shipped units with `systemd-analyze verify`. It also installs, upgrades, downgrades and removes the package in a disposable Debian VM. The repository is initialized locally with committed lockfiles. Native ARM64 and remote CI results remain to be obtained after an operator connects its remote.

## Sign and install

The repository builder requires an existing signing key. Manage the production key separately and verify the exported public key's fingerprint through a trusted channel. Test evidence uses a disposable key, never a production key.

```sh
packaging/repository.sh "$PWD/dist/apt" YOUR_SIGNING_KEY_FINGERPRINT dist/*.deb
```

Publish `dist/apt` through the chosen HTTPS apt host. Install the verified exported key as `/usr/share/keyrings/limeos-archive-keyring.gpg`, owned by root with mode 0644. Add this source using the operator's actual repository URL:

```text
deb [signed-by=/usr/share/keyrings/limeos-archive-keyring.gpg] https://YOUR_APT_HOST/ stable main
```

Then install on the test host:

```sh
sudo apt-get update
sudo apt-get install limeos
sudo /usr/lib/limeos/limeosctl status
```

Apt verifies signed metadata and SHA256 package hashes. Repository metadata expires after seven days; regenerate and sign it before that deadline. Package payloads belong to root. The installer refuses existing human/root accounts under service names, corrupt configuration, corrupt ceilings and symlinked managed paths. Account identities and ceilings survive reinstall.

## HTTPS and initial enrollment

Core listens on `127.0.0.1:8003`. Browser authentication requires HTTPS through a local proxy. Install and configure a proxy separately; [`packaging/Caddyfile.example`](../packaging/Caddyfile.example) serves the packaged UI and proxies `/api/`. Its local certificate authority must be trusted in each browser. For a host name other than `localhost`, change both the proxy address and `/etc/limeos/core.json` to the same exact HTTPS origin, including a non-default port. Validate before restarting:

```sh
sudo /usr/lib/limeos/limeosctl check-config /etc/limeos/core.json
sudo systemctl restart limeos-core
```

Do not expose the cleartext loopback service through a port forward. Core ignores forwarded identity and client-IP headers. Every request arrives from the proxy, so the login quota does not use the peer address. Each username, existing or not, gets its own ten failed password attempts per minute (429 `overloaded` after that); no two names share a budget. Requests that cannot be a login do not count: a malformed body, a username that is not 1–64 characters of letters, digits and `_-.:`, or an empty or over-1024-byte password. A correct password does not count, even if the login then fails.

Names with no account also share one budget of 120 password comparisons a minute, so a flood of made-up names cannot keep both password workers busy. An existing account never draws on it: failures against other names cannot lock out a real account, though they can make a name with no account answer 429. While that budget lasts, a name with no account is compared against a dummy hash and answers exactly like a wrong password. Once it is spent, such a name gets 429 without a comparison until the minute ends. This is a recorded decision: while someone is flooding the login form, the difference reveals which names do not exist; on a household appliance that is preferable to letting the flood lock out the owner. A login that finds both password workers busy waits up to two seconds, in arrival order and with at most eight waiting, then gets 429; that costs the name nothing, but for a name with no account it still uses the shared budget. At most 2,048 names hold a live budget at once; if that table were ever full, an unseen name would get 429 until entries expire.

Anyone who can reach the login page can lock one named account by sending ten wrong passwords a minute for it, and keep it locked while they continue; this is inherent to a per-name limit that cannot tell the owner from an attacker. The lockout ends one minute after the failures stop. To recover during an attack, stop the source at the proxy or firewall (core only sees the proxy's address). Restarting `limeos-core` clears every budget at once but does not stop an attacker who continues.

There is no default user or password. As local root, issue a bootstrap token:

```sh
sudo /usr/lib/limeos/limeosctl bootstrap
```

The returned token is a secret, expires in fifteen minutes and is replaced by a new issuance. Create a root-readable file with mode 0600 containing this JSON, using a new password of 12–1024 bytes:

```json
{"token":"TOKEN_FROM_BOOTSTRAP","username":"YOUR_ADMIN_NAME","password":"YOUR_NEW_PASSWORD"}
```

Supply the file through standard input, then remove it:

```sh
sudo sh -c '/usr/lib/limeos/limeosctl enroll < /root/limeos-enrollment.json'
sudo rm /root/limeos-enrollment.json
```

User names contain at most 64 ASCII letters, digits or `_-.:`. Enrollment is local root only and succeeds once. Credentials never go in command arguments or environment variables. Sign in through the HTTPS UI after enrollment. An authenticated browser receives a Secure, HttpOnly, SameSite=Strict host cookie and an in-memory CSRF token. Logout requires the exact origin and CSRF token. Sessions expire after eight hours without sliding renewal.

## Ownership and authority

| Component | Identity | Owned writable paths | Permitted access |
|---|---|---|---|
| Core | `limeos-core`, primary group `limeos-rpc` | `/var/lib/limeos/core` (0700), `/run/limeos-core` (0750) | Authority DB, executor socket access groups; no Docker group |
| Password worker | A short-lived child of core | No state files | Bounded password IPC over inherited pipes; same systemd limits as core |
| Container executor | `limeos-containerd`, primary group `limeos-container-access` | `/run/limeos-containerd` (0750) | Docker group; health only in P01 |
| Host executor | `limeos-storaged`, primary group `limeos-host-access` | `/run/limeos-storaged` (0750) | Root-owned policy; host observation and health, no capabilities, private network namespace; no Docker group |
| Reserved assistant | `limeos-assistant` | `/var/lib/limeos/assistant` (0700) | No Docker or executor access; no P01 service |
| Packaged code/UI/contracts | root | `/usr/lib/limeos` | Services cannot modify it |
| Independent ceilings | root (`host.json` group `limeos-host-access`, 0640, reset on every configure) | `/etc/limeos/system-policy` | Exact core UID, protocol version and allow-health flag |

Every configure sets `host.json` to `root:limeos-host-access 0640`, even one an operator had tightened to 0600: an older package created it `root:root`, and the host executor now runs unprivileged as `limeos-storaged` (primary group `limeos-host-access`), which must read it at startup. The file holds no secret, only a format version, core's UID, boolean allow flags and (unused by the host role) lists of managed container ids and stack names, so the wider read costs nothing confidential. It is now readable by root, the host executor and core (which holds `limeos-host-access` as a supplementary group for the host socket), and by no one else; it stays writable only by root, so neither service can change its own ceiling. A fresh install writes it 0644 and narrows it to 0640 in the same configure. `container.json` and `storage.json` keep their owners and modes.

Each socket uses mode 0660 and a distinct access group. Executors inspect kernel peer credentials before allocating a request frame. Having the socket's group does not substitute for the configured core UID. The root ceiling is independent of core's user policy and is read on executor startup; restart the affected executor after an operator changes it. Protocols accept no shell, executable, arbitrary file content or caller-provided principal.

Only core owns the SQLite authority database. Task tokens store digests and bind principal revision, service UID, task ID, exact scopes, expiry and core generation. Changing a grant invalidates earlier sessions and task authority. Restart invalidates task authority; browser sessions remain durable. P01 supplies the tested storage primitives for future job dispatch and provider budgets; it exposes no task-minting or effectful operation API.

## Persistence and recovery

Contracts and the authority schema are version 1. The schema is generated under `contracts/generated`; Rust types are authoritative. SQLite 3.53.2 is bundled; startup refuses versions older than 3.51.3. One bounded worker serializes transactions and holds the single-core file lock. WAL uses FULL synchronization. Job intent and its event commit together. Reusing an idempotency key with changed intent fails. Resource locks survive ambiguous interruption.

At startup, running/verifying jobs become `needs_intervention`. Core never retries them automatically. Receipt-based reconciliation arrives with executor effects in P03. Unknown provider usage keeps the maximum reservation; settlement is unique and cannot exceed it. Cost units are integers, not floating-point currency.

Writes fail closed below an 8 MiB free-space reserve or at the 64 MiB audit allowance. Logout and task revocation still go through both, and startup recovery may record its events in a 1 MiB reserve past the allowance. Read requests do not renew sessions, write timestamps or rewrite configuration. Core prunes settled audit history oldest first, at most once a minute and 1,024 rows per transaction, down to 32 MiB; open work, intervention history and automation proofs are never pruned. Authority events (bootstrap, enrolment, legacy import, grant revision, resolution requests and withdrawals) are kept apart: bulk history never evicts them, and only the newest 4,096 are kept, not counting requests and withdrawals of a job whose history is still kept, which stay with that history. If a restart finds the reserve full, startup prunes before recording recovery (settled history first, then authority events down to their own budget) and refuses only when nothing more can be pruned. Core logs a warning at 75% of the allowance, an error at the allowance, and an error for each failed pruning batch, which is retried a minute later. There is no supported archival yet; do not delete or hand-edit database rows to clear the allowance. A future schema causes startup refusal instead of an automatic downgrade. P01's 0.1.0/0.1.1 upgrade and downgrade tests use the same schema; they do not establish compatibility across later schema versions.

Upgrading to schema 17 builds three indexes over the audit table inside the startup transaction. Core estimates the free space this needs in the state directory (`/var/lib/limeos`) from the audit table itself: the indexes' entries (measured key sizes plus 25 bytes each) with a third added for page overhead, counted twice because the new pages are written to the WAL and then to the database, above the 8 MiB reserve that must remain afterwards, plus a fixed 256 KiB for the index roots and write-ahead log that even an empty table needs. That is about 170 bytes per job-less session event and about 340 per job transition: about 51 MiB for a full allowance of session events and about 93 MiB for a full allowance of job transitions. With less, core refuses to start before writing anything and logs `the database schema upgrade needs more free space` with the bytes needed and available; free space and restart. If the build runs out of space all the same, the upgrade rolls back and core logs the same message, asking for another estimate's worth on top of what was free. The index sort runs in memory for the upgrade, not in temporary files: at most about 31 MiB, within core's 128 MiB memory limit, for a full allowance whose every event has a job. Only a table far past the allowance (over 32 MiB of sort) sorts in temporary files in core's private `/tmp` (`PrivateTmp=yes`), and the estimate then adds twice that sort.

Upgrading to schema 18 adds one nullable column to the one-row `meta` table (`executor_epoch`, the container executor's receipt-store epoch core last saw; ADR 0003) and needs no space estimate beyond the write reserve. The first epoch core records after the upgrade marks every container and stack job still held across it (`executor_store_changed`): each is settled from its receipt if the executor still has one, and otherwise waits for an operator rather than being released on a closure. Jobs dispatched after the upgrade are dispatched under a known epoch and are not affected.

For a configuration failure:

```sh
sudo /usr/lib/limeos/limeosctl check-config /etc/limeos/core.json
sudo journalctl -u limeos-core -u limeos-containerd -u limeos-storaged
# Repair the reported configuration or space problem, preserving the original.
sudo dpkg --configure limeos
sudo /usr/lib/limeos/limeosctl status
sudo dpkg --verify limeos
```

The installer preserves corrupt files and exits with a repair instruction. Activation and removal errors surface to dpkg. Removal stops core, then its executors, then the optional storage units, and fails, naming the unit and its pids, if any process survives a stop or systemd cannot say; nothing is killed. Removal and purge then drop the three services' enablement links and reload systemd. Both can be repeated and tolerate units that are already gone or were never enabled. Neither deletes `/var/lib/limeos` (authority, assistant state, executor receipts and journals), the ceilings or any other file under `/etc/limeos`, drop-ins under `/etc/systemd/system`, or the service accounts and groups; purge loses only `/etc/limeos/core.json`, which dpkg deletes as a conffile. Reinstall a compatible package to recover. An upgrade restarts the three services and starts the storage reader again if it was running; the storage target and mount services stay stopped until started by hand. An upgrade from a package built before this behaviour cannot know the reader was running: start it by hand once.

A failed upgrade from such a package can also leave every service stopped: its pre-removal script stops them, and when the upgrade then aborts, its post-installation script does not start them again. The new package's `failed-upgrade` step refuses only when a process survives a stop or systemd cannot answer. After such a failure, start the services by hand (the reader only if it was running before):

```sh
sudo dpkg --audit
sudo systemctl start limeos-storaged.service limeos-containerd.service limeos-core.service
sudo systemctl start limeos-storage-reader.service
sudo /usr/lib/limeos/limeosctl status
```

Between packages that both have this behaviour, an aborted upgrade, removal or deconfiguration starts again the core services and the reader if it stopped them, except units masked or disabled meanwhile. The record of what to start lives in `/run/limeos-package` and is used once, by the step that ends the package change. Online backup, encrypted restore and the production updater are P04 work. Never copy an active SQLite database file without its WAL as a backup.

## Limits and verification

| Boundary | P01 limit |
|---|---|
| DB admission | 64 queued closures; reject saturation |
| HTTP | 64 connections, 32 headers, 16 KiB parser buffer, 8 KiB application headers, 16 KiB login body |
| HTTP deadlines | 5 seconds for headers, 10 seconds for a handler, 30 seconds per connection |
| RPC | 64 KiB frame, 5 seconds; core 16 connections, each executor 8 |
| Passwords | Two child processes, 8 seconds, 16 KiB input, 2 KiB output; child killed on cancellation |
| Hash policy | Argon2id 19 MiB/two passes/one lane; imported Werkzeug scrypt/PBKDF2 parameters are capped |
| Sessions/tasks | 8,192 global records, sixteen sessions per user; task expiry at most one hour |
| Services | Core 128 MiB/32 tasks; each executor 32 MiB; no swap or core dumps |

The disposable VM harness verifies the official Debian cloud image's SHA512 checksum, creates a throwaway SSH key and an isolated overlay disk, and destroys the VM after completion. No host directory is mounted into the guest. Its root test script refuses a different hostname. It deliberately fills the guest disk and injects service/installer failures; run it only through the harness:

```sh
python3 packaging/build.py --arch amd64 --binaries target/release --version 0.1.1
# Use a disposable signing key for the test repository.
packaging/repository.sh "$PWD/dist/test-repo" TEST_KEY_FINGERPRINT dist/*.deb
python3 tests/privileged_vm/run.py --repository dist/test-repo
```

The harness expects `.cache/p01-vm/debian-12-genericcloud-amd64.qcow2` and its official `SHA512SUMS` file, downloaded over HTTPS from Debian's Bookworm cloud-image directory. It checks signed apt installation, root and sudo reruns, local enrollment, HTTP/RPC boundaries, package integrity, low space, upgrade/downgrade and remove/reinstall. The final idle measurement follows authentication and enforces a combined resident-service PSS of at most 30 MiB. This is a VM result; Pi 4/5 timing, thermal/load behavior, week-long package integrity and 24-hour write qualification remain hardware/soak gates.
