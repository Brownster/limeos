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

Do not expose the cleartext loopback service through a port forward. Core ignores forwarded identity and client-IP headers. Requests through one proxy share the same per-peer login quota: ten attempts per minute, with a global maximum of 120.

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
| Host executor | root, primary group `limeos-host-access` | `/run/limeos-storaged` (0750) | Root-owned policy; health only, no ambient capabilities |
| Reserved assistant | `limeos-assistant` | `/var/lib/limeos/assistant` (0700) | No Docker or executor access; no P01 service |
| Packaged code/UI/contracts | root | `/usr/lib/limeos` | Services cannot modify it |
| Independent ceilings | root | `/etc/limeos/system-policy` | Exact core UID, protocol version and allow-health flag |

Each socket uses mode 0660 and a distinct access group. Executors inspect kernel peer credentials before allocating a request frame. Having the socket's group does not substitute for the configured core UID. The root ceiling is independent of core's user policy and is read on executor startup; restart the affected executor after an operator changes it. Protocols accept no shell, executable, arbitrary file content or caller-provided principal.

Only core owns the SQLite authority database. Task tokens store digests and bind principal revision, service UID, task ID, exact scopes, expiry and core generation. Changing a grant invalidates earlier sessions and task authority. Restart invalidates task authority; browser sessions remain durable. P01 supplies the tested storage primitives for future job dispatch and provider budgets; it exposes no task-minting or effectful operation API.

## Persistence and recovery

Contracts and the authority schema are version 1. The schema is generated under `contracts/generated`; Rust types are authoritative. SQLite 3.53.2 is bundled; startup refuses versions older than 3.51.3. One bounded worker serializes transactions and holds the single-core file lock. WAL uses FULL synchronization. Job intent and its event commit together. Reusing an idempotency key with changed intent fails. Resource locks survive ambiguous interruption.

At startup, running/verifying jobs become `needs_intervention`. Core never retries them automatically. Receipt-based reconciliation arrives with executor effects in P03. Unknown provider usage keeps the maximum reservation; settlement is unique and cannot exceed it. Cost units are integers, not floating-point currency.

Writes fail closed below an 8 MiB free-space reserve or at the 64 MiB audit allowance. Read requests do not renew sessions, write timestamps or rewrite configuration. Audit retention and supported archival are later work; do not delete or hand-edit database rows to clear the allowance. A future schema causes startup refusal instead of an automatic downgrade. P01's 0.1.0/0.1.1 upgrade and downgrade tests use the same schema; they do not establish compatibility across later schema versions.

For a configuration failure:

```sh
sudo /usr/lib/limeos/limeosctl check-config /etc/limeos/core.json
sudo journalctl -u limeos-core -u limeos-containerd -u limeos-storaged
# Repair the reported configuration or space problem, preserving the original.
sudo dpkg --configure limeos
sudo /usr/lib/limeos/limeosctl status
sudo dpkg --verify limeos
```

The installer preserves corrupt files and exits with a repair instruction. Activation and removal errors surface to dpkg. Removal and purge preserve authority, account identities and ceilings; reinstall a compatible package to recover. Online backup, encrypted restore and the production updater are P04 work. Never copy an active SQLite database file without its WAL as a backup.

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
