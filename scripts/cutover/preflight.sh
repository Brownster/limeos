#!/usr/bin/env bash
# T4: read-only pre-cutover snapshot. Writes only into the snapshot directory.
# Refuses (exit 1) on memory pressure, high swap, a stopped managed container,
# active Jellyfin playback (or no way to check it), missing state or a missing stack
# management catalog. Hashes state files; never prints or stores contents.
set -Eeuo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

MIN_AVAILABLE_BYTES=1610612736 # 1.5 GiB
MAX_SWAP_PERCENT=90

fct_usage() {
    cat <<'USAGE'
Usage: preflight.sh --state-list FILE
                    (--jellyfin-key-file FILE | --no-playback-check)
                    [--out DIR] [--managed-file FILE] [--frozen-port N]
                    [--jellyfin-url URL] [--help]

Read-only snapshot for the basic cutover. Default DIR is
/var/lib/limeos-cutover/<UTC timestamp>/. --managed-file lists container names
(one per line) that must be running; the default is the household set from
docs/plans/2026-11-05-basic-cutover-tooling-scope.md.
--state-list names the frozen app's state files, one absolute path per line (#
comments allowed), built from the `limeosctl migrate scan` report; it is copied
into the snapshot as state-paths.txt and activate.sh and return.sh hash the same
files. Every listed path must exist. Every file under /etc/limeos and
/var/lib/limeos is hashed as well.
--jellyfin-key-file names a file holding a Jellyfin API key (Dashboard > API
Keys), owned by root and readable by root only (chmod 600). The key goes to curl
on stdin, never on a command line or into the snapshot. Active playback, a
rejected key or an unreachable Jellyfin refuses. Without a key file, preflight
refuses unless --no-playback-check is given (then check the dashboard yourself).
The stack management catalog (/etc/limeos/management-catalog.json, from
`limeosctl migrate propose-catalog`) must exist: activate.sh starts the stack reader
with it. Exit 0: snapshot written and every check passed. Exit 1: refused or failed.
USAGE
}

# Empty when FILE is a regular file owned by root that only its owner can read;
# otherwise one line naming the problem.
fct_key_file_problem() { # file
    local file="${1}" owner mode
    if [[ -L "${file}" || ! -f "${file}" ]]; then
        printf '%s is not a regular file\n' "${file}"
        return 0
    fi
    read -r owner mode < <(stat -c '%u %a' -- "${file}")
    if [[ "${CUTOVER_ALLOW_NONROOT:-0}" != 1 && "${owner}" != 0 ]] || (( 8#${mode} & 8#077 )); then
        printf '%s must be owned by root and readable by root only (chmod 600; is %s:%s)\n' "${file}" "${owner}" "${mode}"
    fi
}

fct_main() {
    local out='' managed_file='' state_list='' port="${FROZEN_PORT_DEFAULT}" jelly='http://127.0.0.1:8096/Sessions'
    local key_file='' no_playback_check=0
    while (( $# )); do
        case "${1}" in
            --out) out="${2:?--out needs a directory}"; shift 2 ;;
            --managed-file) managed_file="${2:?--managed-file needs a path}"; shift 2 ;;
            --state-list) state_list="${2:?--state-list needs a path}"; shift 2 ;;
            --frozen-port) port="${2:?--frozen-port needs a number}"; shift 2 ;;
            --jellyfin-url) jelly="${2:?--jellyfin-url needs a URL}"; shift 2 ;;
            --jellyfin-key-file) key_file="${2:?--jellyfin-key-file needs a path}"; shift 2 ;;
            --no-playback-check) no_playback_check=1; shift ;;
            -h|--help) fct_usage; return 0 ;;
            *) fct_usage >&2; fct_die "unknown argument: ${1}" ;;
        esac
    done
    [[ "${port}" =~ ^[0-9]{1,5}$ ]] || fct_die 'invalid --frozen-port'
    fct_require_root
    if [[ -z "${CUTOVER_STATE_FILES:-}" ]]; then
        [[ -n "${state_list}" ]] || fct_die 'a state list is required: --state-list FILE, one absolute path per line, built from the limeosctl migrate scan report (docs/p06-cutover-runbook.md)'
        [[ -f "${state_list}" && -r "${state_list}" ]] || fct_die "state list not readable: ${state_list}"
        STATE_LIST="${state_list}"
    fi
    local -a state_paths=()
    local state_path
    mapfile -t state_paths < <(fct_state_files)
    (( ${#state_paths[@]} > 0 )) || fct_die "the state list names no paths: ${state_list}"
    for state_path in "${state_paths[@]}"; do
        [[ "${state_path}" == /* ]] || fct_die "state list entry is not an absolute path: ${state_path}"
    done

    local stamp epoch
    stamp="$(date -u +%Y%m%dT%H%M%SZ)"
    epoch="$(fct_now)"
    [[ -n "${out}" ]] || out="/var/lib/limeos-cutover/${stamp}"
    out="${CUTOVER_ROOT}${out}"
    if [[ -e "${out}" ]] && [[ -n "$(ls -A -- "${out}" 2>/dev/null)" ]]; then fct_die "snapshot directory is not empty: ${out}"; fi
    umask 077
    mkdir -p -- "${out}"
    printf '%s\n' "${state_paths[@]}" >"${out}/state-paths.txt"
    STATE_LIST="${out}/state-paths.txt"

    local -a managed=() refusals=() warnings=()
    if [[ -n "${managed_file}" ]]; then
        local line
        while IFS= read -r line; do [[ -n "${line}" && "${line}" != \#* ]] && managed+=("${line}"); done <"${managed_file}"
    else
        managed=("${DEFAULT_MANAGED[@]}")
    fi
    printf '%s\n' "${managed[@]}" >"${out}/managed.txt"

    fct_unit_report >"${out}/units.txt"
    ss -H -ltn >"${out}/ports.txt" || true
    fct_docker_ids >"${out}/docker-ps.txt"
    fct_docker_started >"${out}/docker-started.txt"
    fct_state_hashes >"${out}/state.sha256" || refusals+=('state files could not all be hashed (named above); fix the state list or the files')

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
    local record state_found=0
    while IFS= read -r record; do
        fct_parse_record "${record}" || continue
        if [[ "${REC_HASH}" == ABSENT ]]; then
            refusals+=("state path missing: ${REC_PATH} (fix the state list from limeosctl migrate scan)")
        else
            state_found=1
        fi
    done <"${out}/state.sha256"
    (( state_found )) || refusals+=("no frozen-app state found: no listed path exists and $(fct_state_dirs | paste -sd ' ' -) hold no files")
    local problem
    problem="$(fct_catalog_problem)"
    [[ -z "${problem}" ]] || refusals+=("stack management catalog: ${problem}")
    local running name
    running="$(docker ps --format '{{.Names}}' | sort)"
    for name in "${managed[@]}"; do
        grep -qx -- "${name}" <<<"${running}" || refusals+=("managed container not running: ${name}")
    done

    local code key body="${out}/.jellyfin-body"
    if (( no_playback_check )); then
        warnings+=('Jellyfin playback was not checked (--no-playback-check); check the dashboard by hand')
    elif [[ -z "${key_file}" ]]; then
        refusals+=('no Jellyfin API key: pass --jellyfin-key-file FILE (root-only) or --no-playback-check')
    elif problem="$(fct_key_file_problem "${key_file}")" && [[ -n "${problem}" ]]; then
        refusals+=("Jellyfin API key file: ${problem}")
    else
        key=''
        IFS= read -r key <"${key_file}" || true
        key="${key//[[:space:]]/}"
        if [[ ! "${key}" =~ ^[A-Za-z0-9]+$ ]]; then
            refusals+=('Jellyfin API key file must hold one key (letters and digits) on its first line')
        else
            # The key goes to curl on stdin (printf is a builtin), never on a command line.
            code="$(printf 'Authorization: MediaBrowser Token="%s"\n' "${key}" \
                | curl -sS --max-time 5 -H @- -o "${body}" -w '%{http_code}' "${jelly}" 2>/dev/null || true)"
            case "${code}" in
                200) if grep -q 'NowPlayingItem' "${body}"; then refusals+=('Jellyfin has active playback'); fi ;;
                401|403) refusals+=("Jellyfin rejected the API key (HTTP ${code})") ;;
                *) refusals+=("Jellyfin playback could not be checked (HTTP ${code:-none})") ;;
            esac
        fi
        key=''
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
