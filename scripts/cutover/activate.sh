#!/usr/bin/env bash
# T5: stop the frozen writers, install the pinned Rust package, start the Rust
# services and the read-only stack reader behind an HTTPS Caddy site, health-check,
# and roll back (return.sh) automatically on any failed step. Never touches limeops-*, limeos-agent,
# flask-shutdown, docker or containers. Needs explicit operator approval.
set -Eeuo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

MAX_SNAPSHOT_AGE=1800
SITE_NAME=limeos-cutover.caddy
ARMED=0
ROLLING_BACK=0
ATTACHED=0
DETACHED_PID=''
TAIL_PID=''
HEALTH_DETAIL=''
SNAP=''
SITE_DIR='/etc/caddy/conf.d'

fct_usage() {
    cat <<'USAGE'
Usage: activate.sh SNAPSHOT_DIR --yes-i-have-operator-approval
                   --package PATH --expected-version VERSION --host HOST
                   [--port 8002] [--tls internal | --tls-cert FILE --tls-key FILE]
                   [--site-dir /etc/caddy/conf.d] [--caddyfile /etc/caddy/Caddyfile]
                   [--template FILE] [--attached] [--dry-run] [--help]

PATH is the one limeos .deb to install; its Package and Version fields must be
limeos and VERSION, checked before anything changes. It is installed with
DEBIAN_FRONTEND=noninteractive and --force-confold, so an existing
/etc/limeos/core.json is kept. HOST is the name or IPv4
address browsers use (core.json "origin" must be https://HOST:PORT). The default
TLS mode is internal (Caddy local CA). --dry-run prints every command and changes
nothing; a signal during it only stops it and never runs return.sh. Any failed
step after the first change runs return.sh and exits 1, and so do SIGINT, SIGTERM, SIGHUP and an unexpected exit once a change has been made. Each
step runs in its own session, so a terminal Ctrl-C lets it finish (dpkg is never
killed mid-install) before the rollback starts. The rollback runs once and to the
end: signals are ignored while it runs.

Detached by default: after the snapshot check the script runs itself again in a
new session (systemd-run --scope, then setsid; systemd-run is required, and if it
fails nothing has changed), with output appended to SNAPSHOT_DIR/activate.log.
This process only follows that log and exits with the run's status. A dropped SSH
session therefore cannot stop the run mid-step; read the log afterwards.
Signals: Ctrl-C (SIGINT) here is passed to the run as SIGTERM, which rolls back.
SIGTERM or SIGHUP here only stops following; the run carries on. The run itself
rolls back on SIGINT, SIGTERM or SIGHUP: with the terminal gone, use
kill -TERM <pid printed at the start>. --attached runs in the foreground instead
(tests, or an operator inside tmux/screen).

One run per snapshot: the run holds SNAPSHOT_DIR/activate.lock, and a second
activate.sh on that snapshot is refused while it does. A snapshot that has
completed an activation (SNAPSHOT_DIR/activated), or an existing cutover Caddy
site, is refused before any change.
USAGE
}

fct_abort() {
    printf 'FAIL: %s\n' "$*" >&2
    # A dry run changes nothing, so it never arms and never runs return.sh.
    if (( ARMED && ! DRY_RUN && ! ROLLING_BACK )); then
        # Once, and to the end: a second signal or the EXIT trap must neither repeat nor cut it short.
        ROLLING_BACK=1
        trap '' INT TERM HUP
        printf 'ROLLBACK: running return.sh\n' >&2
        "${SCRIPT_DIR}/return.sh" "${SNAP}" --site-dir "${SITE_DIR}" >&2 || printf 'ROLLBACK INCOMPLETE: follow docs/p06-cutover-runbook.md\n' >&2
        ARMED=0
    fi
    exit 1
}

# Any exit while armed (set -e, fct_die, a stray failure) rolls back too.
fct_on_exit() {
    local status=$?
    if (( ARMED )); then fct_abort "exited with status ${status} before activation completed"; fi
}

# The follower passes the operator's Ctrl-C to the run as TERM (a rollback). A TERM or HUP
# to the follower (logind, sudo, a closed terminal) only ends the follower.
fct_forward() { if [[ -n "${DETACHED_PID}" ]]; then kill -s TERM "${DETACHED_PID}" 2>/dev/null || true; fi; }
fct_stop_following() { # exit status
    if [[ -n "${TAIL_PID}" ]]; then kill "${TAIL_PID}" 2>/dev/null || true; fi
    printf 'activate: stopped following; the detached run (pid %s) carries on, see %s\n' "${DETACHED_PID}" "${SNAP}/activate.log" || true
    exit "${1}"
}

fct_detach() { # original arguments
    local log="${SNAP}/activate.log" offset rc
    command -v systemd-run >/dev/null 2>&1 \
        || fct_die 'systemd-run is required to run detached; nothing was changed (or run --attached inside tmux or screen)'
    # A background job of a script starts with SIGINT ignored, and bash cannot trap a signal
    # ignored at entry: env --default-signal=INT gives the run a SIGINT it can trap.
    local -a runner=(systemd-run --scope --quiet --collect -- env --default-signal=INT setsid --wait)
    printf '=== activate.sh started %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"${log}"
    offset="$(stat -c %s -- "${log}")"
    trap fct_forward INT
    trap 'fct_stop_following 143' TERM
    trap 'fct_stop_following 129' HUP
    "${runner[@]}" bash "${SCRIPT_DIR}/activate.sh" "$@" --attached </dev/null >>"${log}" 2>&1 &
    DETACHED_PID=$!
    fct_log "activate: running detached (pid ${DETACHED_PID}); a dropped SSH session does not stop it"
    fct_log "activate: log ${log}; Ctrl-C here rolls back"
    tail -c "+$(( offset + 1 ))" -s 0.2 -f --pid="${DETACHED_PID}" -- "${log}" &
    TAIL_PID=$!
    while kill -0 "${DETACHED_PID}" 2>/dev/null; do wait "${DETACHED_PID}" || true; done
    rc=0
    wait "${DETACHED_PID}" || rc=$?
    wait "${TAIL_PID}" || true
    return "${rc}"
}

# One activation per snapshot at a time: an exclusive lock on SNAPSHOT/activate.lock held
# on fd 9. The detached run inherits the follower's fd 9 (the same open file, so the same
# lock) and keeps the lock after the follower has gone.
fct_lock() {
    local lock="${SNAP}/activate.lock"
    if [[ ! /proc/self/fd/9 -ef "${lock}" ]]; then
        exec 9>>"${lock}" || fct_die "cannot open ${lock}"
    fi
    flock -n 9 || fct_die "another activate.sh is running on this snapshot (${lock} is locked); nothing was changed"
}

# Refuse before arming when the cutover is already in place: a rerun would stop nothing
# useful, fail on the port Caddy holds and roll back a working cutover.
fct_refuse_if_active() {
    local site_file="${CUTOVER_ROOT}${SITE_DIR}/${SITE_NAME}"
    if [[ -e "${SNAP}/activated" ]]; then
        fct_need "this snapshot was already activated ($(head -n1 -- "${SNAP}/activated")); nothing was changed (return.sh goes back; a new activation needs a new preflight)" false
    fi
    fct_need "the cutover is already active (${SITE_DIR}/${SITE_NAME} exists); nothing was changed" test ! -e "${site_file}"
}

# Mutating step: from the first one on, any failure rolls back (never under --dry-run).
# Each step runs in its own session, so a terminal Ctrl-C (sent to the whole foreground
# process group) reaches only this script: the step (dpkg mid-install, say) finishes and
# the trap rolls back after it returns.
fct_do() {
    if (( DRY_RUN )); then fct_run "$@"; return 0; fi
    ARMED=1
    printf '+ %s\n' "$*"
    setsid --wait "$@" 9>&- || fct_abort "command failed: $*"
}

# Read-only precondition. Host-presence problems only warn under --dry-run.
fct_need() { # message, then test command
    local message="${1}"
    shift
    if "$@"; then return 0; fi
    if (( DRY_RUN )); then fct_warn "dry-run: ${message}"; return 0; fi
    fct_die "${message}"
}

fct_retry() { # label, then command; up to 30 tries one second apart
    local label="${1}" attempt
    shift
    if (( DRY_RUN )); then printf '[dry-run] check %s: %s\n' "${label}" "$*"; return 0; fi
    HEALTH_DETAIL=''
    for (( attempt=0; attempt<30; attempt++ )); do
        if "$@" >/dev/null 2>&1; then printf 'ok: %s\n' "${label}"; return 0; fi
        sleep 1
    done
    fct_abort "health check failed: ${label}${HEALTH_DETAIL:+ (${HEALTH_DETAIL})}"
}

fct_render() { # template site tls -> stdout
    local text
    text="$(cat -- "${1}")"
    text="${text//@SITE@/${2}}"
    printf '%s\n' "${text//@TLS@/${3}}"
}

fct_port_free() { ! ss -H -ltn | grep -Eq "[:.]${1}[[:space:]]"; }
fct_core_origin_ok() { tr -d ' \n' <"${CUTOVER_ROOT}/etc/limeos/core.json" | grep -qF "\"origin\":\"${1}\""; }
fct_installed_version_ok() { [[ "$(dpkg-query -W -f='${Version}' limeos 2>/dev/null || true)" == "${1}" ]]; }
# The stack reader (started, never enabled) is up, listening, and not crash-looping.
fct_reader_ready() {
    systemctl is-active --quiet "${STACK_READER}" || return 1
    [[ -S "${CUTOVER_ROOT}${STACK_READER_SOCKET}" ]] || return 1
    [[ "$(systemctl show -p NRestarts --value "${STACK_READER}" 2>/dev/null || echo 0)" == 0 ]]
}
fct_units_active() { local unit; for unit in "$@"; do systemctl is-active --quiet "${unit}" || return 1; done; }
# Managed containers have the snapshot's IDs and StartedAt; HEALTH_DETAIL names any that do not.
fct_docker_matches() {
    local changed
    changed="$(fct_docker_changes "${SNAP}" | fct_managed_only "${SNAP}" | paste -sd ' ' -)"
    HEALTH_DETAIL="${changed:+changed: ${changed}}"
    [[ -z "${changed}" ]]
}
fct_untouched_match() {
    [[ "$(fct_unit_report | grep '^untouched ')" == "$(grep '^untouched ' "${SNAP}/units.txt")" ]]
}

fct_main() {
    local approved=0 package='' version='' host='' port="${FROZEN_PORT_DEFAULT}" tls=internal cert='' key=''
    local caddyfile='/etc/caddy/Caddyfile' template="${SCRIPT_DIR}/../../packaging/Caddyfile.cutover.example"
    local -a original_args=("$@")
    while (( $# )); do
        case "${1}" in
            --yes-i-have-operator-approval) approved=1; shift ;;
            --dry-run) DRY_RUN=1; shift ;;
            --attached) ATTACHED=1; shift ;;
            --package) package="${2:?--package needs a path}"; shift 2 ;;
            --expected-version) version="${2:?--expected-version needs a value}"; shift 2 ;;
            --host) host="${2:?--host needs a value}"; shift 2 ;;
            --port) port="${2:?--port needs a number}"; shift 2 ;;
            --tls) tls="${2:?--tls needs internal}"; shift 2 ;;
            --tls-cert) cert="${2:?--tls-cert needs a path}"; shift 2 ;;
            --tls-key) key="${2:?--tls-key needs a path}"; shift 2 ;;
            --site-dir) SITE_DIR="${2:?--site-dir needs a directory}"; shift 2 ;;
            --caddyfile) caddyfile="${2:?--caddyfile needs a path}"; shift 2 ;;
            --template) template="${2:?--template needs a path}"; shift 2 ;;
            -h|--help) fct_usage; return 0 ;;
            -*) fct_usage >&2; fct_die "unknown argument: ${1}" ;;
            *) [[ -z "${SNAP}" ]] || fct_die 'only one snapshot directory is accepted'; SNAP="${1}"; shift ;;
        esac
    done
    [[ -n "${SNAP}" ]] || { fct_usage >&2; fct_die 'a preflight snapshot directory is required'; }
    if (( ! approved && ! DRY_RUN )); then fct_die 'refusing without --yes-i-have-operator-approval'; fi
    [[ -n "${package}" && -n "${version}" && -n "${host}" ]] || fct_die '--package, --expected-version and --host are required'
    [[ "${host}" =~ ^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?$ ]] || fct_die 'invalid --host (letters, digits, dots, dashes; IPv4 allowed)'
    [[ "${port}" =~ ^[0-9]{2,5}$ ]] || fct_die 'invalid --port'
    [[ "${version}" =~ ^[A-Za-z0-9.+~:-]+$ ]] || fct_die 'invalid --expected-version'
    local tls_value
    if [[ -n "${cert}${key}" ]]; then
        [[ -n "${cert}" && -n "${key}" ]] || fct_die '--tls-cert and --tls-key go together'
        [[ "${cert}" =~ ^/[A-Za-z0-9._/-]+$ && "${key}" =~ ^/[A-Za-z0-9._/-]+$ ]] || fct_die 'certificate and key must be plain absolute paths'
        fct_need "certificate or key not readable" test -r "${CUTOVER_ROOT}${cert}" -a -r "${CUTOVER_ROOT}${key}"
        tls_value="${cert} ${key}"
    else
        [[ "${tls}" == internal ]] || fct_die '--tls accepts only "internal"; use --tls-cert/--tls-key for your own certificate'
        tls_value=internal
    fi
    local site="https://${host}:${port}"
    [[ "${SITE_DIR}" =~ ^/[A-Za-z0-9._/-]+$ ]] || fct_die 'invalid --site-dir'
    fct_require_root

    fct_snapshot_check "${SNAP}" "${MAX_SNAPSHOT_AGE}"
    [[ -f "${SNAP}/state-paths.txt" ]] || fct_die 'snapshot has no state-paths.txt (taken before state lists); run preflight again'
    [[ -f "${SNAP}/managed.txt" ]] || fct_die 'snapshot has no managed.txt (taken before managed sets were recorded); run preflight again'
    STATE_LIST="${SNAP}/state-paths.txt"
    fct_records_ok "${SNAP}/state.sha256" || fct_die 'the snapshot has a malformed state record; run preflight again'
    if (( ! DRY_RUN )); then fct_lock; fi
    fct_refuse_if_active
    if (( ! DRY_RUN && ! ATTACHED )); then
        local rc=0
        fct_detach "${original_args[@]}" || rc=$?
        return "${rc}"
    fi
    # Only the named file is installed, and only when it is the pinned limeos version.
    [[ "${package}" == *.deb ]] || fct_die "--package must name one limeos .deb file: ${package}"
    fct_need "package not found: ${package}" test -f "${CUTOVER_ROOT}${package}"
    if [[ -f "${CUTOVER_ROOT}${package}" ]]; then
        local deb_name deb_version
        deb_name="$(dpkg-deb -f "${CUTOVER_ROOT}${package}" Package)" || fct_die "cannot read the package fields of ${package}"
        deb_version="$(dpkg-deb -f "${CUTOVER_ROOT}${package}" Version)" || fct_die "cannot read the package fields of ${package}"
        [[ "${deb_name}" == limeos ]] || fct_die "${package} is package ${deb_name}, not limeos"
        [[ "${deb_version}" == "${version}" ]] || fct_die "${package} is version ${deb_version}, not ${version}"
    fi
    fct_need 'caddy is not installed (install it separately first)' command -v caddy
    fct_need "${caddyfile} must contain: import ${SITE_DIR}/*.caddy" grep -Eqx "[[:space:]]*import[[:space:]]+${SITE_DIR}/\*(\.caddy)?" "${CUTOVER_ROOT}${caddyfile}"
    fct_need "core.json origin must be ${site}" fct_core_origin_ok "${site}"
    local catalog_problem
    catalog_problem="$(fct_catalog_problem)"
    [[ -z "${catalog_problem}" ]] || fct_need "${catalog_problem}" false
    [[ -r "${template}" ]] || fct_die "site template not readable: ${template}"

    local rendered=''
    if (( DRY_RUN )); then
        printf '[dry-run] render %s -> %s%s/%s (site %s, tls %s)\n' "${template}" "${CUTOVER_ROOT}" "${SITE_DIR}" "${SITE_NAME}" "${site}" "${tls_value}"
        printf '[dry-run] caddy validate --adapter caddyfile --config <rendered site>\n'
    else
        rendered="$(mktemp)"
        fct_render "${template}" "${site}" "${tls_value}" >"${rendered}"
        caddy validate --adapter caddyfile --config "${rendered}" >/dev/null 2>&1 || { rm -f -- "${rendered}"; fct_die 'rendered Caddy site does not validate; nothing was changed'; }
    fi


    fct_log "activate: frozen writers stop, Rust services start, HTTPS at ${site}"
    local unit
    # 1. Stop and disable only the frozen writers (pihealth-helper is PartOf pi-health).
    fct_do systemctl stop limeos-metrics-collector.timer limeos-package-reconcile.timer
    fct_do systemctl stop pi-health.service
    fct_do systemctl stop pihealth-helper.service
    for unit in "${FROZEN_UNITS[@]}"; do fct_do systemctl disable "${unit}"; done
    if (( ! DRY_RUN )); then
        # A hash failure here ends the script under set -e; the EXIT trap rolls back.
        fct_state_hashes >"${SNAP}/state.poststop.sha256"
        local record
        while IFS= read -r record || [[ -n "${record}" ]]; do
            fct_parse_record "${record}" || fct_abort "malformed state record: ${record}"
            [[ "${REC_FLAG}" == S ]] || continue
            grep -qxF -- "${record}" "${SNAP}/state.poststop.sha256" || fct_abort "state file changed before the cutover: ${REC_PATH}"
        done <"${SNAP}/state.sha256"
    else
        printf '[dry-run] hash state files again and compare strict entries with the snapshot\n'
    fi
    if (( ! DRY_RUN )) && ! fct_port_free "${port}"; then fct_abort "port ${port} is still in use after the frozen app stopped"; fi

    # 2. Install the pinned package; its postinst starts the services too. Never prompt, and
    # keep the operator's /etc/limeos/core.json (a conffile) when it differs from the package's.
    fct_do env DEBIAN_FRONTEND=noninteractive dpkg --force-confold -i "${package}"
    if (( DRY_RUN )); then printf '[dry-run] check installed limeos version is %s\n' "${version}"
    elif ! fct_installed_version_ok "${version}"; then fct_abort "installed limeos version is not ${version}"; fi
    fct_do systemctl daemon-reload
    for unit in "${RUST_UNITS[@]}"; do fct_do systemctl enable "${unit}"; done
    for unit in "${RUST_UNITS[@]}"; do fct_do systemctl start "${unit}"; done
    fct_retry 'Rust services active' fct_units_active "${RUST_UNITS[@]}"
    # Stack status and whole-stack actions stay unavailable until the reader runs. It has no
    # Install section on purpose: start it now, never enable it; return.sh stops it.
    fct_do systemctl start "${STACK_READER}"

    # 3. HTTPS proxy on the public port, now that the frozen app has released it.
    fct_do install -d -m 0755 -- "${CUTOVER_ROOT}${SITE_DIR}"
    if (( DRY_RUN )); then
        printf '[dry-run] install -m 0644 <rendered site> %s%s/%s\n' "${CUTOVER_ROOT}" "${SITE_DIR}" "${SITE_NAME}"
    else
        fct_do install -m 0644 -- "${rendered}" "${CUTOVER_ROOT}${SITE_DIR}/${SITE_NAME}"
        rm -f -- "${rendered}"
    fi
    fct_do caddy validate --adapter caddyfile --config "${caddyfile}"
    fct_do systemctl enable caddy.service
    if (( DRY_RUN )) || [[ "$(fct_unit_active caddy.service)" != active ]]; then
        fct_do systemctl start caddy.service
    else
        fct_do systemctl reload caddy.service
    fi

    # 4. Health checks.
    local curl_base=(curl -fsS -k --max-time 5 --resolve "${host}:${port}:127.0.0.1" -o /dev/null)
    fct_retry 'core health via the proxy' "${curl_base[@]}" "${site}/api/v1/health"
    fct_retry 'login page via the proxy' "${curl_base[@]}" "${site}/"
    fct_retry 'limeosctl status' "${LIMEOSCTL}" --socket "${CORE_SOCKET}" status
    fct_retry 'managed containers: docker ps and StartedAt identical to the snapshot' fct_docker_matches
    if (( ! DRY_RUN )); then fct_warn_unmanaged_changes "${SNAP}"; fi
    fct_retry 'untouched units unchanged' fct_untouched_match
    fct_retry 'stack reader ready' fct_reader_ready
    if (( ! DRY_RUN )); then sleep 3; fi
    fct_retry 'stack reader still ready after settling' fct_reader_ready
    ARMED=0
    if (( ! DRY_RUN )); then
        printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${site}" >"${SNAP}/activated" \
            || fct_warn "could not mark ${SNAP} as activated"
    fi
    fct_log "activate complete: ${site}"
}

trap fct_on_exit EXIT
trap 'fct_abort "interrupted by SIGINT"' INT
trap 'fct_abort "interrupted by SIGTERM"' TERM
trap 'fct_abort "interrupted by SIGHUP"' HUP
fct_main "$@"
