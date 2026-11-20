#!/usr/bin/env bash
# shellcheck disable=SC2034  # values here are used by the scripts that source this library
# Shared helpers for the basic-cutover scripts (preflight, activate, return).
# Sourced, never executed. Prints no secrets: state files are hashed, never read
# for content, and only paths and verdicts reach the logs.
# shellcheck shell=bash

# Byte order and byte matching everywhere: sort, comm, grep, awk and [[ ]] compare paths and
# records byte for byte, never by a locale's collation (en_GB.UTF-8 collates some distinct
# names as equal, and sort -u then drops a record). Every message here is plain ASCII.
export LC_ALL=C

# Test hooks: LIMEOS_CUTOVER_ROOT prefixes every host path; CUTOVER_NOW fixes the
# clock; CUTOVER_ALLOW_NONROOT skips the root check (stub-driven tests only).
CUTOVER_ROOT="${LIMEOS_CUTOVER_ROOT:-}"
DRY_RUN=0
# The last-resort signal activate.sh sends a step that outlives TERM. Always KILL in a real run:
# CUTOVER_KILL_SIGNAL is honoured only under the stub test harness (a fake root, the non-root
# override, and not root), where it lets a test model a step that KILL cannot end in time
# (uninterruptible I/O). An inherited CUTOVER_KILL_SIGNAL=STOP cannot suspend a real step.
KILL_SIGNAL=KILL
if [[ -n "${CUTOVER_ROOT}" && "${CUTOVER_ALLOW_NONROOT:-0}" == 1 && "${EUID}" -ne 0 ]]; then
    KILL_SIGNAL="${CUTOVER_KILL_SIGNAL:-KILL}"
fi

# Frozen writers (stopped at cutover), in start order for the return path.
FROZEN_UNITS=(pihealth-helper.service pi-health.service limeos-metrics-collector.timer limeos-package-reconcile.timer)
# Units that are never touched; limeops-* is discovered at run time.
UNTOUCHED_FIXED=(limeos-agent.service flask-shutdown.service)
# Rust services in start order.
RUST_UNITS=(limeos-storaged.service limeos-containerd.service limeos-core.service)
DEFAULT_MANAGED=(jellyfin radarr sonarr sabnzbd transmission jackett vpn navidrome audiobookshelf rdtclient requestrr get_iplayer yt-to-jellyfin watchtower)
FROZEN_PORT_DEFAULT=8002
# Stack status and whole-stack actions: the dormant read-only reader (never enabled) loads this catalog.
STACK_READER=limeos-storage-reader.service
STACK_READER_SOCKET=/run/limeos-storage-reader/executor.sock
CATALOG_PATH=/etc/limeos/management-catalog.json
LIMEOSCTL="${LIMEOSCTL:-/usr/lib/limeos/limeosctl}"
CORE_SOCKET="${CORE_SOCKET:-/run/limeos-core/core.sock}"

# Every systemctl query runs under this bound (seconds), so a hung systemd becomes a named
# failure instead of an indefinite stall.
QUERY_TIMEOUT="${CUTOVER_QUERY_TIMEOUT:-30}"
[[ "${QUERY_TIMEOUT}" =~ ^[1-9][0-9]*$ ]] || QUERY_TIMEOUT=30

fct_log() { printf '%s\n' "$*"; }
fct_warn() { printf 'WARN: %s\n' "$*" >&2; }
fct_die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

fct_require_root() {
    if [[ "${CUTOVER_ALLOW_NONROOT:-0}" != 1 && "${EUID}" -ne 0 ]]; then
        fct_die 'run as root (needed to read the frozen app state and manage units)'
    fi
}

# Run a mutating command, or only print it under --dry-run.
fct_run() {
    if (( DRY_RUN )); then
        printf '[dry-run] %s\n' "$*"
        return 0
    fi
    printf '+ %s\n' "$*"
    "$@"
}

fct_now() { if [[ -n "${CUTOVER_NOW:-}" ]]; then printf '%s\n' "${CUTOVER_NOW}"; else date +%s; fi; }

fct_meta_get() { # file key
    local line
    line="$(grep -m1 -E "^${2}=" "${1}" || true)"
    printf '%s\n' "${line#*=}"
}

# Empty when the management catalog is present, a regular root-owned file closed to
# writers; otherwise one line naming the problem and the exact command that fixes it.
fct_catalog_problem() {
    local file="${CUTOVER_ROOT}${CATALOG_PATH}" owner mode
    if [[ ! -e "${file}" && ! -L "${file}" ]]; then
        printf '%s is missing; generate it with: %s migrate propose-catalog --out /root/management-catalog.proposed.json --install (or review the proposal, then: install -o root -g root -m 0644 /root/management-catalog.proposed.json %s)\n' \
            "${CATALOG_PATH}" "${LIMEOSCTL}" "${CATALOG_PATH}"
    elif [[ -L "${file}" || ! -f "${file}" ]]; then
        printf '%s is not a regular file\n' "${CATALOG_PATH}"
    else
        read -r owner mode < <(stat -c '%u %a' -- "${file}")
        if [[ "${CUTOVER_ALLOW_NONROOT:-0}" != 1 && "${owner}" != 0 ]] || (( 8#${mode} & 8#022 )); then
            printf '%s must be owned by root and not group/world writable (is %s:%s)\n' "${CATALOG_PATH}" "${owner}" "${mode}"
        fi
    fi
}

# A process's start time in clock ticks since boot (field 22 of /proc/PID/stat); empty when
# the process is gone. A pid plus its start time names one process even if the pid is reused.
fct_proc_start() { # pid
    local stat=''
    local -a fields=()
    { IFS= read -r stat <"/proc/${1}/stat"; } 2>/dev/null || return 0
    read -r -a fields <<<"${stat##*) }"
    printf '%s\n' "${fields[19]:-}"
}

# Succeeds while PID is the process that started at START (as fct_proc_start) and has not
# exited (a zombie has). Read-only, so a reused pid can only make it answer no.
fct_proc_running() { # pid start
    local stat=''
    local -a fields=()
    { IFS= read -r stat <"/proc/${1}/stat"; } 2>/dev/null || return 1
    read -r -a fields <<<"${stat##*) }"
    [[ "${fields[0]:-}" != Z && "${fields[19]:-}" == "${2}" ]]
}
fct_pause() { read -r -t 0.2 <> <(:) || true; }

# Every activation of a snapshot runs inside this transient scope (systemd-run --scope
# --unit), in attached mode too, so the kernel's cgroup, not a list of pids, says which
# processes are the activation's; return.sh --force ends it by this unit. Named from the
# first 32 hex digits of the SHA-256 of the snapshot's real path, so two snapshots never
# share a unit (62 characters, within systemd's unit-name rules). activate.sh records it in
# SNAP/activation-scope, which return.sh reads.
SCOPE_RE='^limeos-cutover-activate-[0-9a-f]{32}\.scope$'
fct_scope_unit() { # snapshot_dir
    local real digest _
    real="$(cd -P -- "${1}" && pwd -P)" || return 1
    read -r digest _ < <(printf '%s' "${real}" | sha256sum) || return 1
    [[ "${digest}" =~ ^[0-9a-f]{64}$ ]] || return 1
    printf 'limeos-cutover-activate-%s.scope\n' "${digest:0:32}"
}

# The snapshot's locks (SNAP/activate.lock on fd 9, SNAP/return.lock on fd 8) record their
# holder as one line, "ROLE PID START [UNIT]", written by whichever script takes them; an
# activation adds the transient scope it runs in.
fct_record_holder() { # lock role [unit]
    printf '%s %s %s%s\n' "${2}" "$$" "$(fct_proc_start "$$")" "${3:+ ${3}}" >"${1}"
}
# Sets HOLDER_ROLE, HOLDER_PID, HOLDER_START and HOLDER_UNIT from the record; succeeds only
# when that pid still runs with the recorded start time (otherwise the record is stale:
# exited, or the pid reused).
fct_live_holder() { # lock
    local role='' pid='' start='' unit=''
    { read -r role pid start unit <"${1}"; } 2>/dev/null || true
    HOLDER_ROLE="${role:-none}"
    HOLDER_PID="${pid:-?}"
    HOLDER_START="${start:-?}"
    HOLDER_UNIT="${unit}"
    [[ "${pid}" =~ ^[0-9]+$ && "${start}" =~ ^[0-9]+$ ]] || return 1
    [[ "$(fct_proc_start "${pid}")" == "${start}" ]]
}
# Why the lock is unavailable, naming the holder only when the record is current.
fct_lock_holder_text() { # lock
    if ! fct_live_holder "${1}"; then
        printf '%s is held, but its holder record (%s %s) is stale: that process has exited or its pid was reused, so do not signal it; find the real holder with: fuser -v %s' \
            "${1}" "${HOLDER_ROLE}" "${HOLDER_PID}" "${1}"
    elif [[ "${HOLDER_ROLE}" == activate ]]; then
        printf 'activate.sh (pid %s) is running on this snapshot%s; it rolls back by itself if it fails. To abort it now: kill -TERM %s. Only if it is hung: return.sh --force' \
            "${HOLDER_PID}" "${HOLDER_UNIT:+ in scope ${HOLDER_UNIT}}" "${HOLDER_PID}"
    else
        printf '%s.sh (pid %s) is running on this snapshot; wait for it to finish' "${HOLDER_ROLE}" "${HOLDER_PID}"
    fi
}

# One line per missing tool, so nothing is found missing halfway through a cutover.
fct_missing_requirements() { # need_python need_systemd_run (0 or 1 each)
    local tool
    for tool in flock setsid; do
        command -v "${tool}" >/dev/null 2>&1 || printf '%s is not installed (util-linux; activate.sh needs it)\n' "${tool}"
    done
    if (( ${2} )) && ! command -v systemd-run >/dev/null 2>&1; then
        printf 'systemd-run is not installed (activate.sh runs every activation, --attached too, in a transient systemd scope)\n'
    fi
    env --default-signal=INT true >/dev/null 2>&1 \
        || printf 'env does not support --default-signal (coreutils 8.31 or newer; activate.sh needs it to run detached)\n'
    if (( ${1} )) && ! command -v python3 >/dev/null 2>&1; then
        printf 'python3 is not installed (needed to read Jellyfin sessions; or pass --no-playback-check)\n'
    fi
    return 0
}

# Run a read-only query (its stderr discarded) for at most QUERY_TIMEOUT seconds; a query that
# times out is named in a warning and fails with 124.
fct_query() { # command...
    local rc=0
    timeout --kill-after=5 "${QUERY_TIMEOUT}" "$@" 2>/dev/null || rc=$?
    if (( rc == 124 || rc == 137 )); then fct_warn "timed out after ${QUERY_TIMEOUT}s: $*"; return 124; fi
    return "${rc}"
}

fct_unit_active() { fct_query systemctl is-active "${1}" || true; }
fct_unit_enabled() { fct_query systemctl is-enabled "${1}" || true; }

# limeops-* units, discovered (names only).
fct_limeops_units() {
    local name rest
    while read -r name rest || [[ -n "${name}" ]]; do
        [[ "${name}" == limeops-* ]] && printf '%s\n' "${name}"
    done < <(fct_query systemctl list-unit-files 'limeops-*' --no-legend || true)
    return 0
}

fct_untouched_units() {
    printf '%s\n' "${UNTOUCHED_FIXED[@]}"
    fct_limeops_units
}

# "<class> <unit> <active> <enabled>" for every unit we care about, sorted.
fct_unit_report() {
    local unit
    {
        for unit in "${FROZEN_UNITS[@]}" "${RUST_UNITS[@]}" caddy.service; do
            printf 'managed %s %s %s\n' "${unit}" "$(fct_unit_active "${unit}")" "$(fct_unit_enabled "${unit}")"
        done
        while read -r unit || [[ -n "${unit}" ]]; do
            [[ -n "${unit}" ]] || continue
            printf 'untouched %s %s %s\n' "${unit}" "$(fct_unit_active "${unit}")" "$(fct_unit_enabled "${unit}")"
        done < <(fct_untouched_units)
    } | sort
}

# Paths of the frozen app's state files: one absolute path per line in STATE_LIST (preflight's
# --state-list, built from the `limeosctl migrate scan` report and copied into the snapshot as
# state-paths.txt, which activate and return read). Newline-separated CUTOVER_STATE_FILES /
# CUTOVER_STATE_DIRS override the list and the directories.
STATE_LIST=''
fct_state_files() {
    if [[ -n "${CUTOVER_STATE_FILES:-}" ]]; then printf '%s\n' "${CUTOVER_STATE_FILES}"; return; fi
    [[ -n "${STATE_LIST}" ]] || return 0
    grep -v -e '^[[:space:]]*#' -e '^[[:space:]]*$' -- "${STATE_LIST}" || [[ $? == 1 ]]
}
fct_state_dirs() {
    if [[ -n "${CUTOVER_STATE_DIRS:-}" ]]; then printf '%s\n' "${CUTOVER_STATE_DIRS}"; return; fi
    printf '%s\n' /etc/limeos /var/lib/limeos
}

# "<flag> <sha256|ABSENT>" for one logical path. V marks files a running writer
# may legitimately change (SQLite and its journals); S is strict. Anything that exists
# but is not a regular file, or cannot be hashed, fails with a warning naming it.
fct_hash_one() {
    local path="${1}" flag=S out file="${CUTOVER_ROOT}${1}"
    case "${path}" in *.db|*.sqlite|*.sqlite3|*-wal|*-shm|*.db-journal) flag=V ;; esac
    if [[ ! -e "${file}" && ! -L "${file}" ]]; then printf '%s ABSENT\n' "${flag}"; return 0; fi
    if [[ -L "${file}" || ! -f "${file}" ]]; then fct_warn "state path is not a regular file: ${path}"; return 1; fi
    out="$(sha256sum <"${file}")" && [[ "${out%% *}" =~ ^[0-9a-f]{64}$ ]] || { fct_warn "cannot hash ${path}"; return 1; }
    printf '%s %s\n' "${flag}" "${out%% *}"
}

# "<flag> <hash> <path>" lines, sorted; the path is every byte after the second space
# (read them back with fct_parse_record). Paths the new package owns are skipped. Fails
# when any path cannot be hashed (the group runs in a subshell, so exit leaves only it);
# with "tolerant", prints "! <path>" for such a path instead and carries on.
fct_state_hashes() { # [tolerant]
    local path dir found record tolerant=0
    [[ "${1:-}" != tolerant ]] || tolerant=1
    {
        while IFS= read -r path || [[ -n "${path}" ]]; do
            [[ -n "${path}" ]] || continue
            if ! record="$(fct_hash_one "${path}")"; then
                (( tolerant )) || exit 1
                printf '! %s\n' "${path}"
                continue
            fi
            printf '%s %s\n' "${record}" "${path}"
        done < <(fct_state_files)
        while IFS= read -r dir || [[ -n "${dir}" ]]; do
            [[ -n "${dir}" && -d "${CUTOVER_ROOT}${dir}" ]] || continue
            while IFS= read -r -d '' found; do
                found="${found#"${CUTOVER_ROOT}"}"
                # Records are one per line, so a name with a newline cannot be recorded.
                if [[ "${found}" == *$'\n'* ]]; then
                    fct_warn "state file name contains a newline: ${found@Q}"
                    (( tolerant )) || exit 1
                    printf '! %s\n' "${found@Q}"
                    continue
                fi
                if ! record="$(fct_hash_one "${found}")"; then
                    (( tolerant )) || exit 1
                    printf '! %s\n' "${found}"
                    continue
                fi
                printf '%s %s\n' "${record}" "${found}"
            done < <(find "${CUTOVER_ROOT}${dir}" \
                \( -path "${CUTOVER_ROOT}/var/lib/limeos/core" -o -path "${CUTOVER_ROOT}/var/lib/limeos/executors" \
                -o -path "${CUTOVER_ROOT}/var/lib/limeos/assistant" -o -path "${CUTOVER_ROOT}/etc/limeos/system-policy" \) -prune \
                -o -type f ! -name core.json -print0)
        done < <(fct_state_dirs)
    } | sort -u
}

# Split one "<flag> <hash> <path>" record into REC_FLAG, REC_HASH and REC_PATH (the
# path keeps every byte after the second space). Fails on a malformed record.
fct_parse_record() { # line
    local line="${1}" rest
    [[ "${line}" == *' '*' '* ]] || return 1
    REC_FLAG="${line%% *}"
    rest="${line#* }"
    REC_HASH="${rest%% *}"
    REC_PATH="${rest#* }"
    [[ "${REC_FLAG}" == [SV] && "${REC_PATH}" == /* ]] || return 1
    [[ "${REC_HASH}" == ABSENT || "${REC_HASH}" =~ ^[0-9a-f]{64}$ ]]
}
# Every record in a hash file is well formed and the file ends in a newline (a file cut
# short is not a snapshot); warns about the first problem.
fct_records_ok() { # file
    local line
    if [[ -s "${1}" && -n "$(tail -c 1 -- "${1}")" ]]; then fct_warn "${1} does not end in a newline (cut short?)"; return 1; fi
    while IFS= read -r line || [[ -n "${line}" ]]; do
        fct_parse_record "${line}" || { fct_warn "malformed state record in ${1}: ${line}"; return 1; }
    done <"${1}"
}

fct_docker_ids() { docker ps --no-trunc --format '{{.ID}} {{.Names}}' | sort; }
fct_docker_started() {
    local ids
    mapfile -t ids < <(docker ps --no-trunc --format '{{.ID}}' | sort)
    if (( ${#ids[@]} == 0 )); then return 0; fi
    docker inspect --format '{{.Id}} {{.Name}} {{.State.StartedAt}}' "${ids[@]}" | sort
}

# Names of containers whose line differs between a snapshot file and the current
# listing (started, stopped, re-created or restarted), one per line.
fct_docker_diff_names() { # snapshot_file current_file
    comm -3 <(sort -- "${1}") <(sort -- "${2}") | awk '{ n = $2; sub(/^\//, "", n); print n }' | sort -u
}
fct_docker_changes() { # snapshot_dir
    {
        fct_docker_diff_names "${1}/docker-ps.txt" <(fct_docker_ids)
        fct_docker_diff_names "${1}/docker-started.txt" <(fct_docker_started)
    } | sort -u
}
# Filter names on stdin by the snapshot's managed set (managed.txt, written by preflight).
# A snapshot from before managed sets has none: then every container counts as managed.
fct_managed_only() { if [[ -f "${1}/managed.txt" ]]; then grep -Fxf "${1}/managed.txt" || [[ $? == 1 ]]; else cat; fi; }
fct_unmanaged_only() { if [[ -f "${1}/managed.txt" ]]; then grep -vFxf "${1}/managed.txt" || [[ $? == 1 ]]; else cat >/dev/null; fi; }
# Only the managed set must be unchanged; a change elsewhere (watchtower updating an unmanaged
# container, say) is named and otherwise ignored.
fct_warn_unmanaged_changes() { # snapshot_dir
    local name
    while read -r name || [[ -n "${name}" ]]; do
        if [[ -n "${name}" ]]; then fct_warn "container outside the managed set changed (not rolled back): ${name}"; fi
    done < <(fct_docker_changes "${1}" | fct_unmanaged_only "${1}")
}

# Snapshot must be complete, from a passing preflight, and fresh (max 0 = any age).
fct_snapshot_check() { # dir max_age_seconds
    local dir="${1}" max="${2}" meta ts age
    meta="${dir}/meta.env"
    [[ -f "${meta}" ]] || fct_die "not a preflight snapshot: ${dir}"
    [[ "$(fct_meta_get "${meta}" complete)" == 1 ]] || fct_die 'snapshot is incomplete'
    [[ "$(fct_meta_get "${meta}" preflight)" == pass ]] || fct_die 'snapshot records a refused preflight'
    if (( max > 0 )); then
        ts="$(fct_meta_get "${meta}" epoch)"
        [[ "${ts}" =~ ^[0-9]+$ ]] || fct_die 'snapshot has no valid timestamp'
        age=$(( $(fct_now) - ts ))
        if (( age < 0 || age > max )); then fct_die "snapshot is stale (${age}s old, limit ${max}s); run preflight again"; fi
    fi
}
