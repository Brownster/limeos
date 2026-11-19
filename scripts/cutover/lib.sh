#!/usr/bin/env bash
# shellcheck disable=SC2034  # values here are used by the scripts that source this library
# Shared helpers for the basic-cutover scripts (preflight, activate, return).
# Sourced, never executed. Prints no secrets: state files are hashed, never read
# for content, and only paths and verdicts reach the logs.
# shellcheck shell=bash

# Test hooks: LIMEOS_CUTOVER_ROOT prefixes every host path; CUTOVER_NOW fixes the
# clock; CUTOVER_ALLOW_NONROOT skips the root check (stub-driven tests only).
CUTOVER_ROOT="${LIMEOS_CUTOVER_ROOT:-}"
DRY_RUN=0

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

fct_unit_active() { systemctl is-active "${1}" 2>/dev/null || true; }
fct_unit_enabled() { systemctl is-enabled "${1}" 2>/dev/null || true; }

# limeops-* units, discovered (names only).
fct_limeops_units() {
    local name rest
    while read -r name rest; do
        [[ "${name}" == limeops-* ]] && printf '%s\n' "${name}"
    done < <(systemctl list-unit-files 'limeops-*' --no-legend 2>/dev/null || true)
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
        while read -r unit; do
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
# when any path cannot be hashed (the group runs in a subshell, so exit leaves only it).
fct_state_hashes() {
    local path dir found record
    {
        while IFS= read -r path; do
            [[ -n "${path}" ]] || continue
            record="$(fct_hash_one "${path}")" || exit 1
            printf '%s %s\n' "${record}" "${path}"
        done < <(fct_state_files)
        while IFS= read -r dir; do
            [[ -n "${dir}" && -d "${CUTOVER_ROOT}${dir}" ]] || continue
            while IFS= read -r -d '' found; do
                found="${found#"${CUTOVER_ROOT}"}"
                # Records are one per line, so a name with a newline cannot be recorded.
                if [[ "${found}" == *$'\n'* ]]; then fct_warn "state file name contains a newline: ${found@Q}"; exit 1; fi
                record="$(fct_hash_one "${found}")" || exit 1
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
# Every record in a hash file is well formed; warns about the first that is not.
fct_records_ok() { # file
    local line
    while IFS= read -r line; do
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
fct_managed_only() { grep -Fxf "${1}/managed.txt" || [[ $? == 1 ]]; }
fct_unmanaged_only() { grep -vFxf "${1}/managed.txt" || [[ $? == 1 ]]; }
# Only the managed set must be unchanged; a change elsewhere (watchtower updating an unmanaged
# container, say) is named and otherwise ignored.
fct_warn_unmanaged_changes() { # snapshot_dir
    local name
    while read -r name; do
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
    [[ -f "${dir}/state-paths.txt" ]] || fct_die 'snapshot has no state-paths.txt (taken before state lists); run preflight again'
    [[ -f "${dir}/managed.txt" ]] || fct_die 'snapshot has no managed.txt (taken before managed sets were recorded); run preflight again'
    if (( max > 0 )); then
        ts="$(fct_meta_get "${meta}" epoch)"
        [[ "${ts}" =~ ^[0-9]+$ ]] || fct_die 'snapshot has no valid timestamp'
        age=$(( $(fct_now) - ts ))
        if (( age < 0 || age > max )); then fct_die "snapshot is stale (${age}s old, limit ${max}s); run preflight again"; fi
    fi
}
