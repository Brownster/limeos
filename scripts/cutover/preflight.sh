#!/usr/bin/env bash
# T4: read-only pre-cutover snapshot. Writes only into the snapshot directory.
# Refuses (exit 1) on memory pressure, high swap, a stopped managed container or
# active Jellyfin playback. Hashes state files; never prints or stores contents.
set -Eeuo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
# shellcheck source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

MIN_AVAILABLE_BYTES=1610612736 # 1.5 GiB
MAX_SWAP_PERCENT=90

fct_usage() {
    cat <<'USAGE'
Usage: preflight.sh [--out DIR] [--managed-file FILE] [--frozen-port N]
                    [--jellyfin-url URL] [--help]

Read-only snapshot for the basic cutover. Default DIR is
/var/lib/limeos-cutover/<UTC timestamp>/. --managed-file lists container names
(one per line) that must be running; the default is the household set from
docs/plans/2026-11-05-basic-cutover-tooling-scope.md.
Exit 0: snapshot written and every check passed. Exit 1: refused or failed.
USAGE
}

fct_main() {
    local out='' managed_file='' port="${FROZEN_PORT_DEFAULT}" jelly='http://127.0.0.1:8096/Sessions'
    while (( $# )); do
        case "${1}" in
            --out) out="${2:?--out needs a directory}"; shift 2 ;;
            --managed-file) managed_file="${2:?--managed-file needs a path}"; shift 2 ;;
            --frozen-port) port="${2:?--frozen-port needs a number}"; shift 2 ;;
            --jellyfin-url) jelly="${2:?--jellyfin-url needs a URL}"; shift 2 ;;
            -h|--help) fct_usage; return 0 ;;
            *) fct_usage >&2; fct_die "unknown argument: ${1}" ;;
        esac
    done
    [[ "${port}" =~ ^[0-9]{1,5}$ ]] || fct_die 'invalid --frozen-port'
    fct_require_root

    local stamp epoch
    stamp="$(date -u +%Y%m%dT%H%M%SZ)"
    epoch="$(fct_now)"
    [[ -n "${out}" ]] || out="/var/lib/limeos-cutover/${stamp}"
    out="${CUTOVER_ROOT}${out}"
    if [[ -e "${out}" ]] && [[ -n "$(ls -A -- "${out}" 2>/dev/null)" ]]; then fct_die "snapshot directory is not empty: ${out}"; fi
    umask 077
    mkdir -p -- "${out}"

    local -a managed=() refusals=() warnings=()
    if [[ -n "${managed_file}" ]]; then
        local line
        while IFS= read -r line; do [[ -n "${line}" && "${line}" != \#* ]] && managed+=("${line}"); done <"${managed_file}"
    else
        managed=("${DEFAULT_MANAGED[@]}")
    fi

    fct_unit_report >"${out}/units.txt"
    ss -H -ltn >"${out}/ports.txt" || true
    fct_docker_ids >"${out}/docker-ps.txt"
    fct_docker_started >"${out}/docker-started.txt"
    fct_state_hashes >"${out}/state.sha256"

    local mem_line swap_line available swap_total swap_used
    free -b >"${out}/memory.txt"
    mem_line="$(grep -m1 '^Mem:' "${out}/memory.txt" || true)"
    swap_line="$(grep -m1 '^Swap:' "${out}/memory.txt" || true)"
    read -r _ _ _ _ _ _ available <<<"${mem_line}"
    read -r _ swap_total swap_used _ <<<"${swap_line}"
    getconf PAGESIZE >"${out}/pagesize.txt"

    {
        local proxy_found=no unit
        for unit in caddy nginx traefik; do
            printf '%s %s\n' "${unit}" "$(fct_unit_active "${unit}.service")"
            [[ "$(fct_unit_active "${unit}.service")" == active ]] && proxy_found=yes
        done
        printf 'caddy-binary %s\n' "$(command -v caddy >/dev/null 2>&1 && echo present || echo absent)"
        printf 'https-proxy-active %s\n' "${proxy_found}"
    } >"${out}/proxy.txt"
    {
        local version
        version="$(dpkg-query -W -f='${Version}' limeos 2>/dev/null || true)"
        printf 'limeos %s\n' "${version:-absent}"
    } >"${out}/packages.txt"

    # Checks.
    if [[ ! "${available}" =~ ^[0-9]+$ ]]; then
        refusals+=('cannot read MemAvailable from free')
    elif (( available < MIN_AVAILABLE_BYTES )); then
        refusals+=("MemAvailable ${available} bytes is below 1.5 GiB")
    fi
    if [[ "${swap_total:-0}" =~ ^[0-9]+$ && "${swap_used:-0}" =~ ^[0-9]+$ ]] && (( swap_total > 0 )); then
        if (( swap_used * 100 > swap_total * MAX_SWAP_PERCENT )); then
            refusals+=("swap use is above ${MAX_SWAP_PERCENT}% (${swap_used} of ${swap_total} bytes)")
        fi
    fi
    local running name
    running="$(docker ps --format '{{.Names}}' | sort)"
    for name in "${managed[@]}"; do
        grep -qx -- "${name}" <<<"${running}" || refusals+=("managed container not running: ${name}")
    done

    local code body="${out}/.jellyfin-body"
    code="$(curl -sS --max-time 5 -o "${body}" -w '%{http_code}' "${jelly}" 2>/dev/null || true)"
    if [[ "${code}" == 200 ]]; then
        if grep -q 'NowPlayingItem' "${body}"; then refusals+=('Jellyfin has active playback'); fi
    else
        warnings+=("Jellyfin playback could not be checked without credentials (HTTP ${code:-none}); check the dashboard by hand")
    fi
    rm -f -- "${body}"

    local verdict=pass msg
    (( ${#refusals[@]} == 0 )) || verdict=refused
    {
        printf 'epoch=%s\n' "${epoch}"
        printf 'timestamp=%s\n' "${stamp}"
        printf 'frozen_port=%s\n' "${port}"
        printf 'preflight=%s\n' "${verdict}"
        printf 'complete=1\n'
    } >"${out}/meta.env"
    {
        for msg in "${refusals[@]}"; do printf 'REFUSED: %s\n' "${msg}"; done
        for msg in "${warnings[@]}"; do printf 'WARN: %s\n' "${msg}"; done
    } >"${out}/findings.txt"

    fct_log "snapshot: ${out#"${CUTOVER_ROOT}"}"
    for msg in "${warnings[@]}"; do fct_warn "${msg}"; done
    if [[ "${verdict}" == refused ]]; then
        for msg in "${refusals[@]}"; do printf 'REFUSED: %s\n' "${msg}" >&2; done
        fct_die 'preflight refused; nothing was changed'
    fi
    fct_log 'preflight passed'
}

fct_main "$@"
