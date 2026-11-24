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
STEP_PID=''
STEP_START=''
STEP_LOCK_FD=''
PENDING_SIGNAL=''
# The transient scope this run is inside (set by the internal --scope-unit once it is).
SCOPE_UNIT=''
# How long a signal waits for the running step before stopping it. The longest legitimate step
# is dpkg -i: its postinst restarts three services, each allowed systemd's default 90 s start
# timeout, so five minutes is generous.
STEP_GRACE="${CUTOVER_STEP_GRACE:-300}"
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
killed mid-install) before the rollback starts; after 300 s it is stopped (TERM,
then KILL) and the rollback goes ahead anyway. The rollback runs once and to the
end: signals are ignored while it runs.

Every run happens inside a transient systemd scope named for the snapshot,
limeos-cutover-activate-<SHA-256 of its path>.scope (systemd-run --scope --unit;
systemd-run is required, and if it cannot create the scope nothing has changed).
Everything the run starts stays in that scope, which is how return.sh --force
ends exactly the activation and nothing else.

Detached by default: after the snapshot check the script runs itself again in
that scope and in a new session (setsid), with output appended to
SNAPSHOT_DIR/activate.log. This process only follows that log and exits with the
run's status. A dropped SSH session therefore cannot stop the run mid-step; read
the log afterwards. Signals: Ctrl-C (SIGINT) here is passed to the run as
SIGTERM, which rolls back. SIGTERM or SIGHUP here only stops following; the run
carries on. The run itself rolls back on SIGINT, SIGTERM or SIGHUP: with the
terminal gone, use kill -TERM <pid printed at the start>. --attached runs in the
foreground instead (tests, or an operator inside tmux/screen): the same process
enters the scope and keeps the terminal.

One run per snapshot: the run holds SNAPSHOT_DIR/activate.lock, and a second
activate.sh on that snapshot is refused while it does. A snapshot that has
completed an activation (SNAPSHOT_DIR/activated), one with an unfinished
return.sh --force (SNAPSHOT_DIR/force-takeover), one whose units.txt lacks a
definite state for a unit, or an existing cutover Caddy site, is refused before
any change.

Each health check is tried 30 times, a second apart. A query that times out
(30 s) or does not answer counts as a failure of that check, never a pass: the
check then fails with "could not determine" (and the last try's warnings),
otherwise with "was false", and the activation rolls back.
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

# Any exit while armed (set -e, fct_die, a stray failure) rolls back too. Run by lib.sh's EXIT
# trap, after it has named an unexpected failure.
fct_on_exit() { # status
    local status="${1}"
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

# Attached: this same process enters the scope. systemd-run --scope moves itself into the new
# scope and execs the command in place, so the pid, the terminal, the foreground process group
# and the lock on fd 9 all stay as they are. If systemd-run cannot create the scope it exits
# with its own error before anything has changed.
fct_enter_scope() { # unit, original arguments
    local unit="${1}"
    shift
    fct_log "activate: entering transient scope ${unit} (if systemd-run reports an error, nothing was changed)"
    exec systemd-run --scope --unit="${unit}" --quiet --collect -- bash "${SCRIPT_DIR}/activate.sh" "$@" --scope-unit "${unit}"
}

fct_detach() { # unit, original arguments
    local unit="${1}" log="${SNAP}/activate.log" offset rc
    shift
    command -v systemd-run >/dev/null 2>&1 \
        || fct_die 'systemd-run is required (every activation runs in a transient scope); nothing was changed'
    # A background job of a script starts with SIGINT ignored, and bash cannot trap a signal
    # ignored at entry: env --default-signal=INT gives the run a SIGINT it can trap.
    local -a runner=(systemd-run --scope --unit="${unit}" --quiet --collect -- env --default-signal=INT setsid --wait)
    printf '=== activate.sh started %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"${log}"
    offset="$(stat -c %s -- "${log}")"
    trap fct_forward INT
    trap 'fct_stop_following 143' TERM
    trap 'fct_stop_following 129' HUP
    "${runner[@]}" bash "${SCRIPT_DIR}/activate.sh" "$@" --scope-unit "${unit}" </dev/null >>"${log}" 2>&1 &
    DETACHED_PID=$!
    # The run has its own copy of fd 9; only the scope holds the lock from here on, so ending
    # the scope frees it.
    exec 9>&-
    fct_log "activate: running detached in scope ${unit} (pid ${DETACHED_PID}); a dropped SSH session does not stop it"
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
# on fd 9. The run in the scope inherits fd 9 (the same open file, so the same lock) and
# keeps the lock after the follower has gone.
fct_lock() { # unit
    local lock="${SNAP}/activate.lock"
    if [[ ! /proc/self/fd/9 -ef "${lock}" ]]; then
        exec 9>>"${lock}" || fct_die "cannot open ${lock}"
    fi
    flock -n 9 || fct_die "$(fct_lock_holder_text "${lock}"); nothing was changed"
    # Who holds it and in which scope, for refusals and return.sh (the run in the scope
    # re-records itself).
    fct_record_holder "${lock}" activate "${1}"
}

# Refuse before arming when the cutover is already in place: a rerun would stop nothing
# useful, fail on the port Caddy holds and roll back a working cutover.
fct_refuse_if_active() {
    local site_file="${CUTOVER_ROOT}${SITE_DIR}/${SITE_NAME}"
    if [[ -e "${SNAP}/activated" ]]; then
        fct_need "this snapshot was already activated ($(head -n1 -- "${SNAP}/activated")); nothing was changed (return.sh goes back; a new activation needs a new preflight)" false
    fi
    if [[ -e "${SNAP}/force-takeover" ]]; then
        fct_need "a forced return.sh on this snapshot has not finished (${SNAP}/force-takeover); nothing was changed (run return.sh ${SNAP} first; a new activation needs a new preflight)" false
    fi
    fct_need "the cutover is already active (${SITE_DIR}/${SITE_NAME} exists); nothing was changed" test ! -e "${site_file}"
}

# The running step is still the process started at STEP_START (read-only, so reuse-safe).
fct_step_running() { [[ -n "${STEP_START}" ]] && fct_proc_running "${STEP_PID}" "${STEP_START}"; }

# Mutating step: from the first one on, any failure rolls back (never under --dry-run).
# Each step runs in its own session, so a terminal Ctrl-C (sent to the whole foreground
# process group) reaches only this script: the step (dpkg mid-install, say) finishes and
# the trap rolls back after it returns. setsid changes the session, never the cgroup, so the
# step and everything it starts stay in the activation's scope (nothing here runs a nested
# systemd-run, which would move them out).
#
# fct_finish_step may have to signal the step's process group by number. Bash reaps its
# children by itself, so it cannot hold the step unreaped to keep that number from being
# reused; instead the step's first act (before it execs the command) is to fork a pin: a
# process in the step's session and process group that ignores TERM, INT and HUP and exits
# only once this script unlocks STEP_LOCK (or exits). The kernel never hands out a pid while
# a process group of that number exists, so while the pin lives, -STEP_PID names this step's
# group and nothing else. The lock is held for the whole step and released below.
fct_do() {
    if (( DRY_RUN )); then fct_run "$@"; return 0; fi
    ARMED=1
    printf '+ %s\n' "$*"
    local rc=0
    flock -x "${STEP_LOCK_FD}"
    # In the background so a signal interrupts wait at once (see fct_on_signal).
    # shellcheck disable=SC2016  # expanded by the step's own bash
    setsid --wait bash -c '(trap "" INT TERM HUP; exec flock -s "$0" true) </dev/null >/dev/null 2>&1 & exec "$@"' \
        "${SNAP}/step.lock" "$@" 9>&- {STEP_LOCK_FD}>&- &
    STEP_PID=$!
    STEP_START="$(fct_proc_start "${STEP_PID}")"
    while :; do
        rc=0
        wait "${STEP_PID}" || rc=$?
        if ! fct_step_running; then break; fi
        # A signal interrupted wait while the step still runs: give it the grace period.
        if [[ -n "${PENDING_SIGNAL}" ]]; then
            # Reap the step only once it has gone; one stuck in uninterruptible I/O must not hold
            # the rollback (fct_finish_step recorded it for return.sh).
            if fct_finish_step "$*"; then rc=0; wait "${STEP_PID}" || rc=$?; fi
            break
        fi
    done
    flock -u "${STEP_LOCK_FD}"
    STEP_PID=''
    STEP_START=''
    if [[ -n "${PENDING_SIGNAL}" ]]; then fct_abort "interrupted by SIG${PENDING_SIGNAL}"; fi
    (( rc == 0 )) || fct_abort "command failed: $*"
}

# Wait up to STEP_GRACE seconds for the running step; past that, stop its whole session
# (TERM, then KILL after 10 s) and let the rollback go ahead: a host left half-cut-over with
# the frozen app stopped is worse than an interrupted step. If that step was dpkg, run
# dpkg --audit after the rollback and finish or remove the package by hand.
fct_finish_step() { # command text
    local deadline=$(( SECONDS + STEP_GRACE ))
    while fct_step_running && (( SECONDS < deadline )); do fct_pause; done
    fct_step_running || return 0
    printf 'TIMEOUT: step still running after %ss: %s; stopping it and rolling back anyway (if it was dpkg, run dpkg --audit afterwards)\n' "${STEP_GRACE}" "${1}" >&2
    # The step's pin is alive (this script still holds STEP_LOCK and the pin ignores TERM), so
    # -STEP_PID is still this step's process group: see fct_do.
    kill -s TERM -- "-${STEP_PID}" 2>/dev/null || true
    deadline=$(( SECONDS + 10 ))
    while fct_step_running && (( SECONDS < deadline )); do fct_pause; done
    fct_step_running || return 0
    # Still pinned: TERM cannot end the pin. This signal ends it along with the step, so from
    # here on the step is only watched (by pid and start time), never signalled again.
    kill -s "${KILL_SIGNAL}" -- "-${STEP_PID}" 2>/dev/null || true
    deadline=$(( SECONDS + 5 ))
    while fct_step_running && (( SECONDS < deadline )); do fct_pause; done
    fct_step_running || return 0
    # Still there (a dpkg in uninterruptible I/O): roll back regardless, and leave a record so
    # return.sh can tell whether it is still alive and say what to do.
    printf 'TIMEOUT: step (pid %s) has not exited after SIG%s: %s; rolling back regardless\n' "${STEP_PID}" "${KILL_SIGNAL}" "${1}" >&2
    printf '%s %s %s\n' "${STEP_PID}" "${STEP_START}" "${1}" >"${SNAP}/stuck-step" \
        || fct_warn "could not record the stuck step in ${SNAP}/stuck-step"
    return 1
}

# A signal during a step lets the step finish (bounded by STEP_GRACE) before the rollback;
# anywhere else it rolls back at once.
fct_on_signal() { # INT, TERM or HUP
    if [[ -z "${STEP_PID}" ]]; then fct_abort "interrupted by SIG${1}"; fi
    if [[ -z "${PENDING_SIGNAL}" ]]; then
        PENDING_SIGNAL="${1}"
        printf 'SIG%s: letting the running step finish (up to %ss), then rolling back\n' "${1}" "${STEP_GRACE}" >&2
    fi
}

# Read-only precondition. Host-presence problems only warn under --dry-run.
fct_need() { # message, then test command
    local message="${1}"
    shift
    if "$@"; then return 0; fi
    if (( DRY_RUN )); then fct_warn "dry-run: ${message}"; return 0; fi
    fct_die "${message}"
}

# Up to 30 tries one second apart of a check that succeeds (yes), fails with 2 (it could not
# determine: a query timed out or did not answer) or fails otherwise (no). The verdict is the last
# try's: "could not determine" is never reported as "was false", and its warnings (a timed-out
# query names itself) are printed with it rather than swallowed by the retries.
fct_retry() { # label, then command
    local label="${1}" attempt rc=0 err
    shift
    if (( DRY_RUN )); then printf '[dry-run] check %s: %s\n' "${label}" "$*"; return 0; fi
    err="${SNAP}/health-check.err"
    for (( attempt=0; attempt<30; attempt++ )); do
        HEALTH_DETAIL=''
        rc=0
        "$@" >/dev/null 2>"${err}" || rc=$?
        if (( rc == 0 )); then rm -f -- "${err}"; printf 'ok: %s\n' "${label}"; return 0; fi
        sleep 1
    done
    if [[ -s "${err}" ]]; then cat -- "${err}" >&2 || true; fi
    rm -f -- "${err}"
    if (( rc == 2 )); then
        fct_abort "health check failed: ${label}: could not determine after 30 tries${HEALTH_DETAIL:+ (${HEALTH_DETAIL})}"
    fi
    fct_abort "health check failed: ${label}: was false after 30 tries${HEALTH_DETAIL:+ (${HEALTH_DETAIL})}"
}

fct_render() { # template site tls -> stdout
    local text
    text="$(cat -- "${1}")"
    text="${text//@SITE@/${2}}"
    printf '%s\n' "${text//@TLS@/${3}}"
}

# "free", "used" or "unknown" (ss failed or timed out) for a TCP port.
fct_port_state() { # port
    local listing
    if ! listing="$(fct_query ss -H -ltn)"; then printf 'unknown\n'; return 0; fi
    if grep -Eq "[:.]${1}[[:space:]]" <<<"${listing}"; then printf 'used\n'; else printf 'free\n'; fi
}
fct_core_origin_ok() { tr -d ' \n' <"${CUTOVER_ROOT}/etc/limeos/core.json" | grep -qF "\"origin\":\"${1}\""; }

# Health checks: each succeeds (yes), fails with 1 (no) or with 2 (could not determine), and sets
# HEALTH_DETAIL to say why it failed.
fct_units_active() { # unit...
    local unit state unknown=''
    for unit in "$@"; do
        state="$(fct_unit_active "${unit}")"
        case "${state}" in
            active) ;;
            unknown) unknown+=" ${unit}" ;;
            *) HEALTH_DETAIL="${unit} is ${state}"; return 1 ;;
        esac
    done
    if [[ -n "${unknown}" ]]; then HEALTH_DETAIL="systemd did not report the state of${unknown}"; return 2; fi
}
# The stack reader (started, never enabled) is up, listening as its own account, has never
# restarted, and core takes an answer from it. A restart count that cannot be read is not zero:
# the reader's crash history is then unverified. A socket owned by another account (a drop-in
# setting User=root, say) is a reader core refuses: stack status and storage inventory would stay
# unavailable while everything else looks healthy, so it fails activation. `limeosctl
# reader-status` has core ask the reader now under its socket rule and fails unless core accepted
# the answer.
fct_reader_ready() {
    local restarts owner rc=0 answer
    fct_units_active "${STACK_READER}" || return
    if [[ ! -S "${CUTOVER_ROOT}${STACK_READER_SOCKET}" ]]; then HEALTH_DETAIL="${STACK_READER_SOCKET} is not listening"; return 1; fi
    if ! owner="$(stat -c %U -- "${CUTOVER_ROOT}${STACK_READER_SOCKET}" 2>/dev/null)" || [[ -z "${owner}" ]]; then
        HEALTH_DETAIL="could not read the owner of ${STACK_READER_SOCKET}"
        return 2
    fi
    if [[ "${owner}" != "${STACK_READER_ACCOUNT}" ]]; then
        HEALTH_DETAIL="${STACK_READER_SOCKET} is owned by ${owner}, not ${STACK_READER_ACCOUNT}: core refuses it (look for a drop-in: systemctl cat ${STACK_READER})"
        return 1
    fi
    if ! restarts="$(fct_unit_property NRestarts "${STACK_READER}")" || [[ ! "${restarts}" =~ ^[0-9]+$ ]]; then
        HEALTH_DETAIL="systemd did not report the restart count (NRestarts) of ${STACK_READER}"
        return 2
    fi
    if (( restarts != 0 )); then HEALTH_DETAIL="${STACK_READER} has restarted ${restarts} time(s)"; return 1; fi
    answer="$(fct_query "${LIMEOSCTL}" --socket "${CORE_SOCKET}" reader-status)" || rc=$?
    case "${rc}" in
        0) ;;
        124) HEALTH_DETAIL="limeosctl reader-status did not answer within ${QUERY_TIMEOUT}s"; return 2 ;;
        *) HEALTH_DETAIL="core did not accept an answer from ${STACK_READER} (limeosctl reader-status exit ${rc}: ${answer})"; return 1 ;;
    esac
}
# A curl probe through the proxy: curl's exit 28 (no answer within --max-time) is unknown; any
# other failure (refused, an HTTP error with -f, a broken reply) is a definite no.
fct_http_ok() { # curl command...
    local rc=0
    "$@" || rc=$?
    case "${rc}" in
        0) return 0 ;;
        28) HEALTH_DETAIL='no answer within 5 s (curl exit 28)'; return 2 ;;
        *) HEALTH_DETAIL="curl exit ${rc}"; return 1 ;;
    esac
}
# limeosctl status answers within the query timeout; a timeout is unknown.
fct_core_status() {
    local rc=0
    fct_query "${LIMEOSCTL}" --socket "${CORE_SOCKET}" status || rc=$?
    case "${rc}" in
        0) return 0 ;;
        124) HEALTH_DETAIL="limeosctl status did not answer within ${QUERY_TIMEOUT}s"; return 2 ;;
        *) HEALTH_DETAIL="limeosctl exit ${rc}"; return 1 ;;
    esac
}
# Managed containers have the snapshot's IDs and StartedAt; HEALTH_DETAIL names any that do not.
fct_docker_matches() {
    local changed
    if ! changed="$(fct_docker_changes "${SNAP}")"; then
        HEALTH_DETAIL='docker did not answer (docker ps or docker inspect failed or timed out)'
        return 2
    fi
    changed="$(fct_lines "${changed}" | fct_managed_only "${SNAP}" | paste -sd ' ' -)"
    HEALTH_DETAIL="${changed:+changed: ${changed}}"
    [[ -z "${changed}" ]]
}
# The untouched units' states are the snapshot's; one systemd did not report is unknown.
fct_untouched_match() {
    local now unknown changed
    now="$(fct_unit_report | grep '^untouched ' || true)"
    unknown="$(awk '$3 == "unknown" || $4 == "unknown" { print $2 }' <<<"${now}" | paste -sd ' ' -)"
    if [[ -n "${unknown}" ]]; then HEALTH_DETAIL="systemd did not report the state of ${unknown}"; return 2; fi
    changed="$(comm -3 <(fct_lines "${now}" | sort) <(grep '^untouched ' "${SNAP}/units.txt" | sort) \
        | awk '{ print $2 }' | sort -u | paste -sd ' ' -)"
    HEALTH_DETAIL="${changed:+changed: ${changed}}"
    [[ -z "${changed}" ]]
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
            --scope-unit) SCOPE_UNIT="${2:?--scope-unit needs a unit}"; shift 2 ;;
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
    local requirement
    while IFS= read -r requirement; do
        fct_need "requirement: ${requirement}; nothing was changed" false
    done < <(fct_missing_requirements 0 "$(( ! DRY_RUN ))")
    if (( ! DRY_RUN )); then
        local unit
        unit="$(fct_scope_unit "${SNAP}")" || fct_die "cannot name the transient scope for ${SNAP}; nothing was changed"
        if [[ -n "${SCOPE_UNIT}" ]]; then
            [[ "${SCOPE_UNIT}" == "${unit}" ]] || fct_die "--scope-unit is internal (activate.sh passes it to itself, here ${unit}); nothing was changed"
            case "$(fct_unit_active "${unit}")" in
                active) ;;
                unknown) fct_die "could not determine whether this run is inside its transient scope ${unit} (the systemd query named above did not answer); nothing was changed" ;;
                *) fct_die "not running inside its transient scope ${unit}; nothing was changed" ;;
            esac
        fi
        fct_lock "${unit}"
        fct_refuse_if_active
        if [[ -z "${SCOPE_UNIT}" ]]; then
            # Which scope is this snapshot's, for return.sh (it never derives the name itself).
            printf '%s\n' "${unit}" >"${SNAP}/activation-scope" || fct_die "cannot record the scope in ${SNAP}/activation-scope; nothing was changed"
            if (( ATTACHED )); then fct_enter_scope "${unit}" "${original_args[@]}"; fi
            local rc=0
            fct_detach "${unit}" "${original_args[@]}" || rc=$?
            # The run's status, passed on deliberately: an exit, so the library's error report
            # does not take a failed run (already reported in its log) for an unexpected stop.
            exit "${rc}"
        fi
    else
        fct_refuse_if_active
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
    if (( ! DRY_RUN )); then exec {STEP_LOCK_FD}>>"${SNAP}/step.lock" || fct_die "cannot open ${SNAP}/step.lock"; fi

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
        fct_state_hashes strict >"${SNAP}/state.poststop.sha256"
        local record
        while IFS= read -r record || [[ -n "${record}" ]]; do
            fct_parse_record "${record}" || fct_abort "malformed state record: ${record}"
            [[ "${REC_FLAG}" == S ]] || continue
            grep -qxF -- "${record}" "${SNAP}/state.poststop.sha256" || fct_abort "state file changed before the cutover: ${REC_PATH}"
        done <"${SNAP}/state.sha256"
    else
        printf '[dry-run] hash state files again and compare strict entries with the snapshot\n'
    fi
    if (( ! DRY_RUN )); then
        case "$(fct_port_state "${port}")" in
            free) ;;
            used) fct_abort "port ${port} is still in use after the frozen app stopped" ;;
            *) fct_abort "could not determine whether port ${port} is free after the frozen app stopped (ss -H -ltn failed or timed out)" ;;
        esac
    fi

    # 2. Install the pinned package; its postinst starts the services too. Never prompt, and
    # keep the operator's /etc/limeos/core.json (a conffile) when it differs from the package's.
    fct_do env DEBIAN_FRONTEND=noninteractive dpkg --force-confold -i "${package}"
    if (( DRY_RUN )); then
        printf '[dry-run] check installed limeos version is %s\n' "${version}"
    else
        local installed
        installed="$(fct_pkg_version limeos)"
        if [[ "${installed}" == unknown ]]; then
            fct_abort "could not determine the installed limeos version (dpkg-query failed or timed out, named above)"
        elif [[ "${installed}" != "${version}" ]]; then
            fct_abort "installed limeos version is not ${version} (dpkg reports ${installed})"
        fi
    fi
    fct_do systemctl daemon-reload
    for unit in "${RUST_UNITS[@]}"; do fct_do systemctl enable "${unit}"; done
    # Every start of core counts against its start limit (ten in six hours), successful ones
    # included: a deliberate start begins from a clear count. Nothing reads core's failed
    # state or Result before this.
    fct_do systemctl reset-failed limeos-core.service
    for unit in "${RUST_UNITS[@]}"; do fct_do systemctl start "${unit}"; done
    fct_retry 'Rust services active' fct_units_active "${RUST_UNITS[@]}"
    # Stack status and whole-stack actions stay unavailable until the reader runs. It has no
    # Install section on purpose: start it now, never enable it; return.sh stops it. Drop-ins on
    # the LimeOS units are named first: they change what the packaged units run as.
    if (( ! DRY_RUN )); then
        local dropin
        while IFS= read -r dropin; do
            fct_warn "LimeOS unit drop-in (systemctl cat to read it): ${dropin}"
        done < <(fct_dropin_report)
    fi
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
    local caddy_now=inactive
    if (( ! DRY_RUN )); then caddy_now="$(fct_unit_active caddy.service)"; fi
    case "${caddy_now}" in
        active) fct_do systemctl reload caddy.service ;;
        unknown)
            fct_warn 'could not determine whether caddy.service is running (named above); using reload-or-restart, which loads the site either way (reloads a running Caddy, starts a stopped one)'
            fct_do systemctl reload-or-restart caddy.service
            ;;
        *) fct_do systemctl start caddy.service ;;
    esac

    # 4. Health checks.
    local curl_base=(curl -fsS -k --max-time 5 --resolve "${host}:${port}:127.0.0.1" -o /dev/null)
    fct_retry 'core health via the proxy' fct_http_ok "${curl_base[@]}" "${site}/api/v1/health"
    fct_retry 'login page via the proxy' fct_http_ok "${curl_base[@]}" "${site}/"
    fct_retry 'limeosctl status' fct_core_status
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

trap 'fct_on_signal INT' INT
trap 'fct_on_signal TERM' TERM
trap 'fct_on_signal HUP' HUP
fct_main "$@"
