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
# The services behind them (the timers start the last two). One that declares StateDirectory=limeos
# owns /var/lib/limeos as far as systemd is concerned: every start of it sets the directory's mode
# and, when its owner differs from the unit's User=/Group=, chowns it and everything in it to them.
# On wybie pi-health.service and limeos-metrics-collector.service do (holly:pihealth, 0750), so one
# start of either while LimeOS runs takes LimeOS's own state away from it.
FROZEN_SERVICES=(pihealth-helper.service pi-health.service limeos-metrics-collector.service limeos-package-reconcile.service)
SHARED_STATE_DIR=/var/lib/limeos
# Directories the frozen app and LimeOS share; preflight records each one's owner and mode.
SHARED_DIRS=(/etc/limeos /var/lib/limeos)
# The drop-in activate.sh puts on each such unit while LimeOS owns the directory: it makes a start
# of the unit fail (its assertion names the drop-in itself), so a `systemctl start pi-health` by hand
# cannot chown LimeOS's state. return.sh removes it before it starts the frozen units again.
GUARD_NAME=zz-limeos-cutover-guard.conf
# LimeOS's trees inside the shared state directory, "PATH ACCOUNT": everything in each belongs to
# that account (and its primary group). The executors directory itself is root's, not recursively.
PACKAGE_TREES=("/var/lib/limeos/core limeos-core" "/var/lib/limeos/assistant limeos-assistant"
    "/var/lib/limeos/executors/container limeos-containerd" "/var/lib/limeos/executors/storage root")
PACKAGE_EXECUTORS_DIR=/var/lib/limeos/executors
# Units that are never touched; limeops-* is discovered at run time.
UNTOUCHED_FIXED=(limeos-agent.service flask-shutdown.service)
# Rust services in start order.
RUST_UNITS=(limeos-storaged.service limeos-containerd.service limeos-core.service)
DEFAULT_MANAGED=(jellyfin radarr sonarr sabnzbd transmission jackett vpn navidrome audiobookshelf rdtclient requestrr get_iplayer yt-to-jellyfin watchtower)
FROZEN_PORT_DEFAULT=8002
# Stack status and whole-stack actions: the dormant read-only reader (never enabled) loads this catalog.
STACK_READER=limeos-storage-reader.service
STACK_READER_SOCKET=/run/limeos-storage-reader/executor.sock
# The account the stack reader runs as (PR-32); core refuses a reader socket anyone else owns.
STACK_READER_ACCOUNT=limeos-storage-reader
CATALOG_PATH=/etc/limeos/management-catalog.json
LIMEOSCTL="${LIMEOSCTL:-/usr/lib/limeos/limeosctl}"
CORE_SOCKET="${CORE_SOCKET:-/run/limeos-core/core.sock}"

# Every query (systemctl, docker, dpkg-query, ss, limeosctl) runs under this bound (seconds), so a
# hung systemd becomes a named failure instead of an indefinite stall.
QUERY_TIMEOUT="${CUTOVER_QUERY_TIMEOUT:-30}"
[[ "${QUERY_TIMEOUT}" =~ ^[1-9][0-9]*$ ]] || QUERY_TIMEOUT=30
# Every query has three answers: yes, no, and "unknown" (it timed out or gave no usable answer).
# The helpers below print the literal word "unknown" for the third, and every caller handles it:
# preflight refuses, a health check fails, return.sh takes the restoring action anyway, and
# measure.sh reports it as unknown.

fct_log() { printf '%s\n' "$*"; }
fct_warn() { printf 'WARN: %s\n' "$*" >&2; }
fct_die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

# No script stops with a non-zero status without saying what failed and where. set -e ends a
# script silently, and bash 5.2 (Debian 12) can end one without running its ERR trap at all
# (a failed command substitution of a bare redirection, guarded or not). So the ERR trap only
# records the failing command and its place (it also fires, harmlessly, inside command
# substitutions whose failure is handled), and the EXIT trap reports: that record, or, when no
# ERR trap ran, the command bash was running and the functions it was in. A deliberate exit
# (exit N, as fct_die does, after its own message) is not reported again. Installed below for
# every script that sources this library; a script that needs more at exit defines
# fct_on_exit STATUS, which runs after the report.
FCT_ERR_REPORT=''

# Sets FCT_CHAIN to ", called from FILE:LINE in FUNC" for each caller of the function its caller
# was called from (where the failure was), out to the script's top level.
fct_call_chain() {
    local -i i
    FCT_CHAIN=''
    for (( i = 2; i < ${#FUNCNAME[@]} - 1; i++ )); do
        FCT_CHAIN+=", called from ${BASH_SOURCE[i + 1]##*/}:${BASH_LINENO[i]} in ${FUNCNAME[i + 1]}"
    done
}

fct_on_err() { # status command
    [[ "$-" == *e* ]] || return 0
    fct_call_chain
    FCT_ERR_REPORT="'${2}' failed with status ${1} at ${BASH_SOURCE[1]##*/}:${BASH_LINENO[0]} in ${FUNCNAME[1]:-main}${FCT_CHAIN}"
}

fct_exit_trap() { # status command
    local status="${1}" command="${2}" what
    if (( status != 0 )) && [[ "${command}" != exit && "${command}" != 'exit '* ]]; then
        if [[ -n "${FCT_ERR_REPORT}" ]]; then
            what="${FCT_ERR_REPORT}"
        else
            # Bash names no line here: give the function the command was in and its callers.
            fct_call_chain
            what="'${command}' failed with status ${status} in ${FUNCNAME[1]:-main} (bash gives no line)${FCT_CHAIN}"
        fi
        printf 'ERROR: %s stopped unexpectedly (exit %s): %s\n' "${0##*/}" "${status}" "${what}" >&2 || true
    fi
    if declare -F fct_on_exit >/dev/null; then fct_on_exit "${status}"; fi
}
trap 'fct_on_err "$?" "${BASH_COMMAND}"' ERR
trap 'fct_exit_trap "$?" "${BASH_COMMAND}"' EXIT

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

# Empty when the kernel's cgroup.kill can end every process of control group CGROUP (a path under
# /sys/fs/cgroup, such as /system.slice/UNIT) at once, with no pid involved; otherwise why not. It
# needs the cgroup2 unified hierarchy and kernel 5.14 or newer (the root cgroup has no cgroup.kill).
fct_cgroup_kill_problem() { # cgroup
    local base="${CUTOVER_ROOT}/sys/fs/cgroup" dir
    dir="${base}${1}"
    if [[ "$(stat -f -c %T -- "${base}" 2>/dev/null || true)" != cgroup2fs \
            || "$(stat -f -c %T -- "${dir}" 2>/dev/null || true)" != cgroup2fs ]]; then
        printf '%s is not on a cgroup2 file system (the unified hierarchy)\n' "${dir}"
    elif [[ -L "${dir}" || ! -d "${dir}" ]]; then
        printf '%s is not a control group directory\n' "${dir}"
    elif [[ ! -e "${dir}/cgroup.kill" ]]; then
        printf '%s has no cgroup.kill (kernel before 5.14)\n' "${dir}"
    fi
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

# Succeeds when one unit declares StateDirectory=limeos (or limeos:ALIAS, systemd 250 and newer);
# fails with 1 when it does not, 2 when systemd did not answer. A unit that is not loaded is read
# from its unit file (systemctl show loads it), so a oneshot that last ran an hour ago still counts.
fct_unit_claims_state() { # unit
    local value rc=0 word
    local -a words=()
    value="$(fct_unit_property StateDirectory "${1}")" || rc=$?
    (( rc == 0 )) || return 2
    read -r -a words <<<"${value}"
    for word in "${words[@]}"; do
        if [[ "${word}" == limeos || "${word}" == limeos:* ]]; then return 0; fi
    done
    return 1
}

# The frozen services that declare StateDirectory=limeos, one per line; fails with 2 (printing
# nothing) when systemd did not answer for one of them.
fct_state_claimants() {
    local unit rc
    local -a found=()
    for unit in "${FROZEN_SERVICES[@]}"; do
        rc=0
        fct_unit_claims_state "${unit}" || rc=$?
        case "${rc}" in 0) found+=("${unit}") ;; 1) ;; *) return 2 ;; esac
    done
    if (( ${#found[@]} )); then printf '%s\n' "${found[@]}"; fi
}

# Every service unit on the host that declares StateDirectory=limeos, frozen or not, one per line:
# the unit files systemd knows (enabled or not, loaded or not) and the units it has loaded (a
# transient or generated unit has no file). Template files (NAME@.service) are skipped: only their
# instances can run, and a loaded instance is listed. Fails with 2 when systemd did not answer.
fct_all_state_claimants() {
    local listing loaded name rest rc
    local -A seen=()
    local -a found=()
    if ! listing="$(fct_query systemctl list-unit-files --type=service --no-legend)" || [[ -z "${listing}" ]]; then
        fct_warn 'no answer from: systemctl list-unit-files --type=service --no-legend'
        return 2
    fi
    if ! loaded="$(fct_query systemctl list-units --type=service --all --plain --no-legend)"; then
        fct_warn 'no answer from: systemctl list-units --type=service --all --plain --no-legend'
        return 2
    fi
    while read -r name rest; do
        [[ "${name}" == *.service && "${name}" != *@.service && -z "${seen["${name}"]:-}" ]] || continue
        seen["${name}"]=1
        rc=0
        fct_unit_claims_state "${name}" || rc=$?
        case "${rc}" in 0) found+=("${name}") ;; 1) ;; *) return 2 ;; esac
    done < <(printf '%s\n%s\n' "${listing}" "${loaded}")
    : "${rest:-}"
    if (( ${#found[@]} )); then printf '%s\n' "${found[@]}"; fi
}

# "PATH UID GID MODE" for a directory, "PATH absent" when there is none, or "PATH not-a-directory".
fct_dir_record() { # path
    local dir="${CUTOVER_ROOT}${1}" owner group mode
    if [[ ! -e "${dir}" && ! -L "${dir}" ]]; then printf '%s absent\n' "${1}"; return 0; fi
    if [[ -L "${dir}" || ! -d "${dir}" ]]; then printf '%s not-a-directory\n' "${1}"; return 0; fi
    read -r owner group mode < <(stat -c '%u %g %a' -- "${dir}")
    printf '%s %s %s %s\n' "${1}" "${owner}" "${group}" "${mode}"
}

# The guard drop-in of one frozen service, as a host path.
fct_guard_file() { printf '%s/etc/systemd/system/%s.d/%s\n' "${CUTOVER_ROOT}" "${1}" "${GUARD_NAME}"; }
# Every guard drop-in on the host, whatever unit it is on, a dangling symlink included (host paths).
fct_guard_files() {
    local file
    for file in "${CUTOVER_ROOT}"/etc/systemd/system/*.d/"${GUARD_NAME}"; do
        if [[ -e "${file}" || -L "${file}" ]]; then printf '%s\n' "${file}"; fi
    done
}

# Give every entry of TREE owned by another account to UID:GID, without following a symlink and
# without a path being resolved again after the walk: GNU chown -R walks with fts (FTS_PHYSICAL,
# FTS_CWDFD: each directory is opened relative to its parent's descriptor with O_NOFOLLOW and its
# device and inode checked against the walk's own lstat), and -h with --from changes each entry with
# fchownat(AT_SYMLINK_NOFOLLOW) relative to that descriptor, only when it is still owned by the
# account named in --from. So an account that owns entries inside TREE cannot redirect the change
# outside it by swapping a directory for a symlink, and an entry it gives away meanwhile is left as
# it is. The foreign owners are listed first (read-only); prints the chown commands it runs through
# RUNNER (fct_do in activate.sh). Fails when a listing or a chown fails.
fct_reclaim_tree() { # runner tree uid gid
    local runner="${1}" tree="${2}" uid="${3}" gid="${4}" owners owner
    [[ -d "${tree}" && ! -L "${tree}" ]] || return 0
    owners="$(find "${tree}" -xdev ! -uid "${uid}" -printf '%U\n' | sort -u)" || return 1
    while IFS= read -r owner; do
        [[ "${owner}" =~ ^[0-9]+$ ]] || continue
        "${runner}" chown -R -P -h --from="${owner}" -- "${uid}:${gid}" "${tree}" || return 1
    done <<<"${owners}"
}

# "UID GID" of an account from the passwd database; fails when there is no such account.
fct_account_ids() { # account
    local line
    local -a fields=()
    line="$(getent passwd "${1}" 2>/dev/null)" || return 1
    IFS=: read -r -a fields <<<"${line}"
    [[ "${fields[2]:-}" =~ ^[0-9]+$ && "${fields[3]:-}" =~ ^[0-9]+$ ]] || return 1
    printf '%s %s\n' "${fields[2]}" "${fields[3]}"
}

# The Caddy site template: the --site-template file, else the one next to the scripts (a copied
# directory, see bundle.sh), else the repository's packaging/ copy. Prints the first readable
# candidate, or the first candidate when none is (the caller reports it).
fct_default_template() {
    local candidate
    for candidate in "${SCRIPT_DIR}/Caddyfile.cutover.example" "${SCRIPT_DIR}/../../packaging/Caddyfile.cutover.example"; do
        if [[ -f "${candidate}" && -r "${candidate}" ]]; then printf '%s\n' "${candidate}"; return 0; fi
    done
    printf '%s\n' "${SCRIPT_DIR}/Caddyfile.cutover.example"
}
fct_template_problem() { # template
    [[ -f "${1}" && -r "${1}" ]] && return 0
    printf 'site template not readable: %s (copy packaging/Caddyfile.cutover.example next to the scripts, build the directory to copy with scripts/cutover/bundle.sh, or pass --site-template FILE)\n' "${1}"
}

# One line per problem with what activate.sh needs from the directory the scripts run from: its
# sibling scripts, and, in a directory made by bundle.sh, every file matching SHA256SUMS.
fct_layout_problems() {
    local script
    for script in lib.sh activate.sh return.sh; do
        [[ -f "${SCRIPT_DIR}/${script}" && -r "${SCRIPT_DIR}/${script}" ]] \
            || printf '%s is missing next to %s (copy the whole directory; scripts/cutover/bundle.sh builds it)\n' "${script}" "${SCRIPT_DIR}"
    done
    if [[ -f "${SCRIPT_DIR}/SHA256SUMS" ]] && ! (cd -- "${SCRIPT_DIR}" && sha256sum --quiet --strict -c SHA256SUMS >/dev/null 2>&1); then
        printf 'a file in %s does not match its SHA256SUMS (check: cd %s && sha256sum -c SHA256SUMS)\n' "${SCRIPT_DIR}" "${SCRIPT_DIR}"
    fi
    return 0
}

# Read-only preconditions of the proxy step, one line per problem.
fct_caddy_problems() { # caddyfile site_dir
    command -v caddy >/dev/null 2>&1 || printf 'caddy is not installed (install it separately first)\n'
    grep -Eqx "[[:space:]]*import[[:space:]]+${2}/\*(\.caddy)?" "${CUTOVER_ROOT}${1}" 2>/dev/null \
        || printf '%s must contain: import %s/*.caddy\n' "${1}" "${2}"
    return 0
}
fct_core_origin_ok() { tr -d ' \n' <"${CUTOVER_ROOT}/etc/limeos/core.json" 2>/dev/null | grep -qF "\"origin\":\"${1}\""; }
# The one limeos .deb to install, at VERSION; one line naming the problem, or nothing.
fct_package_problem() { # package version
    local deb_name deb_version
    if [[ "${1}" != *.deb ]]; then printf -- '--package must name one limeos .deb file: %s\n' "${1}"; return 0; fi
    if [[ ! -f "${CUTOVER_ROOT}${1}" ]]; then printf 'package not found: %s\n' "${1}"; return 0; fi
    deb_name="$(dpkg-deb -f "${CUTOVER_ROOT}${1}" Package 2>/dev/null)" || { printf 'cannot read the package fields of %s\n' "${1}"; return 0; }
    deb_version="$(dpkg-deb -f "${CUTOVER_ROOT}${1}" Version 2>/dev/null)" || { printf 'cannot read the package fields of %s\n' "${1}"; return 0; }
    if [[ "${deb_name}" != limeos ]]; then printf '%s is package %s, not limeos\n' "${1}" "${deb_name}"; return 0; fi
    if [[ "${deb_version}" != "${2}" ]]; then printf '%s is version %s, not %s\n' "${1}" "${deb_version}" "${2}"; fi
    return 0
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

# The state word systemd reports for a unit (is-active: active, inactive, failed, activating, ...;
# is-enabled: enabled, disabled, static, ...), or "unknown" when the query timed out or printed no
# single state word (systemctl that cannot reach systemd prints nothing). Always succeeds; an
# unknown answer is named in a warning. Older systemd versions print nothing for is-enabled on a
# unit file that does not exist (newer ones print not-found): then LoadState decides, and
# "not-found" is a definite answer.
fct_unit_query() { # is-active|is-enabled unit
    local out='' rc=0
    out="$(fct_query systemctl "${1}" "${2}")" || rc=$?
    if (( rc != 124 )) && [[ "${out}" =~ ^[a-z][a-z-]*$ && "${out}" != unknown ]]; then
        printf '%s\n' "${out}"
        return 0
    fi
    if (( rc != 124 )) && [[ "${1}" == is-enabled && -z "${out}" ]]; then
        rc=0
        out="$(fct_query systemctl show -p LoadState --value "${2}")" || rc=$?
        if (( rc == 0 )) && [[ "${out}" == not-found ]]; then printf 'not-found\n'; return 0; fi
    fi
    (( rc == 124 )) || fct_warn "no answer from: systemctl ${1} ${2}"
    printf 'unknown\n'
}
fct_unit_active() { fct_unit_query is-active "${1}"; }
fct_unit_enabled() { fct_unit_query is-enabled "${1}"; }

# One property of a unit (systemctl show -p PROP --value UNIT); fails with 2 when systemd did not
# answer. The value may be empty (ControlGroup of a unit that is not running).
fct_unit_property() { # property unit
    local out rc=0
    out="$(fct_query systemctl show -p "${1}" --value "${2}")" || rc=$?
    if (( rc != 0 )); then
        (( rc == 124 )) || fct_warn "no answer from: systemctl show -p ${1} --value ${2}"
        return 2
    fi
    printf '%s\n' "${out}"
}

# Every unit the limeos package ships (the standard profile this cutover installs; the shadow
# profile's units are named limeos-shadow-*). Their drop-ins are reported: an operator override (a
# User= line, say) changes what the packaged unit runs as, and is easy to miss.
SHIPPED_UNITS=("${RUST_UNITS[@]}" "${STACK_READER}" limeos-storage-ready.service limeos-storage-targets.service limeos-storage-mounts.service)
# One line per shipped unit with drop-ins, "UNIT PATH...", or "UNIT unknown" when systemd did not
# answer. Read-only.
fct_dropin_report() {
    local unit paths rc
    for unit in "${SHIPPED_UNITS[@]}"; do
        rc=0
        paths="$(fct_unit_property DropInPaths "${unit}")" || rc=$?
        if (( rc != 0 )); then printf '%s unknown\n' "${unit}"; continue; fi
        [[ -z "${paths//[[:space:]]/}" ]] || printf '%s %s\n' "${unit}" "${paths}"
    done
}

# limeos-* units, discovered (names only), from the list of every unit file: a host always has
# some, so an empty or failed listing means systemd did not answer (fails with 2), never "none".
fct_limeops_units() {
    local listing name rest
    if ! listing="$(fct_query systemctl list-unit-files --no-legend)" || [[ -z "${listing}" ]]; then
        fct_warn 'no answer from: systemctl list-unit-files --no-legend'
        return 2
    fi
    while read -r name rest; do
        if [[ "${name}" == limeops-* ]]; then printf '%s\n' "${name}"; fi
    done <<<"${listing}"
    return 0
}

# "<class> <unit> <active> <enabled>" for every unit we care about, sorted. A state systemd did not
# report is "unknown"; when the limeops-* units could not be listed, the line
# "untouched limeops-* unknown unknown" stands for them. Never a blank field.
fct_unit_report() {
    local unit limeops='' listed=0
    limeops="$(fct_limeops_units)" || listed=$?
    {
        for unit in "${FROZEN_UNITS[@]}" "${RUST_UNITS[@]}" caddy.service; do
            printf 'managed %s %s %s\n' "${unit}" "$(fct_unit_active "${unit}")" "$(fct_unit_enabled "${unit}")"
        done
        while read -r unit; do
            [[ -n "${unit}" ]] || continue
            printf 'untouched %s %s %s\n' "${unit}" "$(fct_unit_active "${unit}")" "$(fct_unit_enabled "${unit}")"
        done < <(printf '%s\n' "${UNTOUCHED_FIXED[@]}" "${limeops}")
        if (( listed )); then printf 'untouched limeops-* unknown unknown\n'; fi
    } | sort
}

# Empty when a snapshot's units.txt records a definite state for every unit and has a record for
# every managed unit; otherwise one line naming the first problem. A guessed state would decide
# what return.sh restarts, so a blank, unknown or missing one is refused like a missing hash.
fct_units_problem() { # file
    local file="${1}" line unit
    local -A seen=()
    if [[ ! -s "${file}" ]]; then printf 'units.txt is missing or empty\n'; return 0; fi
    if [[ -n "$(tail -c 1 -- "${file}")" ]]; then printf 'units.txt does not end in a newline (cut short?)\n'; return 0; fi
    while IFS= read -r line; do
        if [[ ! "${line}" =~ ^(managed|untouched)\ ([^ ]+)\ ([a-z][a-z-]*)\ ([a-z][a-z-]*)$ ]] \
                || [[ "${BASH_REMATCH[3]}" == unknown || "${BASH_REMATCH[4]}" == unknown ]]; then
            printf 'units.txt has a record with a missing or unknown state: "%s"\n' "${line}"
            return 0
        fi
        if [[ "${BASH_REMATCH[1]}" == managed ]]; then seen["${BASH_REMATCH[2]}"]=1; fi
    done <"${file}"
    for unit in "${FROZEN_UNITS[@]}" "${RUST_UNITS[@]}" caddy.service; do
        if [[ -z "${seen["${unit}"]:-}" ]]; then printf 'units.txt has no record for %s\n' "${unit}"; return 0; fi
    done
}

# Sets SNAP_ACTIVE and SNAP_ENABLED from a snapshot's record of a managed unit (fct_snapshot_check
# has checked that every managed unit has one, with definite states).
fct_snapshot_unit() { # snapshot_dir unit
    local states
    states="$(awk -v u="${2}" '$1 == "managed" && $2 == u { print $3, $4; exit }' "${1}/units.txt")"
    read -r SNAP_ACTIVE SNAP_ENABLED <<<"${states}"
    [[ -n "${SNAP_ACTIVE}" && -n "${SNAP_ENABLED}" ]] || fct_die "${1}/units.txt has no record for ${2}"
}

# The installed version of a package; "absent" when dpkg does not have it, "unknown" when
# dpkg-query could not answer (exit 2 or a timeout; exit 1 means not found).
fct_pkg_version() { # package
    local out rc=0
    # shellcheck disable=SC2016  # a dpkg-query format, expanded by dpkg-query
    out="$(fct_query dpkg-query -W -f='${Version}' "${1}")" || rc=$?
    case "${rc}" in
        0) printf '%s\n' "${out:-absent}" ;;
        1) printf 'absent\n' ;;
        *) (( rc == 124 )) || fct_warn "no answer from: dpkg-query -W ${1} (exit ${rc})"; printf 'unknown\n' ;;
    esac
}

# Prints TEXT followed by a newline when it is not empty, and nothing otherwise.
fct_lines() { if [[ -n "${1}" ]]; then printf '%s\n' "${1}"; fi; }

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

# State that services the cutover leaves running write (the limeops-* homes, files a schedule
# outside the frozen app rewrites): preflight's --live-list, copied into the snapshot as
# live-paths.txt. One absolute path per line; a path ending in / stands for everything under that
# directory, and a file entry also covers its SQLite companions (-wal, -shm, -journal). A comment
# line gives the reason for the entries after it. These paths are hashed and reported, never
# failed on: the frozen app's own state stays strict.
LIVE_ENTRIES=()
LIVE_REASONS=()
fct_load_live() { # file
    local line reason='written by a service the cutover leaves running'
    LIVE_ENTRIES=()
    LIVE_REASONS=()
    [[ -n "${1}" && -f "${1}" ]] || return 0
    while IFS= read -r line || [[ -n "${line}" ]]; do
        if [[ "${line}" =~ ^[[:space:]]*#[[:space:]]*(.*[^[:space:]])[[:space:]]*$ ]]; then reason="${BASH_REMATCH[1]}"; continue; fi
        [[ "${line}" =~ ^[[:space:]]*$ ]] && continue
        LIVE_ENTRIES+=("${line}")
        LIVE_REASONS+=("${reason}")
    done <"${1}"
}
# Succeeds when PATH is live, setting LIVE_REASON to why.
fct_live_match() { # path
    local i entry
    for (( i = 0; i < ${#LIVE_ENTRIES[@]}; i++ )); do
        entry="${LIVE_ENTRIES[i]}"
        if [[ "${entry}" == */ && "${1}" == "${entry}"* ]] \
                || [[ "${1}" == "${entry}" || "${1}" == "${entry}-wal" || "${1}" == "${entry}-shm" || "${1}" == "${entry}-journal" ]]; then
            LIVE_REASON="${LIVE_REASONS[i]}"
            return 0
        fi
    done
    return 1
}
# One line per problem with the live list against the state list (both loaded): every entry is
# absolute and exists (a / entry as a directory); none is a shared directory as a whole or the
# root; none covers a path of the state list, which is the frozen app's and stays strict.
fct_live_problems() {
    local entry path shared
    for entry in "${LIVE_ENTRIES[@]}"; do
        if [[ "${entry}" != /* ]]; then printf 'live list entry is not an absolute path: %s\n' "${entry}"; continue; fi
        for shared in / /etc/limeos/ /var/lib/limeos/ /etc/limeos /var/lib/limeos; do
            if [[ "${entry}" == "${shared}" ]]; then printf 'live list entry %s would exempt a whole shared directory; name the files or subdirectories the running service writes\n' "${entry}"; continue 2; fi
        done
        if [[ "${entry}" == */ ]]; then
            [[ -d "${CUTOVER_ROOT}${entry}" && ! -L "${CUTOVER_ROOT}${entry%/}" ]] || printf 'live list directory missing (or not a directory): %s\n' "${entry}"
        elif [[ ! -e "${CUTOVER_ROOT}${entry}" ]]; then
            printf 'live list path missing: %s\n' "${entry}"
        fi
    done
    while IFS= read -r path; do
        [[ -n "${path}" ]] || continue
        if fct_live_match "${path}"; then printf 'state list path %s is also covered by the live list (%s); a file is either the frozen app'"'"'s (strict) or a running service'"'"'s (reported only)\n' "${path}" "${LIVE_REASON}"; fi
    done < <(fct_state_files)
    return 0
}

# "<flag> <sha256|ABSENT>" for one logical path. V marks files a running writer
# may legitimately change (SQLite and its journals); L marks a path of the live list (written by a
# service the cutover leaves running: reported, never failed on); S is strict. Anything that exists
# but is not a regular file, or cannot be hashed, fails with a warning naming it, unless it is live.
fct_hash_one() {
    local path="${1}" flag=S out file="${CUTOVER_ROOT}${1}"
    case "${path}" in *.db|*.sqlite|*.sqlite3|*-wal|*-shm|*.db-journal|*.sqlite3-journal|*.sqlite-journal) flag=V ;; esac
    if (( ${#LIVE_ENTRIES[@]} )) && fct_live_match "${path}"; then flag=L; fi
    if [[ ! -e "${file}" && ! -L "${file}" ]]; then printf '%s ABSENT\n' "${flag}"; return 0; fi
    if [[ -L "${file}" || ! -f "${file}" ]]; then
        # A live file can vanish or be replaced between the listing and the hash (a SQLite journal).
        if [[ "${flag}" == L ]]; then printf 'L ABSENT\n'; return 0; fi
        fct_warn "state path is not a regular file: ${path}"
        return 1
    fi
    if ! out="$(sha256sum <"${file}")" || [[ ! "${out%% *}" =~ ^[0-9a-f]{64}$ ]]; then
        if [[ "${flag}" == L ]]; then printf 'L ABSENT\n'; return 0; fi
        fct_warn "cannot hash ${path}"
        return 1
    fi
    printf '%s %s\n' "${flag}" "${out%% *}"
}

# "<flag> <hash> <path>" lines, sorted; the path is every byte after the second space
# (read them back with fct_parse_record). Paths the new package owns are skipped. Fails
# when any path cannot be hashed (the group runs in a subshell, so exit leaves only it);
# with "tolerant", prints "! <path>" for such a path instead and carries on.
fct_state_hashes() { # strict|tolerant
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
    [[ "${REC_FLAG}" == [SVL] && "${REC_PATH}" == /* ]] || return 1
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

# The running containers ("ID NAME", sorted). Fails with 2, printing nothing, when docker did not
# answer (daemon down, a timeout): that is not an empty list.
fct_docker_ids() {
    local out
    out="$(fct_query docker ps --no-trunc --format '{{.ID}} {{.Names}}')" || { fct_warn 'no answer from: docker ps'; return 2; }
    fct_lines "${out}" | sort
}
# "ID /NAME STARTEDAT" of each running container, sorted; fails with 2 like fct_docker_ids.
fct_docker_started() {
    local out
    local -a ids=()
    out="$(fct_query docker ps --no-trunc --format '{{.ID}}')" || { fct_warn 'no answer from: docker ps'; return 2; }
    mapfile -t ids < <(fct_lines "${out}" | sort)
    if (( ${#ids[@]} == 0 )); then return 0; fi
    out="$(fct_query docker inspect --format '{{.Id}} {{.Name}} {{.State.StartedAt}}' "${ids[@]}")" \
        || { fct_warn 'no answer from: docker inspect'; return 2; }
    fct_lines "${out}" | sort
}

# Names of containers whose line differs between a snapshot file and the current
# listing (started, stopped, re-created or restarted), one per line.
fct_docker_diff_names() { # snapshot_file current_file
    comm -3 <(sort -- "${1}") <(sort -- "${2}") | awk '{ n = $2; sub(/^\//, "", n); print n }' | sort -u
}
# Fails with 2, printing nothing, when docker did not answer.
fct_docker_changes() { # snapshot_dir
    local ids started
    ids="$(fct_docker_ids)" || return 2
    started="$(fct_docker_started)" || return 2
    {
        fct_docker_diff_names "${1}/docker-ps.txt" <(fct_lines "${ids}")
        fct_docker_diff_names "${1}/docker-started.txt" <(fct_lines "${started}")
    } | sort -u
}
# Filter names on stdin by the snapshot's managed set (managed.txt, written by preflight).
# A snapshot from before managed sets has none: then every container counts as managed.
fct_managed_only() { if [[ -f "${1}/managed.txt" ]]; then grep -Fxf "${1}/managed.txt" || [[ $? == 1 ]]; else cat; fi; }
fct_unmanaged_only() { if [[ -f "${1}/managed.txt" ]]; then grep -vFxf "${1}/managed.txt" || [[ $? == 1 ]]; else cat >/dev/null; fi; }
# Only the managed set must be unchanged; a change elsewhere (watchtower updating an unmanaged
# container, say) is named and otherwise ignored.
fct_warn_unmanaged_changes() { # snapshot_dir
    local name changes
    if ! changes="$(fct_docker_changes "${1}")"; then
        fct_warn 'could not check the containers outside the managed set: docker did not answer'
        return 0
    fi
    while read -r name; do
        if [[ -n "${name}" ]]; then fct_warn "container outside the managed set changed (not rolled back): ${name}"; fi
    done < <(fct_lines "${changes}" | fct_unmanaged_only "${1}")
}

# Snapshot must be complete, from a passing preflight, record a definite state for every unit,
# and be fresh (max 0 = any age). ADVICE is what to do about a snapshot whose unit states cannot
# be used.
fct_snapshot_check() { # dir max_age_seconds [advice]
    local dir="${1}" max="${2}" meta ts age problem
    meta="${dir}/meta.env"
    [[ -f "${meta}" ]] || fct_die "not a preflight snapshot: ${dir}"
    [[ "$(fct_meta_get "${meta}" complete)" == 1 ]] || fct_die 'snapshot is incomplete'
    [[ "$(fct_meta_get "${meta}" preflight)" == pass ]] || fct_die 'snapshot records a refused preflight'
    problem="$(fct_units_problem "${dir}/units.txt")"
    [[ -z "${problem}" ]] || fct_die "the snapshot's ${problem}; nothing was changed. ${3:-Run preflight again.}"
    if (( max > 0 )); then
        ts="$(fct_meta_get "${meta}" epoch)"
        [[ "${ts}" =~ ^[0-9]+$ ]] || fct_die 'snapshot has no valid timestamp'
        age=$(( $(fct_now) - ts ))
        if (( age < 0 || age > max )); then fct_die "snapshot is stale (${age}s old, limit ${max}s); run preflight again"; fi
    fi
}
