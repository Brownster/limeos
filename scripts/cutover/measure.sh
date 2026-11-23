#!/usr/bin/env bash
# T7: post-activation footprint and performance against the section-10 targets of
# docs/plans/2026-10-04-rust-rewrite-architecture.md. Read-only by default: reads
# /proc, /sys/fs/cgroup, `systemctl show` and dpkg-query, times loopback HTTPS through
# the proxy, and writes only into the output directory. Never calls docker and never
# reads /opt/stacks or the frozen app's files. A missed target is reported, never acted
# on: no rollback. The one exception to read-only is the opt-in --measure-core-restart.
set -Eeuo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

# Core plus base executors (the 30 MiB PSS target); these must be running.
BASE_UNITS=("${RUST_UNITS[@]}")
# Other resident LimeOS executors, measured when running (the 60 MiB all-resident target).
EXTRA_UNITS=("${STACK_READER}" limeos-storage-targets.service limeos-storage-mounts.service)
CORE_UNIT=limeos-core.service
RAW=''

fct_usage() {
    cat <<'USAGE'
Usage: measure.sh --host HOST [--port 8002] [--out DIR] [--window SECONDS]
                  [--requests N] [--cookie-file FILE] [--measure-core-restart] [--help]

Run after activate.sh succeeds. Samples the LimeOS units (limeos-core, limeos-storaged,
limeos-containerd and any running limeos-storage-* executor) over an idle window
(default 600 s), then writes DIR/report.json and DIR/raw.txt. Default DIR is
/var/lib/limeos-cutover/measure-<UTC timestamp>/; it must be new or empty.

Measured: PSS and swap from /proc/PID/smaps_rollup, CPU from /proc/PID/stat, disk
writes from /proc/PID/io, HTTPS p95 of /api/v1/health through the proxy over
loopback (--requests, default 100), and the dpkg footprint of the limeos package.
--cookie-file FILE (a curl cookie file holding a browser session; read, never copied)
adds the authenticated /api/v1/overview p95; without it that row is unmeasured.
--measure-core-restart RESTARTS limeos-core to time readiness; off by default.

A query that times out (30 s) or does not answer (systemctl show for a unit's
processes, dpkg-query for the package) is reported as unknown, with the row's
status "unknown" and no value, never as a number or a pass.

Exit 0: every measured target passed. Exit 2: a target failed, a required process
was missing or a row is unknown (report still written; nothing is rolled back).
Exit 1: error.
USAGE
}

# PIDs of a unit: its cgroup members, else MainPID; nothing when it is not running. Fails with 2
# when systemd did not answer: then the unit's processes are unknown, and neither its main process
# alone nor "not running" stands in for them.
fct_unit_pids() {
    local unit="${1}" cg pid file
    local -a pids=()
    cg="$(fct_unit_property ControlGroup "${unit}")" || return 2
    if [[ "${cg}" =~ ^/[A-Za-z0-9@._/-]+$ && "${cg}" != *..* ]]; then
        file="${CUTOVER_ROOT}/sys/fs/cgroup${cg}/cgroup.procs"
        if [[ -r "${file}" ]]; then
            while read -r pid; do [[ "${pid}" =~ ^[0-9]+$ ]] && pids+=("${pid}"); done <"${file}"
        fi
    fi
    if (( ${#pids[@]} == 0 )); then
        pid="$(fct_unit_property MainPID "${unit}")" || return 2
        [[ "${pid}" =~ ^[0-9]+$ ]] || return 2
        [[ "${pid}" != 0 && -d "${CUTOVER_ROOT}/proc/${pid}" ]] && pids+=("${pid}")
    fi
    if (( ${#pids[@]} )); then printf '%s\n' "${pids[@]}"; fi
}

# "<starttime> <utime+stime+cutime+cstime> <write_bytes|NA>"; nothing once the process is gone.
fct_cpu_io() {
    local pid="${1}" stat rest wb
    local -a field
    stat="$(cat -- "${CUTOVER_ROOT}/proc/${pid}/stat" 2>/dev/null)" || return 0
    rest="${stat##*) }"
    read -ra field <<<"${rest}"
    (( ${#field[@]} >= 20 )) || return 0
    wb="$(awk '$1 == "write_bytes:" { print $2 }' "${CUTOVER_ROOT}/proc/${pid}/io" 2>/dev/null || true)"
    [[ "${wb}" =~ ^[0-9]+$ ]] || wb=NA
    printf '%s %s %s\n' "${field[19]}" "$(( field[11] + field[12] + field[13] + field[14] ))" "${wb}"
}

# "<pss_kB> <swap_kB> <swappss_kB|NA>" from smaps_rollup; nothing when unreadable.
fct_mem() {
    awk '/^Pss:/ { p = $2 } /^Swap:/ { s = $2 } /^SwapPss:/ { sp = $2 }
        END { if (p == "") exit 1; printf "%s %s %s\n", p, (s == "" ? 0 : s), (sp == "" ? "NA" : sp) }' \
        "${CUTOVER_ROOT}/proc/${1}/smaps_rollup" 2>/dev/null || true
}

fct_uptime() { local up _; read -r up _ <"${CUTOVER_ROOT}/proc/uptime"; printf '%s\n' "${up}"; }

# One sample of every unit: "sample PHASE UNIT PID START TICKS WRITE_BYTES" plus missing/absent
# lines, and "unknown PHASE UNIT" for a unit whose processes systemd did not report.
fct_sample() {
    local phase="${1}" unit pid line comm listed
    local -a pids
    printf 'uptime %s %s\n' "${phase}" "$(fct_uptime)"
    for unit in "${BASE_UNITS[@]}" "${EXTRA_UNITS[@]}"; do
        if ! listed="$(fct_unit_pids "${unit}")"; then
            printf 'unknown %s %s\n' "${phase}" "${unit}"
            continue
        fi
        mapfile -t pids < <(fct_lines "${listed}")
        if (( ${#pids[@]} == 0 )); then
            if [[ " ${BASE_UNITS[*]} " == *" ${unit} "* ]]; then printf 'missing %s %s\n' "${phase}" "${unit}"
            else printf 'absent %s %s\n' "${phase}" "${unit}"; fi
            continue
        fi
        for pid in "${pids[@]}"; do
            line="$(fct_cpu_io "${pid}")"
            if [[ -n "${line}" ]]; then printf 'sample %s %s %s %s\n' "${phase}" "${unit}" "${pid}" "${line}"; fi
            [[ "${phase}" == end ]] || continue
            line="$(fct_mem "${pid}")"
            [[ -n "${line}" ]] || continue
            comm="$(cat -- "${CUTOVER_ROOT}/proc/${pid}/comm" 2>/dev/null || echo '?')"
            printf 'mem %s %s %s %s\n' "${unit}" "${pid}" "${line}" "${comm//[^A-Za-z0-9._-]/_}"
        done
    done
    return 0
}

# "http LABEL CODE TOTAL APPCONNECT CONNECT" per request over loopback; curl writes the body to /dev/null.
fct_http() { # label host port url requests [extra curl args...]
    local label="${1}" host="${2}" port="${3}" url="${4}" count="${5}" i line
    shift 5
    for (( i=0; i<count; i++ )); do
        line="$(curl -sS -k --max-time 5 --resolve "${host}:${port}:127.0.0.1" "$@" -o /dev/null \
            -w '%{http_code} %{time_total} %{time_appconnect} %{time_connect}' "${url}" 2>/dev/null || true)"
        printf 'http %s %s\n' "${label}" "${line:-000 NA NA NA}"
    done
}

# "pkg NAME STATUS VERSION INSTALLED_SIZE_KIB FILE_BYTES FILE_COUNT", "pkg NAME absent", or
# "pkg NAME unknown" when dpkg-query could not answer (exit 2 or a timeout; exit 1 is not found).
fct_pkg() {
    local pkg="${1}" info status version size path total=0 count=0 bytes rc=0 listing
    local -a files=()
    # shellcheck disable=SC2016  # a dpkg-query format, expanded by dpkg-query
    info="$(fct_query dpkg-query -W -f='${db:Status-Status} ${Version} ${Installed-Size}' "${pkg}")" || rc=$?
    if (( rc != 0 && rc != 1 )); then printf 'pkg %s unknown\n' "${pkg}"; return 0; fi
    read -r status version size <<<"${info}"
    if [[ "${status:-}" != installed ]]; then printf 'pkg %s absent\n' "${pkg}"; return 0; fi
    # The installed package's file list: one that cannot be read leaves the footprint unknown.
    listing="$(fct_query dpkg-query -L "${pkg}")" || { printf 'pkg %s unknown\n' "${pkg}"; return 0; }
    while IFS= read -r path; do
        [[ -f "${CUTOVER_ROOT}${path}" && ! -L "${CUTOVER_ROOT}${path}" ]] && files+=("${CUTOVER_ROOT}${path}")
    done < <(fct_lines "${listing}")
    if (( ${#files[@]} )); then
        while read -r bytes; do total=$(( total + bytes )); count=$(( count + 1 )); done < <(stat -c '%s' -- "${files[@]}")
    fi
    printf 'pkg %s installed %s %s %s %s\n' "${pkg}" "${version:-NA}" "${size:-NA}" "${total}" "${count}"
}

# Opt-in only: restart core and time until `limeosctl status` answers (conservative: includes the stop).
fct_core_restart() {
    local start now deadline
    start="$(date +%s%N)"
    deadline=$(( start + 30000000000 ))
    if ! fct_run systemctl restart "${CORE_UNIT}" >&2; then printf 'restart NA systemctl restart failed\n'; return 0; fi
    while :; do
        if "${LIMEOSCTL}" --socket "${CORE_SOCKET}" status >/dev/null 2>&1; then
            now="$(date +%s%N)"
            printf 'restart %s ok\n' "$(( (now - start) / 1000000 ))"
            return 0
        fi
        (( $(date +%s%N) < deadline )) || { printf 'restart NA not ready within 30 s\n'; return 0; }
        sleep 0.02
    done
}

fct_main() {
    local out='' host='' port="${FROZEN_PORT_DEFAULT}" window=600 requests=100 cookie='' restart=0
    while (( $# )); do
        case "${1}" in
            --out) out="${2:?--out needs a directory}"; shift 2 ;;
            --host) host="${2:?--host needs a value}"; shift 2 ;;
            --port) port="${2:?--port needs a number}"; shift 2 ;;
            --window) window="${2:?--window needs seconds}"; shift 2 ;;
            --requests) requests="${2:?--requests needs a number}"; shift 2 ;;
            --cookie-file) cookie="${2:?--cookie-file needs a path}"; shift 2 ;;
            --measure-core-restart) restart=1; shift ;;
            -h|--help) fct_usage; return 0 ;;
            *) fct_usage >&2; fct_die "unknown argument: ${1}" ;;
        esac
    done
    [[ -n "${host}" ]] || { fct_usage >&2; fct_die '--host is required (the name activate.sh used)'; }
    [[ "${host}" =~ ^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?$ ]] || fct_die 'invalid --host (letters, digits, dots, dashes; IPv4 allowed)'
    [[ "${port}" =~ ^[0-9]{2,5}$ ]] || fct_die 'invalid --port'
    if [[ ! "${window}" =~ ^[0-9]{1,6}$ ]] || (( 10#${window} < 1 )); then fct_die 'invalid --window (whole seconds, at least 1)'; fi
    if [[ ! "${requests}" =~ ^[0-9]{1,5}$ ]] || (( 10#${requests} < 1 )); then fct_die 'invalid --requests'; fi
    window=$(( 10#${window} ))
    requests=$(( 10#${requests} ))
    if [[ -n "${cookie}" ]]; then
        [[ "${cookie}" =~ ^/[A-Za-z0-9._/-]+$ ]] || fct_die '--cookie-file must be a plain absolute path'
        [[ -r "${CUTOVER_ROOT}${cookie}" ]] || fct_die "cookie file not readable: ${cookie}"
    fi
    command -v python3 >/dev/null || fct_die 'python3 is required to write the report'
    fct_require_root

    local stamp epoch site="https://${host}:${port}"
    stamp="$(date -u +%Y%m%dT%H%M%SZ)"
    epoch="$(fct_now)"
    [[ -n "${out}" ]] || out="/var/lib/limeos-cutover/measure-${stamp}"
    out="${CUTOVER_ROOT}${out}"
    if [[ -e "${out}" ]] && [[ -n "$(ls -A -- "${out}" 2>/dev/null)" ]]; then fct_die "output directory is not empty: ${out}"; fi
    umask 077
    mkdir -p -- "${out}"
    RAW="${out}/raw.txt"

    local scopes=0 scope
    for scope in "${CUTOVER_ROOT}"/sys/fs/cgroup/system.slice/docker-*.scope; do [[ -d "${scope}" ]] && scopes=$(( scopes + 1 )); done
    {
        printf 'meta epoch %s\n' "${epoch}"
        printf 'meta timestamp %s\n' "${stamp}"
        printf 'meta hostname %s\n' "$(uname -n)"
        printf 'meta site %s\n' "${site}"
        printf 'meta clk_tck %s\n' "$(getconf CLK_TCK)"
        printf 'meta window_s %s\n' "${window}"
        printf 'meta requests %s\n' "${requests}"
        printf 'meta docker_scopes %s\n' "${scopes}"
        printf 'meta base_units %s\n' "${BASE_UNITS[*]}"
        printf 'meta extra_units %s\n' "${EXTRA_UNITS[*]}"
        awk '$1 == "SwapTotal:" || $1 == "SwapFree:" || $1 == "MemAvailable:" { sub(":", "", $1); print "meta host_" $1 "_kB", $2 }' \
            "${CUTOVER_ROOT}/proc/meminfo" 2>/dev/null || true
    } >"${RAW}"

    fct_log "measure: idle window ${window}s, output ${out#"${CUTOVER_ROOT}"}"
    fct_sample start >>"${RAW}"
    sleep "${window}"
    fct_sample end >>"${RAW}"

    fct_log "measure: ${requests} loopback requests to ${site}/api/v1/health"
    fct_http health "${host}" "${port}" "${site}/api/v1/health" "${requests}" >>"${RAW}"
    if [[ -n "${cookie}" ]]; then
        fct_http overview "${host}" "${port}" "${site}/api/v1/overview" "${requests}" -b "${CUTOVER_ROOT}${cookie}" >>"${RAW}"
    fi
    { fct_pkg limeos; fct_pkg limeos-shadow; } >>"${RAW}"
    if (( restart )); then
        fct_warn "--measure-core-restart: restarting ${CORE_UNIT}"
        fct_core_restart >>"${RAW}"
    fi

    local code=0
    python3 -I - "${RAW}" "${out}/report.json" <<'PY' || code=$?
import json
import math
import sys

raw_path, report_path = sys.argv[1], sys.argv[2]
MIB = 1024 * 1024
# Section 10 targets.
CORE_PSS = 30 * MIB          # core plus base executors, PSS combined
ALL_PSS = 60 * MIB           # all resident LimeOS processes, PSS combined
CPU_PCT = 0.1                # percent of one core over the idle window
HTTP_MS = 20.0               # cached observation p95, excluding network transit
DISK_MB_DAY = 20.0           # resident processes, 1 MB = 10^6 bytes
FOOTPRINT = 154_000_000      # installed bytes, 1 MB = 10^6 bytes (stricter than MiB)
READY_MS = 1000.0            # core ready after a restart

meta, samples, uptime, mem = {}, {"start": {}, "end": {}}, {}, []
missing, absent, http, pkgs, restart = {"start": set(), "end": set()}, set(), {}, {}, None
unknown = set()
with open(raw_path) as handle:
    for line in handle:
        f = line.split()
        if not f:
            continue
        kind = f[0]
        if kind == "meta":
            meta[f[1]] = " ".join(f[2:])
        elif kind == "uptime":
            uptime[f[1]] = float(f[2])
        elif kind == "sample":
            phase, unit, pid, start, ticks, wb = f[1:7]
            samples[phase][(unit, pid, start)] = (int(ticks), None if wb == "NA" else int(wb))
        elif kind == "mem":
            unit, pid, pss, swap, swappss = f[1:6]
            mem.append({"unit": unit, "pid": int(pid), "comm": " ".join(f[6:]),
                        "pss_bytes": int(pss) * 1024, "swap_bytes": int(swap) * 1024,
                        "swap_pss_bytes": None if swappss == "NA" else int(swappss) * 1024})
        elif kind == "missing":
            missing[f[1]].add(f[2])
        elif kind == "absent":
            absent.add(f[2])
        elif kind == "unknown":
            unknown.add(f[2])
        elif kind == "http":
            http.setdefault(f[1], []).append(f[2:6])
        elif kind == "pkg":
            pkgs[f[1]] = f[2:]
        elif kind == "restart":
            restart = (f[1], " ".join(f[2:]))

base = meta.get("base_units", "").split()
clk = int(meta.get("clk_tck", "100"))
window = float(meta["window_s"])
gone = sorted(missing["start"] | missing["end"])
measurements = []


def add(ident, description, source, unit, target, comparison, value=None, reason=None, fail=None,
        unknown_note=None, **extra):
    # A row whose inputs could not be determined is "unknown", never a value or a pass.
    if unknown_note:
        status, value, reason, fail = "unknown", None, unknown_note, None
    elif fail:
        status = "fail"
    elif value is None:
        status = "unmeasured"
    elif comparison == "<=":
        status = "pass" if value <= target else "fail"
    elif comparison == "<":
        status = "pass" if value < target else "fail"
    else:
        status = "pass" if value == target else "fail"
    row = {"id": ident, "description": description, "value": value, "unit": unit,
           "target": target, "comparison": comparison, "status": status, "source": source}
    if fail or reason:
        row["note"] = fail or reason
    if unknown_note:
        row["note"] = unknown_note
    row.update(extra)
    measurements.append(row)


not_running = f"required process not running: {', '.join(gone)}" if gone else None


def unknown_text(units):
    found = sorted(set(units) & unknown)
    return (f"unknown: could not determine the processes of {', '.join(found)} "
            "(systemctl show did not answer)") if found else None


unknown_base = unknown_text(base)
unknown_any = unknown_text(unknown)

# Memory, sampled at the end of the idle window.
base_mem = [m for m in mem if m["unit"] in base]
add("core_base_idle_pss", "Core plus base executors, idle PSS combined", "/proc/PID/smaps_rollup Pss",
    "bytes", CORE_PSS, "<=", None if gone else sum(m["pss_bytes"] for m in base_mem), not_running,
    unknown_note=unknown_base)
add("core_base_idle_swap", "Core plus base executors, idle swap (no steady-state swap)", "/proc/PID/smaps_rollup Swap",
    "bytes", 0, "==", None if gone else sum(m["swap_bytes"] for m in base_mem), not_running,
    unknown_note=unknown_base,
    swap_pss_bytes=None if gone or unknown_base else sum(m["swap_pss_bytes"] or 0 for m in base_mem))
add("all_resident_idle_pss", "All resident LimeOS processes, idle PSS combined", "/proc/PID/smaps_rollup Pss",
    "bytes", ALL_PSS, "<=", None if gone else sum(m["pss_bytes"] for m in mem), not_running,
    unknown_note=unknown_any,
    units=sorted({m["unit"] for m in mem}), not_running_optional=sorted(absent))

# CPU and disk writes over the idle window, for processes present at both ends.
if "start" in uptime and "end" in uptime and uptime["end"] > uptime["start"]:
    elapsed, elapsed_source = uptime["end"] - uptime["start"], "/proc/uptime"
else:
    elapsed, elapsed_source = window, "requested window (uptime did not advance)"
matched = sorted(set(samples["start"]) & set(samples["end"]))
changed = sorted(set(samples["start"]) ^ set(samples["end"]))
change_note = (f"process set changed during the window: {', '.join(f'{u}:{p}' for u, p, _ in changed)}"
               if changed else None)
cpu = None
if not gone:
    ticks = sum(samples["end"][k][0] - samples["start"][k][0] for k in matched)
    cpu = round(ticks / clk / elapsed * 100, 4)
lifetime = 0.0
for unit, pid, start in matched:
    age = uptime.get("end", 0) - int(start) / clk
    if age > 0:
        lifetime += samples["end"][(unit, pid, start)][0] / clk / age * 100
add("backend_cpu_idle", "Backend CPU over the idle window, percent of one core", "/proc/PID/stat utime+stime+cutime+cstime",
    "percent", CPU_PCT, "<", cpu, not_running or change_note, unknown_note=unknown_any,
    elapsed_s=round(elapsed, 3), elapsed_source=elapsed_source,
    since_start_percent=round(lifetime, 4))
disk = None
disk_reason = not_running or change_note
if not gone:
    if any(samples[p][k][1] is None for p in ("start", "end") for k in matched):
        disk_reason = "write_bytes unreadable (run as root)"
    else:
        written = sum(samples["end"][k][1] - samples["start"][k][1] for k in matched)
        disk = round(written / elapsed * 86400 / 1e6, 3)
add("resident_disk_writes", "Disk writes of resident LimeOS processes, extrapolated", "/proc/PID/io write_bytes",
    "MB/day", DISK_MB_DAY, "<", disk, disk_reason, unknown_note=unknown_any, elapsed_s=round(elapsed, 3))


def p95(label, ident, description, url_path, absent_reason):
    rows = http.get(label)
    source = f"curl over loopback through the proxy, GET {url_path}, time_total minus TCP/TLS setup"
    if not rows:
        add(ident, description, source, "ms", HTTP_MS, "<", None, absent_reason)
        return
    ok, codes = [], {}
    for code, total, app, conn in rows:
        codes[code] = codes.get(code, 0) + 1
        if code.startswith("2") and total != "NA":
            ok.append((float(total) - max(float(app), float(conn))) * 1000)
    total_ms = sorted(float(r[1]) * 1000 for r in rows if r[0].startswith("2") and r[1] != "NA")
    def nearest(values):
        return round(values[max(0, math.ceil(0.95 * len(values)) - 1)], 3) if values else None
    extra = {"requests": len(rows), "codes": codes, "p95_total_ms": nearest(total_ms)}
    if label == "overview" and not ok and set(codes) <= {"401", "403"}:
        add(ident, description, source, "ms", HTTP_MS, "<", None,
            f"session rejected (HTTP {'/'.join(sorted(codes))}); supply a fresh --cookie-file", **extra)
        return
    failed = len(rows) - len(ok)
    add(ident, description, source, "ms", HTTP_MS, "<", nearest(sorted(ok)),
        fail=f"{failed} of {len(rows)} requests failed" if failed else None, **extra)


p95("health", "health_http_p95", "Unauthenticated /api/v1/health p95 through the proxy (loopback floor)",
    "/api/v1/health", "not measured")
p95("overview", "cached_observation_http_p95", "Authenticated cached observation /api/v1/overview p95 through the proxy",
    "/api/v1/overview",
    "unmeasured: needs a browser session; limeosctl offers no authenticated HTTP read. Rerun with --cookie-file")

packages = {}
for name, fields in pkgs.items():
    if fields[0] == "installed":
        version, size, files, count = fields[1:5]
        packages[name] = {"version": version, "installed_size_bytes": None if size == "NA" else int(size) * 1024,
                          "file_bytes": int(files), "file_count": int(count)}
    elif fields[0] == "unknown":
        packages[name] = {"status": "unknown"}
footprint, reason, footprint_unknown = None, "limeos package not installed", None
if packages.get("limeos", {}).get("status") == "unknown":
    footprint_unknown = "unknown: could not determine the limeos package's state or files (dpkg-query did not answer)"
elif "limeos" in packages:
    entry = packages["limeos"]
    footprint, reason = max(entry["installed_size_bytes"] or 0, entry["file_bytes"]), None
add("installed_footprint", "Installed footprint of the limeos package (larger of dpkg Installed-Size and installed files)",
    "dpkg-query Installed-Size and -L with file sizes", "bytes", FOOTPRINT, "<", footprint, reason,
    unknown_note=footprint_unknown, packages=packages)

if restart is None:
    add("core_restart_readiness", "Core ready after a restart", "systemctl restart then limeosctl status", "ms",
        READY_MS, "<", None, "not requested; opt in with --measure-core-restart (restarts limeos-core)")
elif restart[0] == "NA":
    add("core_restart_readiness", "Core ready after a restart", "systemctl restart then limeosctl status", "ms",
        READY_MS, "<", None, fail=restart[1])
else:
    add("core_restart_readiness", "Core ready after a restart (includes the stop)", "systemctl restart then limeosctl status",
        "ms", READY_MS, "<", float(restart[0]))

required = {"core_base_idle_pss", "core_base_idle_swap", "backend_cpu_idle", "resident_disk_writes",
            "health_http_p95", "installed_footprint"}
if any(m["status"] == "fail" for m in measurements):
    verdict = "fail"
elif gone or unknown or any(m["status"] in ("unmeasured", "unknown") for m in measurements if m["id"] in required):
    verdict = "incomplete"
else:
    verdict = "pass"

processes = []
for m in [m for m in mem if m["unit"] not in unknown]:
    key = next((k for k in matched if k[0] == m["unit"] and k[1] == str(m["pid"])), None)
    entry = dict(m)
    if key:
        entry["cpu_ticks_delta"] = samples["end"][key][0] - samples["start"][key][0]
        wb0, wb1 = samples["start"][key][1], samples["end"][key][1]
        entry["write_bytes_delta"] = None if wb0 is None or wb1 is None else wb1 - wb0
    processes.append(entry)

report = {
    "schema": "limeos-cutover-measure/1",
    "targets_source": "docs/plans/2026-10-04-rust-rewrite-architecture.md section 10",
    "read_only": restart is None,
    "meta": meta,
    "verdict": verdict,
    "missing_units": gone,
    "unknown_units": sorted(unknown),
    "processes": processes,
    "measurements": measurements,
}
with open(report_path, "w") as handle:
    json.dump(report, handle, indent=2)
    handle.write("\n")
for m in measurements:
    print(f"{m['status'].upper():10} {m['id']}: {m['value']} {m['unit']} (target {m['comparison']} {m['target']})"
          + (f" - {m['note']}" if "note" in m else ""))
print(f"verdict: {verdict}")
sys.exit(0 if verdict == "pass" else 2)
PY
    fct_log "report: ${out#"${CUTOVER_ROOT}"}/report.json"
    if (( code == 2 )); then fct_warn 'targets missed, unmeasured or unknown; nothing was rolled back, the operator decides'; exit 2; fi
    (( code == 0 )) || fct_die "report generation failed (python3 exit ${code})"
}

fct_main "$@"
