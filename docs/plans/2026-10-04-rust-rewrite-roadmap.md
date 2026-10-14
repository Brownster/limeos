# LimeOS Rust rewrite: delivery roadmap

Status: P01 foundation implementation delivered 2026-10-04; local acceptance and current native ARM64/AMD64 CI pass. Remaining P00 assumptions, current installed ARM64 and hardware/soak qualification stay open.
Latest completed-slice integration: [October 12 acceptance](../rewrite-evidence/p04/2026-10-12-completed-slices/README.md), including before-images, retained SnapRAID sources, configured pool/protection dependencies, Samba planning and the current ARM64 harness. P04 and live storage/restore gates remain open. The [integrated Pi runbook](2026-10-12-arm64-integrated-qualification-runbook.md) retains the original frozen inputs and resulting qualification limits.
The [October 13 configured graph integration](../rewrite-evidence/p04/2026-10-13-configured-graph-integration/README.md) adds the pure container/pool/protection/Samba join to current main, with 533 passing Rust cases and 61 ARM64 harness tests. Protected live collection, complete claims, effect-time checks and recovery remain the next RW-040 boundary; the signed ARM64 candidate keeps its earlier frozen source.
The [October 13 retained configuration acceptance](../rewrite-evidence/p04/2026-10-13-storage-configuration/README.md) adds a protected read-only owner for typed storage, pool, SnapRAID and declared share files. Current local validation passes 559 Rust cases and 61 ARM64 harness tests. The owner detects observed file/ancestor changes and preserves its original lease; physical/namespace composition, whole-worker admission, actual Samba state, installed qualification and effects remain open. Pi cutover stays deferred.
The [October 13 destination acceptance](../rewrite-evidence/p04/2026-10-13-container-destinations/README.md) adds a borrowed running-destination/physical-source owner for flat supported mounts, with 589 passing Rust cases and 61 local ARM64 harness tests. Same-object checks preserve independent namespace mount IDs and original parent leases. Whole-worker admission/supervision, Engine/configuration integration, nested and unselected physical consumers, actual pool/share state and installed root/foreign-namespace qualification remain open. Cutover stays deferred.
The [October 13 process admission acceptance](../rewrite-evidence/p04/2026-10-13-process-read-admission/README.md) adds a closed combined-reader profile with complete membership refusal and bounded mount parsing, with 602 passing Rust cases and 61 local ARM64 harness tests. Eight owned local processes measured 37 retained and 41 peak descriptors under a private host-authentication substitution. Storage/source admission, whole-worker memory and supervision, public root/foreign-namespace and installed qualification remain open; no worker or cutover is enabled.
The [October 13 storage admission acceptance](../rewrite-evidence/p04/2026-10-13-storage-read-admission/README.md) extends closed bounds through retained inventory/source/dependency owners, with 623 passing Rust cases and 61 local ARM64 harness tests. Host parsing, topology edges, probes, swap rows, source references and symlink queues refuse excess input; parents retain their profile and original expiry. Concrete whole-worker budgeting and independent supervision come next. Public root/device, foreign namespace and installed qualification remain open; cutover stays deferred.
The [October 13 combined budget acceptance](../rewrite-evidence/p04/2026-10-13-combined-read-budget/README.md) adds closed configuration shape admission and an unregistered bin-library resource model/session, with 644 passing Rust cases and 61 local ARM64 harness tests. It reserves 178 retained descriptors under a required 226-FD worker limit, preserves closed adapter profiles, and refuses phase/report/resource excess. The memory model is conditional, with installed headroom unqualified. Genuine owner composition and independent worker supervision remain next; no worker or cutover is enabled.
The [October 13 retained collection acceptance](../rewrite-evidence/p04/2026-10-13-combined-read-collection/README.md) composes genuine closed-profile configuration, Engine, process, physical and destination owners with the configured graph. Local validation passes 658 Rust cases and 61 ARM64 harness tests. Original leases survive bracketed validation and one bounded borrowed report; late cancellation/expiry discards the result. The entry remains unregistered and the probe’s combined case stays blocked pending independent worker supervision. Root-positive composition, installed resource/capability/namespace qualification and performance remain open; cutover stays deferred.
The [October 14 worker supervision acceptance](../rewrite-evidence/p04/2026-10-14-read-worker-supervision/README.md) adds unregistered guardian/monitor mechanics, with 675 passing Rust cases under the opt-in local fixture feature and 61 ARM64 harness tests. Independent deadline/cancellation, manager death, adopted escaped-session children and confirmed cleanup pass local owned-process tests. Admission stays blocked through cleanup and poisons unconfirmed completion. Protected fixed launch/configuration selection, combined probe integration, installed confinement/resource qualification and root-positive composition remain next; no worker service or cutover is enabled.
The [October 14 protected launch acceptance](../rewrite-evidence/p04/2026-10-14-combined-read-launch/README.md) adds unregistered fixed-image policy, bounded configuration selection and a separate opt-in combined probe, with 690 passing Rust cases and 61 local ARM64 harness tests. A bounded sealed ELF copy preserves executable bytes across source changes; the monitor holds its lease through cleanup and preserves the original preparation deadline. Owned local guardian/worker fixtures reach the genuine nonroot configuration guard. Public root-positive collection, installed memfd/proc/fd/syscall compatibility, whole-tree resource headroom and independent review remain open. The existing standalone combined case stays blocked pending external adoption of the new seam; the frozen signed ARM64 candidate, worker services and cutover remain unchanged.
The [October 14 declared impact acceptance](../rewrite-evidence/p04/2026-10-14-storage-impact/README.md) adds a pure selection planner with complete expected-device matching, original declaration age and bounded typed resource candidates, with 706 passing Rust cases and 61 local ARM64 harness tests. Related pool/global SnapRAID members remain separate from selected devices and their affected-container list. Frozen-layout, identity, deduplication, 128/129-resource, global route and report-size regressions pass locally. Every preview keeps unresolved retained/physical/runtime, stack ownership, complete claims/approval and effect-time checks explicit; no live claim, lock or effect is acquired. Installed qualification, P04/defect/performance gates and cutover remain open.
The [October 14 Compose membership acceptance](../rewrite-evidence/p04/2026-10-14-compose-membership/README.md) adds an unregistered UID-0 fixed-endpoint reader for complete Engine-declared project/service/one-off membership, with 732 passing Rust cases and 61 local ARM64 harness tests. Selected labels are double-inspected and freshly compared under the original Engine lifetime and unchanged closed ledgers/report limits. Failure, in-flight cancellation or reentrancy permanently invalidates the new owner; old storage interfaces remain unchanged. Owned listener tests use private credential/clock fixtures and qualify no installed/root-positive Engine. Actual protected stack files, management/claim identity, physical completeness, effects/recovery and installed qualification remain open; no stack action, service activation or cutover is enabled.
The [October 14 Compose file acceptance](../rewrite-evidence/p04/2026-10-14-compose-files/README.md) adds unregistered retained file identities/hashes for one recognized operator file and an explicitly selected fixed managed file, with 759 passing Rust cases and 61 local ARM64 harness tests. Protected no-follow traversal, whole-request filename ambiguity, fresh/held content and late named-object/ancestor/absence changes, original lifetime/read accounting and permanent invalidation pass local owned-descriptor tests. The production reader requires UID 0 and a retained PID-1-relative mount-namespace reference; private successful fixtures qualify no root-positive/foreign-namespace/installed collection. Approved catalog/management and genuine Engine-to-file binding, actual Compose semantics, complete claims, effects/recovery and installed resource/performance qualification remain open. No worker model, service activation, signed candidate or cutover changes.
The [October 14 declared management binding acceptance](../rewrite-evidence/p04/2026-10-14-compose-management/README.md) binds one protected management registration to genuine retained catalog/file and Engine owners, with 793 passing Rust cases and 61 local ARM64 harness tests. Exact Engine/root/path/file-order/hash/service matching, complete original membership, overlapping project refusal, original leases/ledgers, borrowed report bounds and permanent failure/cancel/drop/timeout/reentrancy/concurrency/unwind invalidation pass local owned fixtures. The first failed run and corrective checks remain lossless; the absent managed variant is closed and the stopped fixture respects the existing zero-PID guard. A final rollback fixture margin avoids a wall-second boundary race. Root-controlled declarations are separate from P03 permissions/approvals and actual executable Compose semantics. Complete physical consumers, acquired claims, locks/effects/recovery, independent review, worker integration, public root/namespace/installed/resource/performance qualification and cutover remain open; no service, schema, signed candidate or activation changes.
The [October 14 typed override acceptance](../rewrite-evidence/p04/2026-10-14-compose-override/README.md) adds a pure conservative compiler for replacement managed Compose JSON, with 813 passing Rust cases and 61 local ARM64 harness tests. Supplied operator/current/desired projects retain numeric ownership, literal paths, complete impacts and managed-service shutdown requirements. Byte-exact fixtures and an independent test-side merge oracle cover supported map/resource/sequence semantics; inherited service/port/mount/device removal, remapping and unsupported clearing refuse. Pre-clone admission, sixteen-service/eight-entry and 128-volume boundaries, deterministic artifacts and 64 KiB artifact/256 KiB borrowed-summary ceilings pass locally. Actual protected source admission, pinned Compose conformance, retained runtime/physical binding, fresh executable approval, locks/writes/recovery and verified shutdown remain open. No host adapter, schema, service, signed candidate or activation changes; P04/installed/performance/cutover gates remain open.
The [October 14 strict JSON source acceptance](../rewrite-evidence/p04/2026-10-14-compose-source/README.md) admits actual bounded operator/managed byte slices and verifies generated override combinations, with 836 passing Rust cases, 61 local ARM64 harness tests and six independent offline configuration-parser checks. Closed fields, duplicates/escaped equivalents/nulls, raw and aggregate bounds, scalar inheritance/false values, numeric ownership, literal mounts/devices, standard volume references, complete service preservation and raw hashes pass/refuse as specified. Final review closes Serde's alternative array-struct and tagged-object enum representations; both failed targeted runs and corrective evidence remain lossless, with final acceptance bound to hardened source. Shared fixtures bind Rust compiler outputs to the observed AMD64 Compose 5.3.0 parser and exact executable hash, using scratch files, cleared environment and no daemon endpoint/effects. General YAML, other versions, publisher/protected launch trust and ARM64 installation remain unqualified. Genuine retained source/Engine binding, physical completeness, fresh executable approval, locks/writes/recovery and verified shutdown remain open; no schema, Core/Store, service, signed candidate or activation changes. P04/installed/performance/cutover gates remain open.
The [October 14 retained JSON acceptance](../rewrite-evidence/p04/2026-10-14-compose-retained-json/README.md) binds supported JSON admitted from protected held FDs to the genuine management/catalog/file/Engine owners, with 860 passing Rust cases, 61 local ARM64 harness tests and six existing pinned offline parser checks. Original four-second windows and spent ledgers remain intact; identity/hash/namespace/name checks bracket positional body reads and parsing. Defined/registered/observed service association, complete original membership, original byte/FD/report bounds and permanent failure/cancel/drop/timeout/reentrant/concurrent/unwind invalidation pass local owned fixtures. Valid child checks cannot restore a cancelled source owner; the initial test-helper visibility compile diagnostic remains lossless. No new lease/retained FD, child getter or host effect is introduced. Deployed image/mount/port/user/network equivalence, general YAML, independent daemon/physical identity, physical completeness, fresh approval, claims/locks/writes/recovery, verified shutdown, worker integration and root/installed/resource/performance qualification remain open. No Core/Store/schema/dependency/service/signed candidate/activation change; P04 and cutover stay open.
The [October 14 bound lifecycle acceptance](../rewrite-evidence/p04/2026-10-14-compose-lifecycle/README.md) consumes genuine retained JSON/catalog/file/Engine owners to compile exact managed candidate bytes and associate source changes with complete original selected service membership. Local acceptance covers 876 Rust cases, 61 local ARM64 harness tests and six existing pinned offline parser checks. Registered desired services, full impacts/numeric ownership, add/change/remove candidates, replicas/one-offs/stopped/mountless rows, original windows/read ledgers, 256 KiB whole-report refusal and permanent failure/cancel/drop/timeout/reentrant/concurrent/unwind invalidation pass owned fixtures. Successful child checks cannot restore a failed proposal; initial test expectations and exhausted-ledger diagnostics remain lossless. Source change association and artifact SHA256 grant no execution, principal permission, approval, shutdown receipt, safe absence or deletion authority. Deployed configuration equivalence, independent daemon/host identity, physical consumers, acquired claims/locks, fresh approval, atomic writes/recovery, shutdown/effect-time checks, worker integration, public root/installed/resource/performance qualification and independent review remain open. No Core/Store/schema/dependency/service/signed candidate/activation change; P04 and cutover stay open.
The [October 14 Compose review-subject acceptance](../rewrite-evidence/p04/2026-10-14-compose-review/README.md) binds ordinary P03 preview facts to genuine retained lifecycle/source/file/catalog/Engine owners, with 894 passing Rust cases, 61 local ARM64 harness tests and six existing pinned offline parser checks. Canonical before/desired/impact/operator matching, 48 KiB pre-clone preview hashing, distinct domain-separated review identities, exact existing scope requirements, complete original Engine socket/policy/PID/declared-mount context and 256 KiB whole-report/streaming-hash bounds pass owned fixtures. Genuine recollection of identical objects matches despite new collection times; PID/mount changes require a different review identity. P03 expiry is checked before and after genuine child validation. Original leases/ledgers and permanent failure/cancel/drop/timeout/reentrant/concurrent/unwind invalidation remain; successful child checks cannot restore failed subjects. Preview values, digests and scope requirements do not authenticate Store/session/principal/current grants or approve effects. Existing P03 approval remains preview-only. Authenticated durable review/fresh executable approval, acquired claims/locks, recovery, runtime/physical completeness, shutdown/effect-time checks, worker integration, root/installed/resource/performance qualification and independent review stay open. No Core/Store/schema/dependency/service/signed candidate/activation change; P04 and cutover remain open.
The [October 14 authenticated Compose review-read acceptance](../rewrite-evidence/p04/2026-10-14-compose-authority/README.md) resolves actual live session/task authority, stored preview facts and complete exact requirements in one deferred SQLite read snapshot. The existing Core preview-read API now uses session resolution and rejects inconsistent durable revision/expiry metadata while preserving its response and schema 9. Local acceptance covers 927 Rust cases after the independently merged private backup reconstruction, 61 local ARM64 harness tests and six existing pinned offline parser checks. Revocation/expiry/current grants/roles/task UID/task ID/generation/revision, exact elevated requirements, bounded caller comparison, catalog/cancellation/body/digest/metadata corruption, rollback/late expiry, query-only purity, concurrent WAL revocation and restart pass actual database fixtures. Owner/report import refusal and 64 KiB borrowed report limits pass. All diagnostics and the API regression red/green remain lossless. Observations retain no authentication secret and grant no permission lease, fresh executable approval, claim, lock or effect. Root broker/transport, durable fresh approval, acquired locks/recovery, runtime/physical completeness, shutdown/effect-time checks and installed/resource/performance qualification remain open. The operator permits needed Pi testing after 22:00 Europe/London; this database slice uses local tests. No host access or cutover occurs; signed ARM64 candidate and P04/cutover gates stay unchanged.
The [October 12/13 Pi report](../rewrite-evidence/arm64/2026-10-13-current-016ba49-memory-guard/README.md) records passing current deadline supervision and healthy teardown, but both compilation attempts stopped at the host swap guard. Installed/upgrade/recovery/footprint gates remain untested. QEMU/image prerequisites were retained dormant with the operator's approval. [Explicit CI-input qualification](2026-10-13-arm64-ci-input-qualification-design.md) is implemented: [local acceptance](../rewrite-evidence/arm64/2026-10-13-native-ci-af94f19-inputs/README.md) binds native CI source `af94f19`, fixtures `687ddb4`, candidate `0.4.8+arm64.3`, fresh signed repositories and 61 passing local tests. The next Pi window can run installed qualification without compilation; a new target/window is required.
The operator identified performance metrics and container/stack UI support as the minimum replacement scope, then deferred the proposed swap on October 13 and asked to continue the rewrite with a later readiness update. The [basic cutover checklist](2026-10-12-basic-cutover-readiness.md) retains the scope comparison, shadow checks, mounted-data safety, migration and a tested return path. No replacement or new host window is currently authorized; no cutover gate is closed.
Current build baseline: [Brownster/pi-health@80593b2](https://github.com/Brownster/pi-health/tree/80593b2). The reference host runs `618ce92`.

This is the entry point for the rewrite. Read the [target architecture](2026-10-04-rust-rewrite-architecture.md) for processes, permissions, storage, jobs, and packaging; the [assistant/model-routing design](2026-10-04-assistant-model-routing.md) for subscription support, native APIs, cost controls, and assistant workflows; and the [defect register](2026-10-04-rust-rewrite-defect-register.md) for every known defect in the current build and the rule that keeps it out of the new one.

## 1. Destination

Deliver a compact Rust host-management application that replaces the current build on existing installations, preserves storage and services, recovers predictably after failures, and includes a useful native assistant. Claude subscription and the native Anthropic API are supported from the first assistant release. Other native API providers and qualified local endpoints follow cutover, so model choice can then follow task quality, price, latency, privacy, and availability.

Ordinary dashboards, operations, recovery, alerts, and notifications work without a model. Mattermost is no longer a prerequisite for the assistant; P00 decides from actual use whether its transport returns after cutover.

The sequence is governed by evidence gates, not a file-by-file translation. A phase is complete only when its user-visible behavior and failure cases work. If a foundation assumption fails a spike, record and revise the architectural decision before building dependent features.

### Decisions that guide the work

- Rust for footprint, start-up speed, and compile-time correctness of root-side and job code. Python is already memory-safe, so memory safety counts only for new native code, not as a gain over the current implementation. The [reference Pi 5 baseline](../rewrite-evidence/p00/2026-10-04-reference-pi5-baseline.md) sets the targets; see "Why Rust" in the architecture.
- The current build is frozen. Its known defects are fixed only in the new build, each with a regression scenario in the defect register (section 2).
- The cutover release is the smallest scope that replaces the current build: host, storage, and deployment parity; deterministic schedules and notifications; and a minimal native assistant. Media requests, more model providers, model-driven automation, and chat transports follow cutover and cannot delay it.
- Phases run one after another with one integrating implementer. On 2026-10-05 the operator asked how two additional engineers could help: [native ARM64 qualification](2026-10-05-engineer-arm64-qualification.md) and [RW-041 pool/protection planning](2026-10-05-engineer-pool-protection-planning.md) can proceed on separate branches while RW-040 continues. These bounded assignments preserve phase gates; shared runtime integration remains sequential.
- One cutover per installation, after read-only shadow running and a rehearsal on a test host. The two builds never mutate the same host at the same time.
- One modular unprivileged core, plus narrow host/Docker executors and isolated assistant/provider/connector runtimes.
- One operation registry, policy engine, and durable job journal for every caller.
- Root-owned releases as Debian packages from a signed apt repository; no privileged code is loaded from an application-writable path.
- Local SQLite authority and bounded observations; no base PostgreSQL, Redis, or message broker.
- The React UI is copied into this repository and moved to the new API screen by screen, using types generated from Rust. It owes no compatibility to the Flask API.
- Independent model routing and permissions; no automatic paid fallback from a subscription limit.

Architecture decisions A01–A10 are defined in the companion document. These are proposed defaults concrete enough to implement; external provider contracts and measured resource targets are verified at their named gates.

## 2. The current build during the rewrite

The current build is frozen at `80593b2`, with the local reference at `/home/marc/Documents/github/pi-health`. It gets no features, refactors, or fixes; the refactor tickets REFAC-001 to REFAC-004 are cancelled. All development happens in this repository. The original repository remains the reference for behavior, fixtures, and test scenarios.

Known defects stay in the current build until cutover. The [defect register](2026-10-04-rust-rewrite-defect-register.md) carries each one into the new build as a requirement with a regression scenario. Its sources are the [June 2026 review](https://github.com/Brownster/pi-health/blob/80593b2/Docs/colleagues_review_of_limeOS.txt), the [July 2026 rollout tickets](../rewrite-evidence/current-build/2026-07-27-rollout-recovery-tickets.md), the plan review, and the [reference-host baseline](../rewrite-evidence/p00/2026-10-04-reference-pi5-baseline.md).

Accepted risk on existing installations until cutover, with mitigations an operator can apply without changing the current build's code:

| Risk | Mitigation |
|---|---|
| The root helper executes a file the dashboard account can write (LIVE-001). | `sudo chown root:root` and `sudo chmod 0644` on the helper's target file. Self-update no longer needs to write it, because the build is frozen. |
| Backup restore extracts an archive over `/` as root (BKP-001). | Restore only archives this installation created. |
| Startup-service activation can enable a unit that hangs the next boot (MNT-001, MNT-002). | Do not use the Startup service card on a guided-storage installation. |

The main control for this risk is a short path to cutover.

## 3. Scope and migration coverage

P00 turns this table into a checked feature inventory covering endpoints, screens, schedules, state files, tests, and deployed usage. Each row must end as preserved, deliberately replaced, or explicitly retired with a migration explanation. Planned features such as NFS are not silently counted as current parity requirements.

| Existing domain and reference | Target ownership | Delivery |
|---|---|---|
| Host overview, process/system stats, temperatures, history, prerequisites, alerts: `overview_service`, `system_stats`, `metric_history`, `host_prerequisites`, `alert_*`. | Observations plus deterministic alert/scheduling modules. | Reads P02; schedules and alerts P04. |
| Container inventory, cached stats, start/stop/restart, logs: `container_*`. | Containers domain and Docker executor. | Reads P02; writes P03. |
| Compose stacks, app catalog, CopyParty/tools, networks, stack notifications: `stack_*`, `catalog_*`, `tools_*`, `network_*`. | Typed deployment plans, capability catalog, bounded executor. | Foundations P03; complete P04. |
| Disk inventory, UUID/serial roles, mounts, storage contract, media layout: `disk_*`, `storage_contract*`, `media_layout*`. | Storage domain with independent root safety checks. | Reads P02; writes P04. |
| MergerFS, SnapRAID, protection, remote mounts, SSHFS/rclone, Samba: `storage_plugins`, `storage_capability_adapters`. | Declarative capabilities and closed host operations. | P04. |
| Backups, restore, packages, container/self-updates: `backup_*`, `limeos_packages`, `update_*`. | Durable schedules, staged restore, apt-based release manager. | Packaging P01; full parity P04. |
| Media profiles, quickstart, service seeding, activity reads: `media_*`, `arr_client`, `activity_*`. | Media domain and credential-scoped connectors. | Quickstart, seeding, and activity P04; assistant media requests P07. |
| Login, users, capability lifecycle: `auth_utils`, `capability_*`. | Core identity/policy and declarative registry. | P01/P03; full lifecycle P04. |
| Assistant provider, gateway, actions, diagnosis: `agent_gateway`, `agent_provider`, `agent_actions`, `limeops`. | Assistant, core jobs/policy, provider adapters. | Minimal native assistant P05. |
| Scheduled reports, supervised repair, canary: `agent_automation`, `agent_supervision`, `agent_findings`. | Bounded runbooks on the core job system. | Retired at cutover with a migration note; returns in P09. |
| Mattermost bot setup, actor mappings, conversation delivery, integration lifecycle: `mattermost_*`, `agent_transport`, `integration_*`. | Optional transport. | Decided in P00; if kept, P09. Mattermost server and data are untouched at cutover. |
| Ordinary notifications. | Deterministic notifier. | P04. |
| React screens in `frontend/`; companion app in `companion/`. | Copied UI on generated contracts; companion compatibility decided in P00. | Screen by screen, P02–P05. |
| Installation, runtime paths, ARM64/x86-64 packaging, systemd, recovery. | Debian packages, signed apt repository, migration/recovery tools. | Foundations P00/P01; signoff P06. |

Existing [assistant capability proposals](https://github.com/Brownster/pi-health/blob/80593b2/Docs/LIMEOS_ASSISTANT_CAPABILITY_ROADMAP.md), [agent action foundations](https://github.com/Brownster/pi-health/blob/80593b2/Docs/LIMEOS_AGENT_ACTIONS_FOUNDATION.md), and [integration design](https://github.com/Brownster/pi-health/blob/80593b2/docs/plans/2026-07-12-ai-agents-integration-design.md) are inputs. Carry forward valuable invariants, approval semantics, and fixtures; replace the Mattermost/provider coupling and duplicate operation mechanisms.

## 4. Phase sequence and dependencies

| Phase | Depends on | Demonstrable result |
|---|---|---|
| P00 — Inventory and prove assumptions | None | Agreed invariants, measured baseline, passed spikes, and decisions on Mattermost and the companion app. |
| P01 — Secure Rust foundation | P00 | Installable Debian packages, identity/policy, durable state. |
| P02 — Observe and compare | P01 | Rust dashboard reads match the current build on the reference host without changing it. |
| P03 — Durable operations and executors | P02 | On the test host, a container restart is authorized, durable, bounded, and recovered after interruption. |
| P04 — Host, storage, and deployment parity | P03 | Every preserved inventory row works on the test host, including schedules, backup/restore, and updates. |
| P05 — Minimal native assistant | P03; built after P04 | Connect Claude, ask a sourced question, diagnose, and perform a permitted repair in the native UI. |
| P06 — Migration and cutover | P04, P05 | The reference host and a fresh install run only the new build; the return path is rehearsed. |
| **After cutover** | | |
| P07 — Media requests | P06 | A scoped household user adds the correct film or series and sees verified progress. |
| P08 — More model providers and routing | P06 | Qualified API and local deployments switch under explicit privacy, quality, and budget rules. |
| P09 — Model-driven automation and chat transports | P06 | Approved scheduled maintenance and, if kept, Mattermost run through the same policy and jobs. |

P05 depends technically only on P03 but runs after P04, for three reasons. Parity is the largest and riskiest work, so doing it first firms up the estimate early. The assistant's tools are registered operations, so it is more useful once parity has registered them. And provider integrations built closer to release need less rework when upstream CLIs and terms change.

P07–P09 can run in any order. None of them can add a dependency to P06.

### Planning rules

- Record an estimate when a phase starts and the actual when it ends. If a phase passes 150% of its estimate, review the cut line before continuing.
- P00's estimate and progress are recorded in the [execution tracker](../rewrite-evidence/p00/2026-10-04-execution-tracker.md).
- Every phase moves its own screens to `/api/v1`. The current UI calls about 97 distinct legacy API paths, and the plan, approval, job, and event model changes every mutation flow, so UI work is budgeted in each phase rather than deferred.
- Each defect-register row is closed by its regression scenario in the phase the register names.
- All mutation testing happens on the test host, a VM or spare Pi carrying a redacted copy of the reference host's configuration. The reference host stays read-only for the new build until P06.
- Wybie is an in-use media server. During active playback, restrict access to light passive reads and defer active probes and benchmarks. Builds, load tests, provider workloads, and failure injection run locally or on the separate test host. Future Pi benchmarks, shadow installation, and cutover use an operator-agreed quiet window.

### P00 — Inventory, threat model, and architecture spikes

Work packages:

- **RW-001:** Inventory existing behavior and state ownership. Distinguish repository intent from the deployed installation, including configuration edited inside the deployed checkout. Capture redacted configuration snapshots, supported disk identities, stack roots, users and their password-hash formats, the service account and its group memberships, active plugins, integrations, schedules, and legacy API clients. Record actual use of Mattermost and the companion app, and decide whether each returns after cutover or is retired.
- **RW-002:** Measure idle/active memory, CPU, startup, HTTP latency, provider latency, database/log writes, and installed size on a reference Pi and x86-64 VM. Use the architecture's reproducible workload and record hardware/kernel versions. The [reference Pi 5 baseline](../rewrite-evidence/p00/2026-10-04-reference-pi5-baseline.md) is recorded. Still needed: authenticated endpoint latency, dashboard readiness after restart, Claude CLI memory during a turn, attribution of the helper's child-process writes, a Pi 4 with an SD-card OS disk, and an x86-64 host.
- **RW-003:** Write the threat model, operation/risk inventory, trust-boundary ADRs, and migration invariants. Define the compatibility promise for Debian versions, storage contracts, credential import, password verification, and APIs. Review the defect register and add anything the inventory finds.
- **RW-004:** Prove Linux mechanics in disposable VMs: peer-credential RPC, root-owned package permissions, mount namespace behavior, disk identity races, process-group cancellation, receipt recovery, database backup/restore, and Debian package install, upgrade, downgrade, and removal beside the current build.
- **RW-005:** Prove provider mechanics with capped opt-in tests: isolated Claude login/output/tool restrictions; the Anthropic native tool loop; bounded usage reservation; and recorded OpenAI/Gemini continuation fixtures, so the canonical contract does not need breaking after cutover. Build a minimal ARM64 Rust binary and confirm the selected toolchain/dependency path.

Gate: each spike produces an artifact, result, and decision. No unresolved assumption about host mount visibility, mutable root code, SQLite recovery, or subscription isolation carries into production writes. Set and record performance targets from the baseline. Live disk mutation is never a spike on the user's data disks.

The RW-005 ARM64 binary also tests the footprint argument. Built with the planned stack (Axum, Tokio, rusqlite, Rustls, and one Docker read) and run on the reference Pi 5, it must idle under 15 MiB PSS and reach readiness in under 1 second. If it does not, revise the footprint targets and the "Why Rust" rationale before P01 begins. The [2026-10-04 result](../rewrite-evidence/p00/2026-10-04-rw005-footprint-result.md) passes: 4.55 MiB idle PSS, no swap, and all ten starts ready within 23.5 ms. Other P00 gates remain open.

Evidence directory: `docs/rewrite-evidence/p00/` with redacted inventory, benchmark method/results, threat model, ADRs, and VM/provider reports. Add evidence as it is produced, not as empty placeholders.

### P01 — Secure Rust foundation

Implementation delivered 2026-10-04 under the operator's authorization to proceed from the positive P00 result. [P01 evidence](../rewrite-evidence/p01/2026-10-04-execution-tracker.md) records local validation and outstanding native ARM64, hardware and soak qualifications. Remaining P00 assumptions stay open and block the affected production effects, not this health-only foundation.

Work packages:

- **RW-010:** Establish the workspace and dependency rules, pinned toolchain and committed lockfile, generated HTTP/RPC/schema contracts, structured error/event types, bounded Tokio work queues, and CI on both architectures. CI gates on `cargo fmt`, Clippy with warnings denied, `cargo deny`, `cargo audit`, ShellCheck over maintainer scripts, `systemd-analyze verify` over shipped units, and the frontend type check and build.
- **RW-011:** Implement core-owned identity, sessions, verification of imported Werkzeug scrypt and PBKDF2-SHA256 hashes with upgrade to Argon2id, one-time local bootstrap enrollment, role/resource scopes, grant revisions, CSRF/rate limits, and independent executor ceiling validation.
- **RW-012:** Implement the authority schema and job state machine, transactional intent, idempotency constraints, events, opaque task tokens, the single-instance core generation and start-up reconciliation of unfinished jobs, migrations, space-pressure handling, and recovery classifications. Configuration reads distinguish missing, corrupt, and valid files and never write; writes are atomic. Add budget reservations to this authority boundary before providers use them.
- **RW-013:** Build ARM64 and x86-64 Debian packages and a signed apt repository: root-owned binaries and UI under `/usr/lib/limeos`, dedicated service users (never root, never a human login account), distinct sockets, credential delivery, systemd limits, health/status commands, and maintainer scripts that are idempotent and safe to rerun.

Gate: a clean VM installs from the apt repository and authenticates without any default credential. Malformed/oversized RPC and forged principal requests are rejected; an unprivileged account cannot edit a privileged executable or open executor sockets. Task tokens expire, can be revoked, and are invalid after a core restart. Crash injection preserves committed intent and rejects conflicting idempotency keys. Core refuses writes when durable state cannot be recorded. A corrupt configuration file is preserved and reported, not replaced. Failure injection during install, rerun, upgrade, and removal leaves the host consistent. Secrets do not appear in schemas, errors, or logs.

Signoff artifacts: schema/contract versions, ownership matrix, tested systemd units, dependency/unsafe/native-code audit, and initial footprint results. Full update rollback comes in P04, but the ownership and verification boundary exists now.

### P02 — Read-only vertical slice and shadow comparison

Work packages:

- **RW-020:** Implement centralized host/container/disk observations with bounded probes, Docker events plus reconciliation, timestamps, adaptive sampling, and disposable telemetry aggregation. One Docker snapshot serves every stack and dashboard. Each optional metric source fails independently.
- **RW-021:** Serve new inventory/overview/read APIs and SSE. Move the matching React screens to generated types, pagination, stale-state display, explicit loading/error/partial-data states, and server error details.
- **RW-022:** Compare new read results to the current build using semantic fixtures and live shadow reads. Import telemetry history or document a separate legacy-history view; do not drop 31 days of history silently.
- **RW-023:** Install the read-only build on the reference host beside the current build: its own port and login, executor ceilings limited to read operations, and no access to the current build's files or databases.

Gate: representative disk/pool/container fixtures agree on resource identity and status; a missing disk or stale cache is clearly represented. Repeated dashboards do not create duplicate collectors. Bounded output, reader interruption, retention, and SSE reconnect tests pass. The shadow install changes no host state, and removing it leaves the reference host as it was. Performance targets have measured results on the reference hardware.

### P03 — One mutation path and narrow executors

Work packages:

- **RW-030:** Deliver `container.restart@1` end to end: scoped plan, authorized queueing, executor receipt, independent verification, and durable progress.
- **RW-031:** Implement container and host executor framing, peer-credential checks, independent policy ceilings, bounded argv/output/time, per-resource locks, cancellation, and protected receipt persistence.
- **RW-032:** Add start/stop and bounded log reads, then the approved Compose plan/diff foundation. Every mutation is a POST with CSRF validation; GET never starts an operation. Reject elevated template privileges without the specific deployment grant.
- **RW-033:** Set up the test host with a redacted copy of the reference host's configuration and representative containers, and run every mutation scenario there.

Gate: kill core/executor before dispatch, during execution, and before receipt persistence. Restarting either cannot issue an unjustified second effect. Demonstrate expired approval, revoked grant, stale resource revision, duplicate request, changed plan, and cancellation behavior. Only the container executor's account can open the Docker socket. A browser and an assistant-shaped test caller receive identical policy decisions.

First test-host release: the copied UI manages containers through Rust on the test host.

### P04 — Host, storage, and deployment parity

Work packages:

- **RW-040:** Port storage identities/contracts, mount and fstab/systemd plans, boot-disk exclusion, guided disk roles, media layout invariants, and fresh mount/identity verification. Mount waits use the contract's device mount points with a bounded timeout. Unmount checks dependent containers, shares, pools, and protection paths first. Runtime mount loss stops the stacks that depend on that mount.
- **RW-041:** Port MergerFS, SnapRAID/protection, remote mounts, supported share capabilities, package/prerequisite setup, network bootstrap, and capability lifecycle. SnapRAID rejects pool paths and unverified mounts and fails closed when its diff check cannot run. Every host operation reports its real result. Replace arbitrary imported plugins with approved adapters and isolated protocol paths.
- **RW-042:** Finish app catalog/Compose deployment, network grouping, tools, media quickstart/seeding and activity reads, and configuration diffs, preserving numeric ownership and container path contracts. Per-stack locks and atomic writes; managed override files instead of re-serialized operator YAML; single-service removal; deletion only after a successful shutdown; ambiguous Compose filenames block actions.
- **RW-043:** Deliver staged encrypted backup, key-recovery documentation, archive limits/link/path validation, managed-destination restore, online DB snapshots, and compatible application-update recovery.
- **RW-044:** Deliver apt-based self-update and container-update policy, migration compatibility, safe version transitions, state-space alerts, and explicit interventions for ambiguous destructive effects. Each host prerequisite (memory cgroup, journal cap) has one detection rule and one repair, owned by the host executor.
- **RW-045:** Run deterministic schedules on the core job system: alerts, backups, protection sync/scrub, container updates, and package reconciliation, with maintenance windows, notification delivery, and no duplicate runs after restart.

Gate: destructive/storage scenarios pass in disposable VMs with realistic virtual disks, then on the test host. Prove boot disk protection, disk replacement/UUID mismatch detection, lost mount safety, concurrent operations, mount namespace correctness, archive traversal/decompression rejection, and restore-after-power-failure behavior. Run media hardlink/path and UID/GID regressions. A compromised low-privilege extension cannot broaden its resource access. Scheduled work survives restart without duplication, and notifications work with every model provider disabled. Each existing feature has a mapped acceptance scenario and migration status, and every defect-register row assigned to P04 passes.

Storage operations do not become autonomous merely because the assistant supports them. Formatting, pool changes, restore, destructive repair, and elevated deployments require fresh authorized approval unless an explicitly documented, sufficiently narrow runbook permits a particular operation.

### P05 — Minimal native assistant, Claude subscription, and Anthropic API

Work packages:

- **RW-050:** Build native conversations, scoped task tokens, bounded evidence/document retrieval, task checkpoints, reconnectable streaming, retention/deletion, and safe usage/status display.
- **RW-051:** Implement the isolated Claude subscription runner and native Anthropic API adapter against the canonical provider contract. Add recorded conformance fixtures, version pinning, auth/reconnect flow, cancellation, and quota/error classification.
- **RW-052:** Implement explicit task profiles, subscription preference, API output/spend reservations, and per-user limits. Paid fallback is off unless an administrator configures it. Keep routing deterministic.
- **RW-053:** Design the native setup journey: choose subscription or API, authenticate, validate the connection, review permitted resources and spending policy, then run a sourced answer and a diagnostic example. Mattermost is not part of this journey.
- **RW-054:** Deliver evidence-based diagnosis and bounded container repair through the P03 operations. Core enforces fresh approvals and configured maintenance grants, not conversation wording.

Gate: subscription-only onboarding works on a machine without Mattermost. API onboarding works without a subscription. Subscription exhaustion does not start a paid request when fallback is disabled. Provider timeout or a broken CLI leaves ordinary host management operational. Prompt injection through logs cannot obtain a new operation scope, secrets, or executor access. Interrupting a turn after a repair does not repeat that repair. Set the initial task-quality rubric and datasets here.

Assistant scope at cutover: ask what is wrong, inspect the evidence, and carry out a permitted repair in the native UI. No arbitrary shell and no live source editing are needed to make it useful.

### P06 — Migration, soak, and cutover

Work packages:

- **RW-060:** Build the migration tool for configuration, users and hashes, storage contracts, stack roots, schedules, integrations, and relevant history. Its sources include `/etc/limeos`, `/var/lib/limeos` (five SQLite databases and about 20 JSON files), and configuration edited inside the deployed checkout. Validate destinations and owners, report unsupported settings, and produce a redacted report and recovery snapshot. Never edit the current build's files.
- **RW-061:** Migrate the service account and ownership. On the reference host the current build runs as `holly`, a human login account in the `docker` group that owns stacks, configuration, and credentials. Move ownership to the new service users, and ask the operator before removing the login account from `docker`, because that also takes Docker away from their shell. Handle installations where the current build runs as root. Imported users become administrators, because the current build has no roles.
- **RW-062:** Rehearse on the test host with a copy of the reference host's state, then on the reference host: fresh install, cutover, failed activation, return to the current build, missing internet, and failed provider, on ARM64 and x86-64. Include companion-client compatibility if P00 kept it.
- **RW-063:** Run an extended representative workload and hardware interruption trials. Set soak duration and repetition count from the incident/update/retention cycles observed in P00; include at least one full cycle of configured backup, protection, and update schedules.
- **RW-064:** Cut over: stop and disable the current build's services and timers, move the new build onto port 8002, and archive the current build's checkout and runtime paths. Leave Mattermost, its data, and every media and storage path in place. Remove old runtime dependencies only after the soak and return-path signoff.
- **RW-065:** Publish operator/developer guides, threat-model limitations, the supported provider/model matrix, release manifest, resource benchmarks, and a tested offline recovery runbook.

Return path: stop and disable the new build's units, restore any ownership, modes, ACLs, or account-group access needed by the current build, and re-enable the current build's services. Keep its source and original database/configuration contents untouched. The migration records previous metadata before changing shared managed paths or account groups; return-path tests prove the old service account can read its configuration and use Docker again. The migration report also lists host effects the new build made during the soak, such as mount units or stack changes, so the return is an informed decision. The path stays available until the operator confirms retirement after the soak.

Gate: every inventory row is accounted for; migration preserves storage/media identities and service availability; exactly one build runs on each host; no root service executes application-writable code; all recovery and authorization suites pass; every defect-register row is closed. Record measured footprint and performance improvements with their workload and exclusions. No claim of improvement is accepted without comparable data.

The cutover release is complete when the reference host and a fresh install run only the new build, recover safely, and a user can complete the diagnose-and-repair journey. A Rust binary that reproduces the old endpoint list is insufficient.

## 5. After cutover

### P07 — Movies, television, and library progress

Work packages:

- **RW-070:** Register supported Radarr, Sonarr, and Jellyfin instances with isolated credentials, endpoint restrictions, verified API schemas, health, and resource-scoped permissions.
- **RW-071:** Implement search, stable title identity, ambiguity resolution, selected-season monitoring, validated root-folder/quality preferences, and idempotent add requests.
- **RW-072:** Implement lifecycle reconciliation, progress notifications, already-present behavior, timeout-after-add recovery, and safe retries. Keep library/download state authoritative in the media services.
- **RW-073:** Add household media-requester roles, quotas, preferences/consent, and a native preview/progress flow. A clear authorized media request does not need a redundant administrator approval.

Gate: test ambiguous remakes, duplicate titles, partial seasons, disconnected connectors, changed quality profile, unavailable/changed storage mounts, and add-success followed by transport timeout. Assert one logical add request and correct stable identity. Distinguish requested from downloaded/imported/available. A media user cannot read connector keys, choose an unmanaged mount, deploy a container, or modify a disk.

Demonstration: a household member requests a specific film and selected TV seasons, receives useful progress, and never needs host-administrator privileges.

### P08 — Qualified model variety and price/performance routing

Work packages:

- **RW-080:** Add native OpenAI and Gemini adapters, explicitly selected API versions, multi-tool support, native continuation handling, usage, and cancellation. Qualify compatible gateways and Ollama separately.
- **RW-081:** Implement the deployment registry, capability/context checks, privacy/region/endpoint filters, explicit profile ordering, model pinning, health/circuit breakers, and safe checkpoint escalation.
- **RW-082:** Build the versioned evaluation corpus, profile admission thresholds, protocol fixture suite, human diagnostic rubric, and measurements of successful-task cost/latency. Include weak tool callers and adversarial inputs.
- **RW-083:** Complete rate revisions, atomic concurrent budget reservations, conservative unknown-charge handling, subscription allowance reporting, opt-in fallback destinations, fallback visibility, and administrator usage controls.

Gate: the same task fixtures run through all supported native families. Changing deployment does not change permissions. Native continuation data stays with its provider; fallback cannot replay a completed effect. Local-only requests never disclose to cloud. Concurrent requests cannot over-reserve the application's configured budget. Cheaper deployments are enabled only for the profiles they actually pass.

Deliver a supported model matrix with test date, protocol/version, task qualifications, measured latency/cost, and known limitations. Do not publish a permanent ranking based on today's model names.

### P09 — Model-driven automation and chat transports

Work packages:

- **RW-090:** Move findings, approved runbooks, cooldowns, attempt limits, canary rules, supervision, and kill switches onto the core job system, alongside the deterministic schedules from P04.
- **RW-091:** Add incident deduplication and bounded background diagnosis budgets. Notify and verify through ordinary events; models are not the periodic monitoring engine.
- **RW-092:** If P00 kept Mattermost, implement its registration, verified immutable actor mappings, scoped conversation/event delivery, approval integration, credential revocation, and integration removal, preserving the existing deployment and its data.
- **RW-093:** Complete integration install/auth/configure/uninstall state machines and development-assistant separation: isolated patch artifacts and signed release handoff, with no live privileged source editing.

Gate: all primary journeys work in the native UI and, when configured, Mattermost. Unknown actors and channel-only identities get no host authority. A revoked integration stops delivery/access. Canary failure, repeated ineffective repair, missing verification, or exceeded budget stops automation. Scheduled work survives restart without duplication.

Mattermost stays optional; removing it is a separate, deliberate operator action, never a side effect of the rewrite.

## 6. Common implementation acceptance rules

Each feature ticket includes the final user behavior, operation/resource scope, data ownership, failure/recovery behavior, migration impact, and required evidence. Test invariants and real failure cases; avoid tests that merely repeat implementation details.

| Suite | What it must establish |
|---|---|
| Domain/contract | Storage and media identities, operation versions, schema bounds, migration fixtures, generated client compatibility. |
| Policy and isolation | Identical caller decisions, denied scope, changed grants, plan-bound approval, peer spoofing, protected releases, secret redaction. |
| Durable effects | Crash at each dispatch boundary, idempotent retry, core restart during an executor effect, unknown outcomes, safe cancellation, space exhaustion. |
| Privileged VM/hardware | Real mount namespaces and devices, boot protection, symlink/path races, archive limits, update/restore interruption. |
| Packaging | Install, rerun, upgrade, downgrade, and removal under failure injection; root-owned executable paths; no default credential. |
| Provider conformance | Native tool loops/continuations, malformed/partial output, quota/error classification, cancellation, budget and privacy boundaries. |
| Journey and migration | Native setup, diagnosis/repair, imported users/configuration, service-account migration, return to the current build. Media and chat journeys after cutover. |
| Defect regression | Every defect-register row's scenario fails against the defect's behavior and passes in the new build. |
| Performance | Same hardware/workload baseline and Rust run, backend and optional runtime totals, CPU/memory/latency/disk writes. |

Use mocks for deterministic CI and disposable hosts for privileged scenarios. Real-provider runs use dedicated accounts, explicit opt-in, small hard request/output limits, and capped spend. No live media library or data disk is used for destructive testing.

## 7. Risk register and response

| Risk | Early mitigation and release condition |
|---|---|
| Known defects stay live on existing installations until cutover. | Smallest viable cutover scope; operator mitigations in section 2. |
| Scope growth delays cutover. | Cut line in section 1; post-cutover phases cannot add dependencies to P06; review at 150% of a phase estimate. |
| Rewrite reproduces accidental complexity or misses a feature. | P00 behavior inventory; architecture dependency checks; explicit retirement decisions; defect register. |
| Cutover fails on a real host. | Rehearsal on the test host with the reference host's state; legacy state left untouched; rehearsed return path. |
| Subscription integration changes upstream. | Isolated versioned runtime; conformance/auth checks; retain API options without automatic spend; recheck supported terms before release. |
| A generic API abstraction loses native state or miscounts usage. | Native adapters, canonical checkpoints, continuation fixtures, recorded price/usage reconciliation. |
| Agent usefulness depends on broad shell/root access. | Prove typed diagnosis and repair early; expand registered operations based on real gaps, not unrestricted execution. |
| Core or executor compromise affects the host. | Independent root ceilings, minimal credentials, root-owned packages, bounded operations, honest threat-model limits. |
| Power failure or retries duplicate effects. | Durable intent/receipts, executor-held resource locks, operation-specific reconciliation, unknown-outcome stop, privileged fault injection. |
| Database schema prevents rollback. | Compatibility declarations, snapshots, migration rehearsal, version-checked package downgrade. |
| Claimed footprint ignores optional components. | Separate and total resource accounting for CLI, providers, local models, chat server, and media applications. |
| More time produces endless redesign. | Resolve architecture uncertainty in spikes, accept ADRs, deliver complete vertical slices, and change decisions only with evidence. |

## 8. First implementation batch

Begin with RW-001 through RW-005. Their output establishes the deployed-state inventory, baseline, accepted ADRs, and spike results. Then implement RW-010 through RW-013 and the read-only P02 slice, including the shadow install on the reference host. Do not start by moving Python files into similarly named Rust modules or adding another chat integration.

Before privileged writes ship, establish the durable operation path and root-owned packaging. Multi-provider routing, media requests, and chat transports wait until after cutover.
