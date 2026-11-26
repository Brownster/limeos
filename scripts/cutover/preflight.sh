#!/usr/bin/env bash
# T4: read-only pre-cutover snapshot. Writes only into the snapshot directory.
# Refuses (exit 1) on memory pressure, high swap, a stopped managed container,
# active Jellyfin playback (or no way to check it), missing state, a missing stack
# management catalog, or any query (systemd, docker, dpkg, ss) that did not answer: the
# snapshot never records a blank or guessed state. Hashes state files; never prints or
# stores contents.
set -Eeuo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

MIN_AVAILABLE_BYTES=1610612736 # 1.5 GiB
MAX_SWAP_PERCENT=90

fct_usage() {
    cat <<'USAGE'
Usage: preflight.sh --state-list FILE [--live-list FILE]
                    (--jellyfin-key-file FILE | --no-playback-check)
                    [--host HOST --package PATH --expected-version VERSION]
                    [--site-template FILE] [--caddyfile FILE] [--site-dir DIR]
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
--live-list names state that services the cutover leaves running write (the
limeops-* homes, files a schedule outside the frozen app rewrites): one absolute
path per line, a path ending in / for a whole directory, a # comment line giving
the reason for the entries after it. Those paths are hashed and any change is
reported by return.sh, never failed on. An entry must exist, must not be a
shared directory as a whole, and must not cover a path of the state list.
Everything activate.sh needs is checked here too, so nothing is found missing in
the window: the scripts next to this one (and SHA256SUMS, in a directory made by
bundle.sh), the Caddy site template (--site-template, default: next to the
scripts, else the repository's packaging/ copy), caddy and the import line in
the Caddyfile; with --host, that core.json's origin is https://HOST:PORT (PORT is
--frozen-port); with --package and --expected-version, the .deb's fields.
Pass the same values activate.sh will get.
The frozen services that declare StateDirectory=limeos (systemd chowns
/var/lib/limeos to them at every start) are recorded in state-claimants.txt and
the owner and mode of /etc/limeos and /var/lib/limeos in shared-dirs.txt:
activate.sh takes /var/lib/limeos over from them and return.sh gives it back.
--jellyfin-key-file names a file holding a Jellyfin API key (Dashboard > API
Keys), owned by root and readable by root only (chmod 600). The key goes to curl
on stdin, never on a command line or into the snapshot. Only a complete HTTP 200
whose body is a JSON list of sessions, none playing, passes (python3 parses it);
active playback, a rejected key, a failed or cut-short transfer or any other
body refuses. Without a key file, preflight
refuses unless --no-playback-check is given (then check the dashboard yourself).
The stack management catalog (/etc/limeos/management-catalog.json, from
`limeosctl migrate propose-catalog`) must exist: activate.sh starts the stack reader
with it. Without the cgroup2 unified hierarchy and cgroup.kill (kernel 5.14 or
newer) preflight warns but passes: return.sh --force could then not end a hung
activation by its scope and would stop without killing anything unless given
--allow-systemctl-kill-fallback. Every unit state, the container list, the limeos package state and the
listening ports are recorded as found; a query that times out (30 s) or gives no
answer refuses, naming it, so a snapshot never holds a blank or guessed state.
Exit 0: snapshot written and every check passed. Exit 1: refused or failed.
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

# "idle", "active" or "invalid" for a saved Jellyfin /Sessions body.
fct_sessions_verdict() { # file
    python3 -I -c '
import json, sys
try:
    sessions = json.load(open(sys.argv[1], encoding="utf-8"))
except Exception:
    sessions = None
if not isinstance(sessions, list) or not all(isinstance(s, dict) for s in sessions):
    print("invalid")
elif any(s.get("NowPlayingItem") is not None for s in sessions):
    print("active")
else:
    print("idle")
' "${1}" 2>/dev/null || printf 'error\n'
}

fct_main() {
    local out='' managed_file='' state_list='' port="${FROZEN_PORT_DEFAULT}" jelly='http://127.0.0.1:8096/Sessions'
    local key_file='' no_playback_check=0 live_list='' host='' package='' version='' template=''
    local caddyfile='/etc/caddy/Caddyfile' site_dir='/etc/caddy/conf.d'
    while (( $# )); do
        case "${1}" in
            --out) out="${2:?--out needs a directory}"; shift 2 ;;
            --managed-file) managed_file="${2:?--managed-file needs a path}"; shift 2 ;;
            --state-list) state_list="${2:?--state-list needs a path}"; shift 2 ;;
            --frozen-port) port="${2:?--frozen-port needs a number}"; shift 2 ;;
            --jellyfin-url) jelly="${2:?--jellyfin-url needs a URL}"; shift 2 ;;
            --jellyfin-key-file) key_file="${2:?--jellyfin-key-file needs a path}"; shift 2 ;;
            --no-playback-check) no_playback_check=1; shift ;;
            --live-list) live_list="${2:?--live-list needs a path}"; shift 2 ;;
            --host) host="${2:?--host needs a value}"; shift 2 ;;
            --package) package="${2:?--package needs a path}"; shift 2 ;;
            --expected-version) version="${2:?--expected-version needs a value}"; shift 2 ;;
            --site-template|--template) template="${2:?--site-template needs a path}"; shift 2 ;;
            --caddyfile) caddyfile="${2:?--caddyfile needs a path}"; shift 2 ;;
            --site-dir) site_dir="${2:?--site-dir needs a directory}"; shift 2 ;;
            -h|--help) fct_usage; return 0 ;;
            *) fct_usage >&2; fct_die "unknown argument: ${1}" ;;
        esac
    done
    [[ "${port}" =~ ^[0-9]{1,5}$ ]] || fct_die 'invalid --frozen-port'
    if [[ -n "${host}" ]]; then
        [[ "${host}" =~ ^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?$ ]] || fct_die 'invalid --host (letters, digits, dots, dashes; IPv4 allowed)'
    fi
    if [[ -n "${package}${version}" && ( -z "${package}" || -z "${version}" ) ]]; then fct_die '--package and --expected-version go together'; fi
    [[ "${site_dir}" =~ ^/[A-Za-z0-9._/-]+$ ]] || fct_die 'invalid --site-dir'
    [[ -n "${template}" ]] || template="$(fct_default_template)"
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
    if [[ -n "${live_list}" ]]; then
        [[ -f "${live_list}" && -r "${live_list}" ]] || fct_die "live list not readable: ${live_list}"
        fct_load_live "${live_list}"
    fi

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
    if [[ -n "${live_list}" ]]; then cp -- "${live_list}" "${out}/live-paths.txt"; fi

    local -a managed=() refusals=() warnings=() notes=()
    if [[ -n "${managed_file}" ]]; then
        local line
        while IFS= read -r line || [[ -n "${line}" ]]; do [[ -n "${line}" && "${line}" != \#* ]] && managed+=("${line}"); done <"${managed_file}"
    else
        managed=("${DEFAULT_MANAGED[@]}")
    fi
    printf '%s\n' "${managed[@]}" >"${out}/managed.txt"

    # Every query below has three answers; "unknown" (it did not answer) is a refusal, never a guess.
    local class unit active enabled listing docker_known=1
    fct_unit_report >"${out}/units.txt"
    while read -r class unit active enabled; do
        if [[ "${unit}" == 'limeops-*' ]]; then
            refusals+=('systemd did not list its unit files (systemctl list-unit-files, named above), so the limeops-* units that must stay untouched are unknown')
            continue
        fi
        [[ "${active}" != unknown ]] || refusals+=("could not determine whether ${unit} is active (systemctl is-active, named above); the snapshot must record its state")
        [[ "${enabled}" != unknown ]] || refusals+=("could not determine whether ${unit} is enabled (systemctl is-enabled, named above); the snapshot must record its state")
    done <"${out}/units.txt"
    : "${class}"
    if listing="$(fct_query ss -H -ltn)"; then
        fct_lines "${listing}" >"${out}/ports.txt"
    else
        : >"${out}/ports.txt"
        refusals+=('could not list the listening TCP ports (ss -H -ltn failed or timed out)')
    fi
    if listing="$(fct_docker_ids)"; then
        fct_lines "${listing}" >"${out}/docker-ps.txt"
    else
        : >"${out}/docker-ps.txt"
        docker_known=0
    fi
    if listing="$(fct_docker_started)"; then
        fct_lines "${listing}" >"${out}/docker-started.txt"
    else
        : >"${out}/docker-started.txt"
        docker_known=0
    fi
    (( docker_known )) || refusals+=('could not list the running containers (docker ps or docker inspect failed or timed out, named above)')
    local requirement hashed=1
    while IFS= read -r requirement; do
        refusals+=("requirement: ${requirement}")
    done < <(fct_missing_requirements "$(( ! no_playback_check ))" 1)
    local problem
    while IFS= read -r problem; do refusals+=("${problem}"); done < <(fct_live_problems)
    fct_state_hashes strict >"${out}/state.sha256" || { hashed=0; refusals+=('state files could not all be hashed (named above); fix the state list or the files'); }

    # Who owns /var/lib/limeos as far as systemd is concerned, and what the shared directories
    # look like now: activate.sh takes the state directory over and return.sh gives it back.
    # Every unit on the host is asked, from its unit file or as loaded: one the cutover does not stop
    # would chown LimeOS's state away at its next start, and activate.sh guards only the frozen units.
    local claimants='' dir guard claimant
    if claimants="$(fct_all_state_claimants)"; then
        # The frozen ones in their own order (activate.sh guards them), the others refused.
        for claimant in "${FROZEN_SERVICES[@]}"; do
            if grep -qxF -- "${claimant}" <<<"${claimants}"; then printf '%s\n' "${claimant}"; fi
        done >"${out}/state-claimants.txt"
        while IFS= read -r claimant; do
            [[ -n "${claimant}" ]] || continue
            if [[ " ${FROZEN_SERVICES[*]} " != *" ${claimant} "* ]]; then
                refusals+=("${claimant} declares StateDirectory=limeos but is not one of the frozen units the cutover stops and guards (${FROZEN_SERVICES[*]}): its next start would chown /var/lib/limeos and LimeOS's state in it to its own account; stop and disable it, or remove that line from its unit (systemctl cat ${claimant}), before the cutover")
            fi
        done <<<"${claimants}"
    else
        : >"${out}/state-claimants.txt"
        refusals+=('could not determine which units declare StateDirectory=limeos (systemctl list-unit-files, list-units or show, named above); activate.sh must know which units own /var/lib/limeos')
    fi
    for dir in "${SHARED_DIRS[@]}"; do fct_dir_record "${dir}"; done >"${out}/shared-dirs.txt"
    while IFS= read -r guard; do
        [[ -n "${guard}" ]] || continue
        refusals+=("a cutover guard from an earlier activation is still in place: ${guard#"${CUTOVER_ROOT}"} (run return.sh on that snapshot, which removes it, or remove it and run systemctl daemon-reload)")
    done < <(fct_guard_files)
    # What the live list relaxes, named, never silent: every path of the snapshot recorded as live
    # (found under the shared directories or listed), in live-covered.txt.
    local record live_count=0 live_names=''
    : >"${out}/live-covered.txt"
    while IFS= read -r record || [[ -n "${record}" ]]; do
        fct_parse_record "${record}" || continue
        [[ "${REC_FLAG}" == L ]] || continue
        printf '%s\n' "${REC_PATH}" >>"${out}/live-covered.txt"
        live_count=$(( live_count + 1 ))
        (( live_count > 5 )) || live_names+="${live_names:+, }${REC_PATH}"
    done <"${out}/state.sha256"
    if (( live_count > 5 )); then live_names+=', ...'; fi
    if (( live_count )); then
        notes+=("the live list relaxes ${live_count} file(s) found under the shared directories or listed (only reported by return.sh, never failed on): ${live_names}; all in ${out#"${CUTOVER_ROOT}"}/live-covered.txt: read it, none may be the frozen app's own")
    fi

    local mem_line swap_line available swap_total swap_used
    # A free that fails leaves no Mem: line, which is refused below by name.
    free -b >"${out}/memory.txt" || true
    mem_line="$(grep -m1 '^Mem:' "${out}/memory.txt" || true)"
    swap_line="$(grep -m1 '^Swap:' "${out}/memory.txt" || true)"
    read -r _ _ _ _ _ _ available <<<"${mem_line}"
    read -r _ swap_total swap_used _ <<<"${swap_line}"
    getconf PAGESIZE >"${out}/pagesize.txt" || refusals+=('could not read the page size (getconf PAGESIZE failed)')

    {
        local proxy_found=no state
        for unit in caddy nginx traefik; do
            state="$(fct_unit_active "${unit}.service")"
            printf '%s %s\n' "${unit}" "${state}"
            case "${state}" in
                active) proxy_found=yes ;;
                unknown)
                    [[ "${proxy_found}" == yes ]] || proxy_found=unknown
                    refusals+=("could not determine whether ${unit}.service is active (systemctl is-active, named above); the snapshot must record it")
                    ;;
            esac
        done
        printf 'caddy-binary %s\n' "$(command -v caddy >/dev/null 2>&1 && echo present || echo absent)"
        printf 'https-proxy-active %s\n' "${proxy_found}"
    } >"${out}/proxy.txt"
    {
        local installed
        installed="$(fct_pkg_version limeos)"
        printf 'limeos %s\n' "${installed}"
        [[ "${installed}" != unknown ]] || refusals+=('could not determine whether the limeos package is installed (dpkg-query failed or timed out, named above)')
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
    while IFS= read -r record || [[ -n "${record}" ]]; do
        fct_parse_record "${record}" || continue
        # A live path may come and go (a SQLite journal of a service that keeps running).
        if [[ "${REC_FLAG}" == L ]]; then continue; fi
        # So may a SQLite journal of the frozen app found under the shared directories while it ran.
        if [[ "${REC_FLAG}" == V && "${REC_PATH}" == *@(-journal|-wal|-shm) ]]; then continue; fi
        if [[ "${REC_HASH}" == ABSENT ]]; then
            refusals+=("state path missing: ${REC_PATH} (fix the state list from limeosctl migrate scan)")
        else
            state_found=1
        fi
    done <"${out}/state.sha256"
    (( state_found || ! hashed )) || refusals+=("no frozen-app state found: no listed path exists and $(fct_state_dirs | paste -sd ' ' -) hold no files")
    problem="$(fct_catalog_problem)"
    [[ -z "${problem}" ]] || refusals+=("stack management catalog: ${problem}")
    # Transient scopes go to system.slice; return.sh --force ends one through its cgroup.kill.
    problem="$(fct_cgroup_kill_problem /system.slice)"
    [[ -z "${problem}" ]] || warnings+=("the cgroup2 unified hierarchy with cgroup.kill is not available (${problem}): return.sh --force could not end a hung activation by its scope and would stop without killing anything, unless run with --allow-systemctl-kill-fallback (systemd then signals the scope's processes by pid, so a pid reused meanwhile could be hit)")
    local running name
    # From the listing recorded above; when docker did not answer that is refused already, and
    # no container is reported as stopped on its account.
    if (( docker_known )); then
        running="$(awk '{ print $2 }' "${out}/docker-ps.txt")"
        for name in "${managed[@]}"; do
            grep -qx -- "${name}" <<<"${running}" || refusals+=("managed container not running: ${name}")
        done
    fi

    local code key body="${out}/.jellyfin-body"
    if (( no_playback_check )); then
        warnings+=('Jellyfin playback was not checked (--no-playback-check); check the dashboard by hand')
    elif [[ -z "${key_file}" ]]; then
        refusals+=('no Jellyfin API key: pass --jellyfin-key-file FILE (root-only) or --no-playback-check')
    elif ! command -v python3 >/dev/null 2>&1; then
        : # refused above as a missing requirement; the answer could not be read anyway
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
            local curl_status=0
            code="$(printf 'Authorization: MediaBrowser Token="%s"\n' "${key}" \
                | curl -sS --max-time 5 -H @- -o "${body}" -w '%{http_code}' "${jelly}" 2>/dev/null)" || curl_status=$?
            if [[ "${code}" == 401 || "${code}" == 403 ]]; then
                refusals+=("Jellyfin rejected the API key (HTTP ${code})")
            elif [[ "${code}" != 200 ]] || (( curl_status != 0 )); then
                refusals+=("Jellyfin playback could not be checked (HTTP ${code:-none}, curl exit ${curl_status})")
            else
                # Idle only when the whole body is a JSON list of sessions and none is playing.
                case "$(fct_sessions_verdict "${body}")" in
                    idle) ;;
                    active) refusals+=('Jellyfin has active playback') ;;
                    error) refusals+=('could not read the Jellyfin answer: python3 failed') ;;
                    *) refusals+=('Jellyfin /Sessions did not return a JSON list of sessions; check the URL and key') ;;
                esac
            fi
        fi
        key=''
    fi
    rm -f -- "${body}"

    # Operator drop-ins on the LimeOS units, reported (never changed): one setting User= on the
    # stack reader makes core refuse it, which activation then fails on.
    local dropin
    while IFS= read -r dropin; do
        if [[ "${dropin}" == *' unknown' ]]; then
            warnings+=("could not list the drop-ins of ${dropin% unknown} (systemctl show -p DropInPaths, named above)")
        else
            warnings+=("LimeOS unit drop-in, which changes what the packaged unit runs as (systemctl cat to read it): ${dropin}")
        fi
    done < <(fct_dropin_report)

    # What activate.sh needs, checked now rather than in the window.
    while IFS= read -r problem; do refusals+=("${problem}"); done < <(fct_layout_problems)
    problem="$(fct_template_problem "${template}")"
    [[ -z "${problem}" ]] || refusals+=("${problem}")
    while IFS= read -r problem; do refusals+=("${problem}"); done < <(fct_caddy_problems "${caddyfile}" "${site_dir}")
    if [[ -n "${host}" ]]; then
        fct_core_origin_ok "https://${host}:${port}" || refusals+=("core.json origin must be https://${host}:${port} (activate.sh refuses otherwise)")
    fi
    if [[ -n "${package}" ]]; then
        problem="$(fct_package_problem "${package}" "${version}")"
        [[ -z "${problem}" ]] || refusals+=("${problem}")
    fi
    if [[ -z "${host}" || -z "${package}" ]]; then
        warnings+=("activation inputs not checked: pass --host, --package and --expected-version as activate.sh will get them, to check core.json's origin and the .deb now")
    fi
    if grep -q '^caddy active$' "${out}/proxy.txt"; then
        warnings+=("caddy.service is running before the cutover (Debian's caddy package starts it on install): unless it serves other sites, stop and disable it now (systemctl disable --now caddy.service); return.sh restores the state this snapshot records")
    fi
    if [[ -s "${out}/state-claimants.txt" ]]; then
        local claimed_by running=''
        claimed_by="$(paste -sd ' ' "${out}/state-claimants.txt")"
        for unit in "${RUST_UNITS[@]}"; do
            if grep -q "^managed ${unit} active " "${out}/units.txt"; then running+=" ${unit}"; fi
        done
        if [[ -n "${running}" ]]; then
            warnings+=("LimeOS services are running while ${claimed_by} own /var/lib/limeos (StateDirectory=limeos): any start of those (a reboot, a timer) makes systemd chown /var/lib/limeos to their User=/Group= and takes LimeOS's state away from it; stop and disable them before the window (systemctl disable --now${running}); activate.sh starts them and return.sh leaves them stopped")
        fi
    fi

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
        for msg in "${notes[@]}"; do printf 'NOTE: %s\n' "${msg}"; done
    } >"${out}/findings.txt"

    fct_log "snapshot: ${out#"${CUTOVER_ROOT}"}"
    for msg in "${notes[@]}"; do fct_log "note: ${msg}"; done
    for msg in "${warnings[@]}"; do fct_warn "${msg}"; done
    if [[ "${verdict}" == refused ]]; then
        for msg in "${refusals[@]}"; do printf 'REFUSED: %s\n' "${msg}" >&2; done
        fct_die 'preflight refused; nothing was changed'
    fi
    fct_log 'preflight passed'
}

fct_main "$@"
