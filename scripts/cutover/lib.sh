#!/usr/bin/env bash
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

# Paths of the frozen app's state. Defaults come from the scope doc; override with
# newline-separated CUTOVER_STATE_FILES / CUTOVER_STATE_DIRS.
fct_state_files() {
    if [[ -n "${CUTOVER_STATE_FILES:-}" ]]; then printf '%s\n' "${CUTOVER_STATE_FILES}"; return; fi
    printf '%s\n' /etc/limeos/credentials.env \
        /home/holly/pi-health/config/backup_config.json \
        /home/holly/pi-health/config/media_paths.json
}
fct_state_dirs() {
    if [[ -n "${CUTOVER_STATE_DIRS:-}" ]]; then printf '%s\n' "${CUTOVER_STATE_DIRS}"; return; fi
    printf '%s\n' /etc/limeos /var/lib/limeos
}

# "<flag> <sha256|ABSENT>" for one logical path. V marks files a running writer
# may legitimately change (SQLite and its journals); S is strict.
fct_hash_one() {
    local path="${1}" flag=S out
    case "${path}" in *.db|*.sqlite|*.sqlite3|*-wal|*-shm|*.db-journal) flag=V ;; esac
    if [[ ! -e "${CUTOVER_ROOT}${path}" ]]; then printf '%s ABSENT\n' "${flag}"; return; fi
    out="$(sha256sum -- "${CUTOVER_ROOT}${path}")" || fct_die "cannot hash ${path}"
    printf '%s %s\n' "${flag}" "${out%% *}"
}

# "<flag> <hash> <path>" lines, sorted. Paths the new package owns are skipped.
fct_state_hashes() {
    local path dir found
    {
        while read -r path; do
            [[ -n "${path}" ]] || continue
            printf '%s %s\n' "$(fct_hash_one "${path}")" "${path}"
        done < <(fct_state_files)
        while read -r dir; do
            [[ -n "${dir}" && -d "${CUTOVER_ROOT}${dir}" ]] || continue
            while IFS= read -r -d '' found; do
                found="${found#"${CUTOVER_ROOT}"}"
                printf '%s %s\n' "$(fct_hash_one "${found}")" "${found}"
            done < <(find "${CUTOVER_ROOT}${dir}" \
                \( -path "${CUTOVER_ROOT}/var/lib/limeos/core" -o -path "${CUTOVER_ROOT}/var/lib/limeos/executors" \
                -o -path "${CUTOVER_ROOT}/var/lib/limeos/assistant" -o -path "${CUTOVER_ROOT}/etc/limeos/system-policy" \) -prune \
                -o -type f ! -name core.json -print0)
        done < <(fct_state_dirs)
    } | sort -u
}

fct_docker_ids() { docker ps --no-trunc --format '{{.ID}} {{.Names}}' | sort; }
fct_docker_started() {
    local ids
    mapfile -t ids < <(docker ps --no-trunc --format '{{.ID}}' | sort)
    if (( ${#ids[@]} == 0 )); then return 0; fi
    docker inspect --format '{{.Id}} {{.Name}} {{.State.StartedAt}}' "${ids[@]}" | sort
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
