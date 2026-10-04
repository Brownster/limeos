# P00 baseline: reference Raspberry Pi 5

Measured: 2026-10-04, read-only, over SSH. Supports RW-002 and the "Why Rust" rationale in the [target architecture](../../plans/2026-10-04-rust-rewrite-architecture.md#why-rust).

No service was restarted, reconfigured, or logged into. Memory and I/O come from `/proc/<pid>/smaps_rollup` and `/proc/<pid>/io`; CPU from systemd `CPUUsageNSec`; timings from the shell `time` builtin and `curl -w %{time_total}` on loopback.

## Host

| Item | Value |
|---|---|
| Hardware | Raspberry Pi 5 Model B Rev 1.0, 4 cores, 8 GB RAM, 512 MiB swap |
| OS | Debian 12 (bookworm), kernel 6.12.25+rpt-rpi-2712, aarch64 |
| OS disk | NVMe (KIOXIA 256 GB); data disks on USB |
| Deployed LimeOS | `618ce92` (2026-07-26), Python 3.11.2, services running 69.9 days at measurement |
| Workload | 18 running containers, including Jellyfin, the *arr apps, Mattermost and its PostgreSQL |
| Memory cgroup | Inactive (host up 98 days, cmdline fix not yet applied by reboot); per-unit memory accounting unavailable |

## Resident memory, LimeOS backend

| Process | Account | RSS MiB | PSS MiB | Swap MiB |
|---|---|---:|---:|---:|
| Dashboard (`app.py`) | holly | 62.9 | 54.9 | 15.6 |
| Privileged helper (`pihealth_helper.py`) | root | 19.7 | 12.3 | 15.9 |
| Action worker (`agent_actions.worker`) | limeops | 50.4 | 43.3 | 0.2 |
| Supervised repair (`agent_supervision.runner`) | limeops | 50.1 | 42.6 | 3.3 |
| Report scheduler (`agent_automation.runner`) | limeops | 40.6 | 33.1 | 7.9 |
| Mattermost assistant (`agent_runtime`) | lime-agent | 11.2 | 4.8 | 9.4 |
| Action broker (`agent_actions.server`) | limeops | 8.3 | 0.9 | 34.6 |
| Read-only broker (`limeops.server`) | limeops | 7.7 | 0.8 | 13.0 |
| **Total, 8 processes** | | **251.0** | **192.6** | **99.9** |

Dashboard plus helper, the equivalent of the planned core and base executors: 67.2 MiB PSS plus 31.5 MiB swap, about 99 MiB. LimeOS accounts for 100 MiB of the 321 MiB of swap in use system-wide.

Per-process interpreter cost: bare Python starts in 18 ms at 7.1 MiB. Importing the application's libraries (`flask`, `docker`, `psutil`, `yaml`, `jsonschema`, `apscheduler`, `ruamel.yaml`) takes 0.54–0.73 s and raises peak RSS to 41.4 MiB, so each Python process pays about 34 MiB before doing any work.

## Optional runtimes (not LimeOS code)

| Runtime | Measurement |
|---|---|
| Mattermost server | 117.8 MiB RSS |
| Mattermost PostgreSQL | 116.8 MiB summed RSS over 8 processes (overcounts shared buffers) |
| Claude CLI 2.1.218 | 270 MB executable at `/usr/bin/claude`; not running at measurement, so per-turn RSS is unmeasured |

## CPU

Cumulative CPU per unit over the 69.9 days since 2026-07-26 18:50:55, including child processes in each unit's cgroup and including real use, not just idle:

| Unit | CPU seconds | Average, % of one core |
|---|---:|---:|
| `pi-health` | 17,022 | 0.282 |
| `limeops-action-worker` | 4,056 | 0.067 |
| `pihealth-helper` | 2,661 | 0.044 |
| `limeops-supervised-repair` | 786 | 0.013 |
| `limeops-report-scheduler` | 452 | 0.007 |
| `limeos-agent`, `limeops-actuatord`, `limeopsd` | 24 | < 0.001 |
| **Total** | **25,001** | **0.414** |

`limeos-metrics-collector.timer` starts a new Python process about every 5 minutes: 218 ms wall time and 111 ms CPU per run, about 32 CPU-seconds a day.

## Latency

| Path | Median | Range |
|---|---:|---:|
| `GET /` (static UI) | 1.8 ms | 1.7–3.7 ms |
| `GET /api/overview` without session (401) | 1.5 ms | 1.5–2.1 ms |
| Unknown path (404) | 1.8 ms | 1.6–3.0 ms |

These show only the framework floor: Werkzeug's development server (`create_app().run(...)`) on loopback. Authenticated endpoint latency was not measured.

External commands that handlers wait on: `docker ps -q` 22 ms, `docker compose ls` 98 ms, `lsblk -J` 21 ms, `df -P` 7 ms.

## Disk writes

| Process | Written over 69.9 days | Per day | Notes |
|---|---:|---:|---|
| Supervised repair runner | 13.2 GB | 189 MB | Confirmed by a 60-second sample (131 KB). Its SQLite file is 135 KB, so this is commit amplification. |
| Dashboard | 14.0 MB | 0.2 MB | |
| Privileged helper | 155.8 GB | 2.2 GB | Includes reaped child processes (daily `backup_create`, protection jobs); attribute per job in P00 before using it as a target. |
| Action worker, report scheduler | 0 in 60-second sample | | |

## Installed size

Checkout without `.git`, `node_modules`, and `.venv`: 21 MB. Virtual environments: dashboard 47 MB, supervised repair 30 MB, report scheduler 30 MB, assistant 26 MB. About 154 MB in total, plus system Python and the Claude CLI.

## Security observations made while measuring

- `pihealth-helper.service` runs `/usr/bin/python3 /usr/local/bin/pihealth_helper.py` as root. That path is a symlink to `/home/holly/pi-health/pihealth_helper.py`, owned `holly:holly`, mode `0660`. The dashboard runs as `holly`, so code running as the dashboard account can change code that root executes.
- `holly` is in the `docker` group, so the dashboard account already has root-equivalent Docker access.
- The deployed checkout has local modifications to `config/backup_config.json` and `config/media_paths.json`; the migration inventory must read configuration from the checkout as well as `/etc/limeos`.

## Not yet measured

- Authenticated endpoint latency (`/api/overview`, containers, disks, storage) at p50/p95 with a test account.
- Dashboard readiness time after a restart.
- Claude CLI RSS and wall time during an assistant turn.
- A Raspberry Pi 4 with an SD-card OS disk, where write volume matters for wear, and an x86-64 host.
- Per-container memory, which needs the memory cgroup active.
- Per-job attribution of the helper's child-process I/O.
