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

fct_usage() {
    cat <<'USAGE'
Usage: return.sh SNAPSHOT_DIR [--site-dir DIR] [--force] [--dry-run] [--help]

Restores the frozen app from a preflight snapshot. Safe to run repeatedly.
Exit 0 only when hashes, the managed containers' IDs and StartedAt (from the
snapshot's managed.txt; other containers only warn) and the frozen port all
verify. Rust units return to their snapshot state.

It takes the snapshot's activate.lock without waiting (activate.sh's own rollback
uses the lock it already holds). While an activation holds it, return.sh refuses
and names that activation's pid: it rolls back by itself on failure, and
kill -TERM <pid> makes it roll back now. --force goes on regardless, only for an
activation that is hung.
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

# The snapshot's activate.lock, without waiting. activate.sh's rollback runs this script with
# the lock already held on fd 9 (the same open file, so the same lock).
fct_lock_snapshot() { # snap
    local lock="${1}/activate.lock"
    if [[ /proc/self/fd/9 -ef "${lock}" ]] && flock -n 9; then return 0; fi
    exec 9>>"${lock}" || fct_die "cannot open ${lock}"
    if ! flock -n 9; then
        if (( FORCE )) && fct_live_holder "${lock}"; then
            fct_warn "activate.sh (pid ${HOLDER_PID}) still holds ${lock}; going on because of --force"
            return 0
        fi
        fct_die "$(fct_lock_holder_text "${lock}")"
    fi
    fct_record_holder "${lock}" return
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

fct_verify_hashes() {
    local snap="${1}" flag hash path record cur_hash baseline writers="${WRITERS_RUNNING}"
    (( HASHES_CAPTURED )) || return 0
    baseline="${snap}/state.poststop.sha256"
    while IFS= read -r record || [[ -n "${record}" ]]; do
        [[ -n "${record}" ]] || continue
        if ! fct_parse_record "${record}"; then fct_fail "malformed state record in the snapshot: ${record}"; continue; fi
        flag="${REC_FLAG}" hash="${REC_HASH}" path="${REC_PATH}"
        if [[ -n "${UNHASHABLE["${path}"]:-}" ]]; then fct_fail "cannot verify state file: ${path} (named above)"; continue; fi
        cur_hash="${CURRENT_HASHES["${path}"]:-ABSENT}"
        if [[ "${flag}" == S ]]; then
            [[ "${cur_hash}" == "${hash}" ]] || fct_fail "state file changed: ${path}"
        elif [[ "${cur_hash}" != "${hash}" ]]; then
            # Live databases may change before the writers stop; compare to the
            # post-stop baseline when activate recorded one.
            if (( writers )); then
                fct_warn "live state file differs from the snapshot (frozen writers are running): ${path}"
            elif [[ -f "${baseline}" ]] && ! grep -qxF "${flag} ${cur_hash} ${path}" "${baseline}"; then
                fct_fail "live state file changed after the writers stopped: ${path}"
            elif [[ ! -f "${baseline}" ]]; then
                fct_warn "live state file differs from the snapshot (writers may have run): ${path}"
            fi
        fi
    done <"${snap}/state.sha256"
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
    fct_verify_hashes "${snap}"
    fct_verify_docker "${snap}"
    fct_verify_untouched "${snap}"
    fct_verify_port "${port}"
    if (( FAILURES > 0 )); then fct_die "return finished with ${FAILURES} failure(s); do not retry cutover until resolved"; fi
    fct_log 'return verified: frozen app serving, state and containers match the snapshot'
}

fct_main "$@"
