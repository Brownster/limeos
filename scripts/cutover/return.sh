#!/usr/bin/env bash
# T6: return to the frozen app. Removes the proxy site, returns the Rust services and
# the frozen units to their snapshot state, then verifies state hashes,
# container IDs and the frozen port. Idempotent; best effort, then verified.
# The package stays installed (removal is the operator's separate decision).
set -Eeuo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

SITE_NAME=limeos-cutover.caddy
FAILURES=0
FORCE=0
HASHES_CAPTURED=0
WRITERS_RUNNING=0
declare -A CURRENT_HASHES=()
declare -A UNHASHABLE=()
# How long --force waits for the activation's scope to empty after SIGKILL. Only a process in
# uninterruptible I/O outlives SIGKILL, and giving up early only stops this return.
FORCE_WAIT="${CUTOVER_FORCE_WAIT:-15}"
[[ "${FORCE_WAIT}" =~ ^[0-9]+$ ]] || FORCE_WAIT=15
# Written by --force before it ends the activation, removed once the activation's scope is empty.
TAKEOVER=force-takeover

fct_usage() {
    cat <<'USAGE'
Usage: return.sh SNAPSHOT_DIR [--site-dir DIR] [--force] [--dry-run] [--help]

Restores the frozen app from a preflight snapshot. Safe to run repeatedly.
Exit 0 only when hashes, the managed containers' IDs and StartedAt (from the
snapshot's managed.txt; other containers only warn) and the frozen port all
verify. Rust units return to their snapshot state.

It first takes SNAPSHOT_DIR/return.lock, so only one return.sh runs per snapshot,
and holds it to the end. Then it takes activate.lock without waiting (activate.sh's
own rollback uses the lock it already holds). While an activation holds it,
return.sh refuses at once, before any systemd query, and names that
activation's pid: it rolls back by itself on failure, and kill -TERM <pid> makes
it roll back now. activate.sh's rollback waits for return.lock for as long as
another return.sh holds it, saying every 30 s who that is. Every systemctl query
gives up after 30 s with a named failure. It also refuses while the
activation's transient scope (limeos-cutover-activate-*.scope) still has
processes, such as a step that outlived activate.sh.

--force is for a hung activation: it ends the activation by its scope, by
writing 1 to the scope's cgroup.kill (or, where that cannot be used,
systemctl kill --kill-whom=all --signal=KILL UNIT): activate.sh, its automatic
rollback and every step, including any started meanwhile, and nothing outside
the scope. It waits up to 15 s for the scope to empty, and only then takes
activate.lock and returns. If the scope does not empty (a process in
uninterruptible I/O), it stops having changed nothing else, lists what is left
and says what to run next; SNAPSHOT_DIR/force-takeover records the unfinished
takeover so a later return.sh explains it. If the activation was inside dpkg,
run dpkg --audit afterwards.
USAGE
}

fct_fail() { FAILURES=$(( FAILURES + 1 )); printf 'FAIL: %s\n' "$*" >&2; }

# Run a step; record a failure and keep going so one error cannot block the rest.
fct_step() {
    if ! fct_run "$@"; then fct_fail "command failed: $*"; fi
}

fct_stop_proxy() {
    local site_file="${1}" caddy_state="${2}"
    fct_step rm -f -- "${site_file}"
    if [[ "${caddy_state}" == active ]]; then
        # Caddy served other sites before the cutover: reload without our site.
        if [[ "$(fct_unit_active caddy.service)" == active ]]; then fct_step systemctl reload caddy.service; fi
    else
        fct_step systemctl stop caddy.service
        if [[ "${caddy_enabled}" != enabled ]]; then fct_step systemctl disable caddy.service; fi
    fi
}

fct_restore_frozen() {
    local snap="${1}" class unit active enabled
    for unit in "${FROZEN_UNITS[@]}"; do
        read -r class _ active enabled < <(grep -m1 -F "managed ${unit} " "${snap}/units.txt" || echo 'managed - inactive disabled')
        [[ "${enabled}" == enabled ]] && fct_step systemctl enable "${unit}"
        [[ "${active}" == active ]] && fct_step systemctl start "${unit}"
    done
    : "${class:-}"
}

# Rust units back to their snapshot state: stopped and disabled unless they already ran or
# were enabled before the cutover (a package installed earlier, for example).
fct_restore_rust() {
    local snap="${1}" unit active enabled
    for unit in limeos-core.service limeos-containerd.service limeos-storaged.service; do
        read -r _ _ active enabled < <(grep -m1 -F "managed ${unit} " "${snap}/units.txt" || echo 'managed - inactive disabled')
        if [[ "${active}" != active ]]; then fct_step systemctl stop "${unit}"; fi
        if [[ "${enabled}" != enabled ]]; then fct_step systemctl disable "${unit}"; fi
    done
    for unit in "${RUST_UNITS[@]}"; do
        read -r _ _ active enabled < <(grep -m1 -F "managed ${unit} " "${snap}/units.txt" || echo 'managed - inactive disabled')
        if [[ "${enabled}" == enabled ]]; then fct_step systemctl enable "${unit}"; fi
        if [[ "${active}" == active ]]; then fct_step systemctl start "${unit}"; fi
    done
}

# The paths of the well-formed records in a hash file, one per line.
fct_record_paths() { # file
    local record
    while IFS= read -r record || [[ -n "${record}" ]]; do
        if fct_parse_record "${record}"; then printf '%s\n' "${REC_PATH}"; fi
    done <"${1}"
}

# How often activate.sh's rollback says who holds return.lock while it waits for it.
LOCK_REPORT_INTERVAL="${CUTOVER_LOCK_REPORT_INTERVAL:-30}"
[[ "${LOCK_REPORT_INTERVAL}" =~ ^[1-9][0-9]*$ ]] || LOCK_REPORT_INTERVAL=30

# Who holds a lock, for the rollback's waiting messages: role, pid, process state and since when.
fct_waiting_text() { # lock
    local since state='' stat=''
    since="$(date -u -r "${1}" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo '?')"
    if ! fct_live_holder "${1}"; then
        printf 'held by an unknown process (record %s %s, stale; find it with: fuser -v %s)' "${HOLDER_ROLE}" "${HOLDER_PID}" "${1}"
        return 0
    fi
    { IFS= read -r stat <"/proc/${HOLDER_PID}/stat"; } 2>/dev/null || true
    stat="${stat##*) }"
    state="${stat%% *}"
    printf 'held by %s.sh (pid %s, state %s) since %s' "${HOLDER_ROLE}" "${HOLDER_PID}" "${state:-?}" "${since}"
    if [[ "${state}" == T ]]; then printf '; it is stopped (Ctrl-Z?): resume it with kill -CONT %s' "${HOLDER_PID}"; fi
}

# The snapshot's locks. return.lock (fd 8) admits one return.sh per snapshot and is held for the
# whole run, a forced takeover included, so no other return can slip in while an activation is
# being ended or after it refuses. activate.lock (fd 9) is the activation's: activate.sh's
# rollback runs this script with it already held on fd 9 (the same open file, so the same lock)
# and from inside the activation's scope.
fct_lock_snapshot() { # snap
    local snap="${1}" lock="${1}/activate.lock" rlock="${1}/return.lock" rollback=0
    if [[ /proc/self/fd/9 -ef "${lock}" ]] && flock -n 9; then rollback=1; fi
    exec 8>>"${rlock}" || fct_die "cannot open ${rlock}"
    if (( rollback )); then
        # The old service is down, so the rollback never gives up on return.lock. Its holder is a
        # live return.sh that either finishes the return or (seeing this activation's lock) refuses
        # at once and releases it; if it dies, the kernel releases it. Say who holds it meanwhile.
        until flock -w "${LOCK_REPORT_INTERVAL}" 8; do
            fct_warn "ROLLBACK: waiting for ${rlock}, $(fct_waiting_text "${rlock}"); this rollback waits until it is released"
        done
    elif ! flock -n 8; then
        fct_die "$(fct_lock_holder_text "${rlock}")"
    fi
    fct_record_holder "${rlock}" return
    if (( ! rollback )); then fct_claim_activation "${snap}"; fi
    fct_record_holder "${lock}" return
}

# The scope this snapshot's activation ran in, as activate.sh recorded it; nothing for a
# snapshot never activated (or activated before scopes).
fct_recorded_scope() { # snap
    local file="${1}/activation-scope" unit=''
    [[ -e "${file}" ]] || return 0
    { IFS= read -r unit <"${file}"; } 2>/dev/null || true
    [[ "${unit}" =~ ${SCOPE_RE} ]] || fct_die "${file} does not name an activation scope (${unit@Q}); nothing was changed"
    printf '%s\n' "${unit}"
}

# activate.lock, and an activation scope with no process left in it: only then has the
# activation ended, whoever still holds or has released its lock. Holding return.lock and
# activate.lock, nothing can join the scope again (a new activation needs activate.lock).
# --force kills only when a live activate.sh holds this snapshot's lock from inside the scope
# the snapshot records; with the lock free there is no such process to vouch for the unit.
fct_claim_activation() { # snap
    local snap="${1}" lock="${1}/activate.lock" unit cgroup='' held=0 empty=0
    exec 9>>"${lock}" || fct_die "cannot open ${lock}"
    flock -n 9 || held=1
    # A running activation: a plain return refuses at once, before any systemd query, so it never
    # holds return.lock across something slow while that activation's rollback may need it.
    if (( held && ! FORCE )); then
        if [[ -e "${snap}/${TAKEOVER}" ]]; then
            fct_die "$(fct_lock_holder_text "${lock}"); an earlier return.sh --force did not finish (${snap}/${TAKEOVER})"
        fi
        fct_die "$(fct_lock_holder_text "${lock}")"
    fi
    unit="$(fct_recorded_scope "${snap}")"
    if [[ -n "${unit}" ]]; then cgroup="$(fct_scope_cgroup "${unit}")"; fi
    if (( ! held )); then
        if [[ -z "${unit}" ]]; then return 0; fi
        fct_scope_empty "${unit}" "${cgroup}" || empty=$?
        if (( empty == 0 )); then
            fct_finish_takeover "${snap}" "${unit}"
            return 0
        fi
        if (( empty == 2 )); then
            fct_die "cannot tell whether the activation's scope ${unit} is empty (the systemd query named above did not answer); this return changed nothing; run it again"
        fi
        if [[ -e "${snap}/${TAKEOVER}" ]]; then
            fct_refuse_scope "${snap}" "${unit}" "${cgroup}" "$(fct_takeover_text "${snap}") did not finish: its scope ${unit} still has processes:" \
                "The activation's automatic rollback was ended, and the old service is still stopped; this return changed nothing."
        fi
        fct_refuse_scope "${snap}" "${unit}" "${cgroup}" \
            "activate.sh is no longer running, but its scope ${unit} still has processes (a step it left behind may still change packages or restart services):" \
            "This return changed nothing; --force does not end them either, since no activation holds ${lock} to vouch for the unit. To end them yourself, after checking they are this activation's (systemd-cgls --unit ${unit}): systemctl kill --kill-whom=all --signal=KILL ${unit}"
    fi
    fct_check_holder_in_scope "${lock}" "${unit}" "${cgroup}"
    fct_end_scope "${snap}" "${unit}" "${cgroup}"
    if (( held )); then
        local deadline=$(( SECONDS + 5 ))
        until flock -n 9; do
            if (( SECONDS >= deadline )); then
                fct_finish_takeover "${snap}" "${unit}"
                fct_die "--force: the activation's scope ${unit} is empty, but ${lock} is still held by a process outside it (find it: fuser -v ${lock}); nothing was changed on the host by this return"
            fi
            fct_pause
        done
    fi
    fct_finish_takeover "${snap}" "${unit}"
}

# --force acts only on a current activate.sh record (pid and start time) that names this
# snapshot's scope, for a process that is inside that scope's control group. Otherwise it
# stops before killing anything.
fct_check_holder_in_scope() { # lock unit cgroup
    local lock="${1}" unit="${2}" cgroup="${3}" pid=''
    local -a members=()
    if ! fct_live_holder "${lock}"; then
        fct_die "--force: $(fct_lock_holder_text "${lock}"); nothing was killed or changed"
    fi
    [[ "${HOLDER_ROLE}" == activate ]] \
        || fct_die "--force overrides only an activation, but ${lock} is held by ${HOLDER_ROLE}.sh (pid ${HOLDER_PID}); nothing was killed or changed"
    [[ -n "${unit}" ]] \
        || fct_die "--force: this snapshot records no activation scope (${lock%/*}/activation-scope), so the activation holding ${lock} (pid ${HOLDER_PID}) cannot be ended by unit; nothing was killed or changed"
    [[ "${HOLDER_UNIT}" == "${unit}" ]] \
        || fct_die "--force: the activation holding ${lock} (pid ${HOLDER_PID}) records scope ${HOLDER_UNIT:-none}, not this snapshot's ${unit}; nothing was killed or changed"
    if [[ -n "${cgroup}" ]]; then
        { mapfile -t members <"${CUTOVER_ROOT}/sys/fs/cgroup${cgroup}/cgroup.procs"; } 2>/dev/null || members=()
    fi
    for pid in "${members[@]}"; do [[ "${pid}" == "${HOLDER_PID}" ]] && break; pid=''; done
    # Checked again afterwards: the member found is the recorded process, not a reused pid.
    if [[ -z "${pid}" ]] || ! fct_proc_running "${HOLDER_PID}" "${HOLDER_START}"; then
        fct_die "--force: activate.sh (pid ${HOLDER_PID}) holding ${lock} is not inside ${unit} (control group ${cgroup:-unknown}); nothing was killed or changed"
    fi
}

# The scope's control group path (/system.slice/UNIT), or nothing when systemd has no such unit.
fct_scope_cgroup() { # unit
    local cgroup
    cgroup="$(fct_query systemctl show -p ControlGroup --value "${1}" || true)"
    if [[ "${cgroup}" == /*"/${1}" && "${cgroup}" != *..* ]]; then printf '%s\n' "${cgroup}"; fi
}

# The scope has no process left: systemd reports it inactive (it goes inactive only once its
# cgroup is empty, and a kill never stops it early), and its cgroup, when known, is gone or
# unpopulated. Fails with 1 while it has processes, 2 when systemd did not answer in time.
fct_scope_empty() { # unit [cgroup]
    local state='' events='' rc=0
    state="$(fct_query systemctl is-active "${1}")" || rc=$?
    if (( rc == 124 )); then return 2; fi
    [[ "${state}" == inactive || "${state}" == failed ]] || return 1
    [[ -n "${2:-}" ]] || return 0
    { events="$(<"${CUTOVER_ROOT}/sys/fs/cgroup${2}/cgroup.events")"; } 2>/dev/null || return 0
    [[ "${events}" != *'populated 1'* ]]
}

# "  PID STAT ARGS" for each process still in the scope. Every read tolerates a process (or the
# whole cgroup) vanishing meanwhile, so the list always completes.
fct_scope_list() { # unit cgroup
    local pid line listed=0
    local -a pids=()
    if [[ -n "${2}" ]]; then
        { mapfile -t pids <"${CUTOVER_ROOT}/sys/fs/cgroup${2}/cgroup.procs"; } 2>/dev/null || pids=()
    fi
    for pid in "${pids[@]}"; do
        [[ "${pid}" =~ ^[0-9]+$ ]] || continue
        listed=1
        line="$(ps -o pid=,stat=,args= -p "${pid}" 2>/dev/null || true)"
        printf '  %s\n' "${line:-${pid} (exited while this list was made)}"
    done
    if (( ! listed )); then printf '  (no list available; see: systemd-cgls --unit %s)\n' "${1}"; fi
    return 0
}

# Refuse with what is still in the scope and the exact next step; changes nothing.
fct_refuse_scope() { # snap unit cgroup headline consequence
    {
        printf 'ERROR: %s\n' "${4}"
        fct_scope_list "${2}" "${3}"
        printf '%s\n' "${5}"
        printf 'Inspect: systemctl status %s; systemd-cgls --unit %s\n' "${2}" "${2}"
        printf "Next: wait until 'systemctl is-active %s' prints inactive, then run: return.sh %s --force\n" "${2}" "${1}"
    } >&2 || true
    exit 1
}

# "a forced return (return.sh --force, pid P, started T)" from the takeover marker.
fct_takeover_text() { # snap
    local pid started
    pid="$(fct_meta_get "${1}/${TAKEOVER}" pid 2>/dev/null || true)"
    started="$(fct_meta_get "${1}/${TAKEOVER}" started 2>/dev/null || true)"
    printf 'a forced return (return.sh --force, pid %s, started %s) ended this snapshot'"'"'s activation but' "${pid:-?}" "${started:-?}"
}

# Kill every process in the scope. Preferably by writing 1 to the scope's cgroup.kill (cgroup v2,
# kernel 5.14 or newer): the kernel kills the whole cgroup at once, a process forked meanwhile
# included, and no pid is involved. Only when that file cannot be used (no cgroup2 mount, an
# older kernel, a failed write) does it fall back to systemctl kill, where systemd lists the
# cgroup's pids and signals each one. Says which it used.
fct_kill_scope() { # unit cgroup
    local unit="${1}" cgroup="${2}" base="${CUTOVER_ROOT}/sys/fs/cgroup" dir why=''
    dir="${base}${cgroup}"
    if [[ -z "${cgroup}" ]]; then
        why="systemd reports no control group for ${unit}"
    elif [[ "$(stat -f -c %T -- "${base}" 2>/dev/null || true)" != cgroup2fs \
            || "$(stat -f -c %T -- "${dir}" 2>/dev/null || true)" != cgroup2fs ]]; then
        why="${dir} is not on a cgroup2 file system"
    elif [[ -L "${dir}" || ! -d "${dir}" ]]; then
        why="${dir} is not the scope's control group directory"
    elif [[ ! -e "${dir}/cgroup.kill" ]]; then
        why="${dir} has no cgroup.kill (kernel before 5.14)"
    elif ! { printf '1\n' >"${dir}/cgroup.kill"; } 2>/dev/null; then
        why="could not write 1 to ${dir}/cgroup.kill"
    else
        fct_warn "--force: killed every process in scope ${unit} at once by writing 1 to ${dir}/cgroup.kill (the kernel's cgroup kill; no pid involved)"
        return 0
    fi
    fct_warn "--force: ${why}; using systemctl kill --kill-whom=all --signal=KILL ${unit} instead (systemd signals each process it finds in the scope)"
    # Fails harmlessly when the unit no longer exists (the scope has emptied already). systemd
    # rereads the cgroup until it finds no new process, and a process with SIGKILL pending
    # cannot complete a fork.
    fct_query systemctl kill --kill-whom=all --signal=KILL "${unit}" >/dev/null || true
}

# --force: end the activation by its scope, not by pid (fct_kill_scope), touching nothing
# outside it. Waits, bounded, for the scope to empty; if it does not, stops having changed
# nothing else, holding no lock, and leaves the marker for the next return.sh.
fct_end_scope() { # snap unit cgroup
    local snap="${1}" unit="${2}" cgroup="${3}" deadline
    printf 'unit=%s\npid=%s\nstarted=%s\n' "${unit}" "$$" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"${snap}/${TAKEOVER}" \
        || fct_warn "could not record the takeover in ${snap}/${TAKEOVER}"
    fct_warn "--force: ending the activation, every process in scope ${unit} (activate.sh, its rollback and its steps); if it was inside dpkg, run dpkg --audit after this return"
    fct_kill_scope "${unit}" "${cgroup}"
    deadline=$(( SECONDS + FORCE_WAIT ))
    until fct_scope_empty "${unit}" "${cgroup}"; do
        if (( SECONDS >= deadline )); then
            fct_refuse_scope "${snap}" "${unit}" "${cgroup}" \
                "--force: the activation's scope ${unit} still has processes ${FORCE_WAIT}s after SIGKILL (a process in uninterruptible I/O, such as dpkg waiting on a disk, cannot exit until that I/O ends):" \
                "The activation's automatic rollback was ended with it, and the old service is still stopped; this return changed nothing else."
        fi
        fct_pause
    done
}

# The activation's scope is empty: a takeover recorded by an earlier --force is complete.
fct_finish_takeover() { # snap unit
    [[ -e "${1}/${TAKEOVER}" ]] || return 0
    rm -f -- "${1}/${TAKEOVER}" || fct_die "cannot remove ${1}/${TAKEOVER}"
    fct_log "return: the activation's scope ${2} is now empty; the forced takeover recorded in ${1}/${TAKEOVER} is complete"
}

# A step activate.sh could not end (SNAP/stuck-step: "PID START COMMAND") may still change
# packages or restart services behind this return: report it with the exact follow-up.
fct_check_stuck_step() { # snap
    local file="${1}/stuck-step" pid start command
    [[ -f "${file}" ]] || return 0
    read -r pid start command <"${file}" || true
    if [[ "${pid}" =~ ^[0-9]+$ && -n "${start}" && "$(fct_proc_start "${pid}")" == "${start}" ]]; then
        fct_fail "the activation's step (pid ${pid}: ${command}) is still running and may still change packages or restart services. When it has exited (check: ps -o pid,stat,args -p ${pid}), run dpkg --audit, then dpkg --configure -a or dpkg --remove limeos as it reports, and run return.sh ${1} again"
    else
        fct_log "return: the activation's stuck step (pid ${pid:-?}: ${command:-?}) has exited since; run dpkg --audit to check the package state"
    fi
}

fct_frozen_writer_running() {
    local unit
    for unit in "${FROZEN_UNITS[@]}"; do
        [[ "$(fct_unit_active "${unit}")" == active ]] && return 0
    done
    return 1
}

# Hash the state before the frozen writers start again (a restored timer can rewrite its
# live files within seconds). A failure is recorded and never stops the restore.
fct_capture_hashes() {
    local hashes record
    # A running frozen writer (the app, or a timer that fires at once) may legitimately
    # rewrite its live databases, so their baseline comparison only applies while all stopped.
    if fct_frozen_writer_running; then WRITERS_RUNNING=1; fi
    # Each path on its own: one that cannot be hashed fails alone, the rest are still verified.
    if ! hashes="$(fct_state_hashes tolerant)"; then
        fct_fail 'cannot list the state files; state verification skipped'
        return 0
    fi
    # Current hashes by exact path (any byte but newline).
    while IFS= read -r record || [[ -n "${record}" ]]; do
        if [[ "${record}" == '! '* ]]; then
            UNHASHABLE["${record#! }"]=1
        elif fct_parse_record "${record}"; then
            CURRENT_HASHES["${REC_PATH}"]="${REC_HASH}"
        fi
    done <<<"${hashes}"
    HASHES_CAPTURED=1
}

# Every path is checked against the post-stop baseline whenever activate recorded one (a live
# file that went A -> B -> A must not pass because it matches preflight again), strict files
# against the preflight snapshot as well; with no baseline, against the preflight snapshot.
fct_verify_hashes() {
    local snap="${1}" flag hash path record cur_hash base_hash baseline writers="${WRITERS_RUNNING}"
    local have_baseline=0 against_baseline=0 against_preflight=0
    local -A base=()
    (( HASHES_CAPTURED )) || return 0
    baseline="${snap}/state.poststop.sha256"
    if [[ -f "${baseline}" ]]; then
        have_baseline=1
        while IFS= read -r record || [[ -n "${record}" ]]; do
            [[ -n "${record}" ]] || continue
            if fct_parse_record "${record}"; then base["${REC_PATH}"]="${REC_HASH}"; else fct_fail "malformed record in the post-stop baseline: ${record}"; fi
        done <"${baseline}"
    fi
    while IFS= read -r record || [[ -n "${record}" ]]; do
        [[ -n "${record}" ]] || continue
        if ! fct_parse_record "${record}"; then fct_fail "malformed state record in the snapshot: ${record}"; continue; fi
        flag="${REC_FLAG}" hash="${REC_HASH}" path="${REC_PATH}"
        if [[ -n "${UNHASHABLE["${path}"]:-}" ]]; then fct_fail "cannot verify state file: ${path} (named above)"; continue; fi
        cur_hash="${CURRENT_HASHES["${path}"]:-ABSENT}"
        base_hash="${base["${path}"]:-ABSENT}"
        if [[ "${flag}" == S ]]; then
            against_preflight=$(( against_preflight + 1 ))
            [[ "${cur_hash}" == "${hash}" ]] || fct_fail "state file changed: ${path} (checked against the preflight snapshot)"
            if (( have_baseline )); then
                against_baseline=$(( against_baseline + 1 ))
                [[ "${cur_hash}" == "${base_hash}" ]] || fct_fail "state file changed: ${path} (checked against the post-stop baseline)"
            fi
        elif (( have_baseline )); then
            against_baseline=$(( against_baseline + 1 ))
            if [[ "${cur_hash}" != "${base_hash}" ]]; then
                if (( writers )); then
                    fct_warn "live state file differs from the post-stop baseline (frozen writers are running): ${path}"
                else
                    fct_fail "live state file changed after the writers stopped: ${path} (checked against the post-stop baseline)"
                fi
            fi
        else
            against_preflight=$(( against_preflight + 1 ))
            if [[ "${cur_hash}" != "${hash}" ]]; then
                fct_warn "live state file differs from the preflight snapshot (no post-stop baseline, so writers may have run): ${path}"
            fi
        fi
    done <"${snap}/state.sha256"
    fct_log "return: state checks: ${against_baseline} against the post-stop baseline, ${against_preflight} against the preflight snapshot"
}

# Only the managed set must match the snapshot; other changes are named, not failures.
fct_verify_docker() {
    local snap="${1}" ids started
    ids="$(fct_docker_diff_names "${snap}/docker-ps.txt" <(fct_docker_ids) | fct_managed_only "${snap}" | paste -sd ' ' -)"
    started="$(fct_docker_diff_names "${snap}/docker-started.txt" <(fct_docker_started) | fct_managed_only "${snap}" | paste -sd ' ' -)"
    if [[ -n "${ids}" ]]; then fct_fail "docker ps IDs differ from the snapshot: ${ids}"; fi
    if [[ -n "${started}" ]]; then fct_fail "container StartedAt differs from the snapshot (a restart happened): ${started}"; fi
    fct_warn_unmanaged_changes "${snap}"
}

fct_verify_port() {
    local port="${1}" attempt code=''
    for (( attempt=0; attempt<20; attempt++ )); do
        code="$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' "http://127.0.0.1:${port}/" 2>/dev/null || true)"
        case "${code}" in 200|301|302|303|307|308|401) return 0 ;; esac
        (( DRY_RUN )) && return 0
        sleep 1
    done
    fct_fail "port ${port} does not serve the frozen app over HTTP (last code: ${code:-none})"
}

fct_verify_untouched() {
    local snap="${1}" class unit active enabled now
    while read -r class unit active enabled || [[ -n "${class}" ]]; do
        [[ "${class}" == untouched ]] || continue
        now="$(fct_unit_active "${unit}")"
        [[ "${now}" == "${active}" ]] || fct_fail "untouched unit changed state: ${unit}"
    done <"${snap}/units.txt"
}

fct_main() {
    local snap='' site_dir='/etc/caddy/conf.d'
    while (( $# )); do
        case "${1}" in
            --dry-run) DRY_RUN=1; shift ;;
            --site-dir) site_dir="${2:?--site-dir needs a directory}"; shift 2 ;;
            --force) FORCE=1; shift ;;
            -h|--help) fct_usage; return 0 ;;
            -*) fct_usage >&2; fct_die "unknown argument: ${1}" ;;
            *) [[ -z "${snap}" ]] || fct_die 'only one snapshot directory is accepted'; snap="${1}"; shift ;;
        esac
    done
    [[ -n "${snap}" ]] || { fct_usage >&2; fct_die 'a snapshot directory is required'; }
    fct_require_root
    fct_snapshot_check "${snap}" 0
    if (( ! DRY_RUN )); then fct_lock_snapshot "${snap}"; fi
    # return.sh is the safety path, so it still accepts a snapshot from before state lists
    # and managed sets, with the rules that snapshot was taken under.
    if [[ -f "${snap}/state-paths.txt" ]]; then
        STATE_LIST="${snap}/state-paths.txt"
    elif [[ -z "${CUTOVER_STATE_FILES:-}" ]]; then
        fct_warn 'older snapshot with no state-paths.txt: comparing the paths recorded in its state.sha256'
        CUTOVER_STATE_FILES="$(fct_record_paths "${snap}/state.sha256")"
    fi
    if [[ ! -f "${snap}/managed.txt" ]]; then
        fct_warn 'older snapshot with no managed.txt: every container must match the snapshot'
    fi
    local port caddy_state caddy_enabled class unit active enabled
    port="$(fct_meta_get "${snap}/meta.env" frozen_port)"
    read -r class unit caddy_state caddy_enabled < <(grep -m1 -F 'managed caddy.service ' "${snap}/units.txt" || echo 'managed caddy.service inactive disabled')

    fct_log 'return: stopping the proxy site'
    fct_stop_proxy "${CUTOVER_ROOT}${site_dir}/${SITE_NAME}" "${caddy_state}"
    fct_log 'return: restoring the Rust services to their snapshot state'
    # The stack reader has no Install section: activate.sh starts it (never enables it) and
    # it is stopped here, whoever started it.
    if [[ "$(fct_unit_active limeos-storage-reader.service)" == active ]]; then
        fct_step systemctl stop limeos-storage-reader.service
    fi
    fct_restore_rust "${snap}"
    # Restore first, verify afterwards: nothing in verification may keep the frozen app down.
    if (( ! DRY_RUN )); then
        fct_log 'return: hashing the state files'
        fct_capture_hashes
    fi
    fct_log 'return: restoring the frozen units'
    fct_restore_frozen "${snap}"

    if (( DRY_RUN )); then fct_log 'dry-run: verification commands are skipped'; return 0; fi
    fct_log 'return: verifying'
    fct_check_stuck_step "${snap}"
    fct_verify_hashes "${snap}"
    fct_verify_docker "${snap}"
    fct_verify_untouched "${snap}"
    fct_verify_port "${port}"
    if (( FAILURES > 0 )); then fct_die "return finished with ${FAILURES} failure(s); do not retry cutover until resolved"; fi
    fct_log 'return verified: frozen app serving, state and containers match the snapshot'
}

fct_main "$@"
