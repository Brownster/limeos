#!/usr/bin/env bash
# One stub for systemctl, systemd-run, docker, ss, free, curl, dpkg, dpkg-deb, dpkg-query, caddy, sleep, stat and
# limeosctl. sleep runs $STUB_DIR/hook-sleep when present (measure.sh's idle window). Copied under each tool's name into a PATH directory by the tests.
# State lives in $STUB_DIR; every call is appended to $STUB_DIR/calls.log. systemd-run --scope --unit and
# systemctl is-active/show/kill on a *.scope model a transient scope (see scope_members).
set -Eeuo pipefail
tool="$(basename -- "$0")"
call="${tool} $*"
printf '%s\n' "${call}" >>"${STUB_DIR}/calls.log"
# The same, prefixed with the transient scope the caller runs in ("-" for none).
printf '%s %s\n' "${STUB_SCOPE:--}" "${call}" >>"${STUB_DIR}/scoped-calls.log"
if [[ -f "${STUB_DIR}/fail.txt" ]] && grep -qxF -- "${call}" "${STUB_DIR}/fail.txt"; then
    printf 'stub: injected failure for: %s\n' "${call}" >&2
    exit 1
fi
# Glob patterns in silent.txt: a matching call prints nothing and exits 2, as a tool that could not
# answer does (systemctl that cannot reach systemd, a docker daemon that is down, a dpkg-query that
# cannot read its database).
if [[ -f "${STUB_DIR}/silent.txt" ]]; then
    while IFS= read -r pattern; do
        # shellcheck disable=SC2053
        if [[ -n "${pattern}" && "${call}" == ${pattern} ]]; then exit 2; fi
    done <"${STUB_DIR}/silent.txt"
fi
# Glob patterns in hang.txt: a matching call hangs for 90 s first (a stuck systemd query).
if [[ -f "${STUB_DIR}/hang.txt" ]]; then
    while IFS= read -r pattern; do
        # shellcheck disable=SC2053
        if [[ -n "${pattern}" && "${call}" == ${pattern} ]]; then command -p sleep 90; break; fi
    done <"${STUB_DIR}/hang.txt"
fi
# "SIGNAL pattern" lines in signal.txt: send SIGNAL to the calling script when the call matches
# the glob pattern, then carry on with the call. "group:SIGNAL" sends it to the caller's whole
# process group instead, as a terminal's Ctrl-C does; a stub in that group dies of it. A call the
# script bounds with timeout(1) has timeout as its parent: the script is the one above it.
if [[ -f "${STUB_DIR}/signal.txt" ]]; then
    caller="${PPID}"
    if [[ "$(ps -o comm= -p "${caller}" 2>/dev/null || true)" == timeout ]]; then
        caller="$(ps -o ppid= -p "${caller}" | tr -d ' ')"
    fi
    while read -r sig pattern; do
        # shellcheck disable=SC2053
        [[ "${call}" == ${pattern} ]] || continue
        if [[ "${sig}" == group:* ]]; then
            kill -s "${sig#group:}" -- "-$(ps -o pgid= -p "${caller}" | tr -d ' ')"
        else
            kill -s "${sig}" "${caller}"
        fi
    done <"${STUB_DIR}/signal.txt"
fi
unit_dir="${STUB_DIR}/units"
set_unit() { printf '%s\n' "${3}" >"${unit_dir}/${1}.${2}"; }
get_unit() { if [[ -f "${unit_dir}/${1}.${2}" ]]; then cat "${unit_dir}/${1}.${2}"; else printf '%s\n' "${3}"; fi; }

# Transient scopes. A process belongs to scope U when its environment carries STUB_SCOPE=U: the
# stub systemd-run sets it before it execs the command, and it is inherited the way cgroup
# membership is (through fork, exec and setsid alike; a parent's death does not lose it). A
# member whose environment also carries STUB_UNKILLABLE=1 survives the stub's systemctl kill,
# standing in for a process in uninterruptible I/O. "killable" lists only the others.
scope_members() { # unit [killable]
    python3 -I - "${STUB_DIR}" "${1}" "${2:-}" <<'PY'
import os, sys
stub, unit, killable = sys.argv[1:4]
mine, want = b"STUB_DIR=" + os.fsencode(stub), b"STUB_SCOPE=" + os.fsencode(unit)
for entry in os.listdir("/proc"):
    if not entry.isdigit() or int(entry) in (os.getpid(), os.getppid()):
        continue
    try:
        with open(f"/proc/{entry}/environ", "rb") as handle:
            env = handle.read().split(b"\0")
        with open(f"/proc/{entry}/stat") as handle:
            state = handle.read().rsplit(") ", 1)[1].split()[0]
    except (OSError, IndexError):
        continue
    if state == "Z" or mine not in env or want not in env:
        continue
    if killable and b"STUB_UNKILLABLE=1" in env:
        continue
    print(entry)
PY
}
# SIGKILL every member that can die, again until none is left (what the kernel's cgroup kill
# and systemd's cgroup walk both achieve).
scope_kill() { # unit signal
    local round
    local -a pids=()
    for (( round=0; round<50; round++ )); do
        mapfile -t pids < <(scope_members "${1}" killable)
        (( ${#pids[@]} )) || break
        kill -s "${2}" "${pids[@]}" 2>/dev/null || true
    done
}
# The scope's cgroup under the fake root, as systemd keeps it: cgroup.procs and cgroup.events
# while it has members (plus any pids in $STUB_DIR/cgroup-extra, which stand for members that
# exit while a caller reads the list), and no directory once it is empty. cgroup.kill is a
# plain file here: whatever was written to it takes effect at the next look at the scope (the
# caller's wait for it to empty), logged in $STUB_DIR/cgroup-kills.log. Flags in $STUB_DIR:
# no-cgroup-kill (a kernel before 5.14), cgroup-kill-readonly (the write fails).
scope_refresh() { # unit -> members on stdout
    local members dir="${LIMEOS_CUTOVER_ROOT}/sys/fs/cgroup/system.slice/${1}"
    if [[ -s "${dir}/cgroup.kill" ]]; then
        : >"${dir}/cgroup.kill"
        printf '%s\n' "${1}" >>"${STUB_DIR}/cgroup-kills.log"
        scope_kill "${1}" KILL
    fi
    members="$(scope_members "${1}")"
    if [[ -n "${members}" ]]; then
        mkdir -p "${dir}"
        { printf '%s\n' "${members}"; if [[ -f "${STUB_DIR}/cgroup-extra" ]]; then cat "${STUB_DIR}/cgroup-extra"; fi; } >"${dir}/cgroup.procs"
        printf 'populated 1\nfrozen 0\n' >"${dir}/cgroup.events"
        if [[ -e "${STUB_DIR}/no-cgroup-kill" ]]; then
            rm -f "${dir}/cgroup.kill"
        else
            [[ -e "${dir}/cgroup.kill" ]] || : >"${dir}/cgroup.kill"
            if [[ -e "${STUB_DIR}/cgroup-kill-readonly" ]]; then chmod 0444 "${dir}/cgroup.kill"; fi
        fi
        printf '%s\n' "${members}"
    else
        rm -rf "${dir}"
    fi
}

case "${tool}" in
    systemd-run)
        # --scope --unit=U: the command runs in place, as the real --scope does, and it and
        # everything it starts become members of U. A unit that still has members is refused.
        unit=''
        while (( $# )) && [[ "${1}" != -- ]]; do
            case "${1}" in --unit=*) unit="${1#--unit=}" ;; --unit) unit="${2}"; shift ;; esac
            shift
        done
        shift
        if [[ -n "${unit}" ]]; then
            if [[ -n "$(scope_members "${unit}")" ]]; then
                printf 'Failed to start transient scope unit: Unit %s already exists.\n' "${unit}" >&2
                exit 1
            fi
            printf '%s\n' "${unit}" >>"${STUB_DIR}/scopes.log"
            export STUB_SCOPE="${unit}"
        fi
        exec "$@" ;;
    sleep) if [[ -x "${STUB_DIR}/hook-sleep" ]]; then "${STUB_DIR}/hook-sleep"; fi; exit 0 ;;
    stat)
        # stat -f -c %T: the fake cgroup tree is a cgroup2 mount unless $STUB_DIR/cgroupfs-tmpfs
        # exists. Anything else is the real stat.
        if [[ "${1:-}" == -f ]]; then
            if [[ -e "${STUB_DIR}/cgroupfs-tmpfs" ]]; then echo tmpfs; else echo cgroup2fs; fi
            exit 0
        fi
        command -p stat "$@"; exit ;;
    systemctl)
        verb="${1:-}"; shift || true
        quiet=0; args=()
        for a in "$@"; do if [[ "${a}" == --quiet ]]; then quiet=1; else args+=("${a}"); fi; done
        if (( ${#args[@]} )) && [[ "${args[-1]}" == *.scope ]]; then
            scope="${args[-1]}"
            case "${verb}" in
                is-active)
                    if [[ -n "$(scope_refresh "${scope}")" ]]; then state=active; else state=inactive; fi
                    (( quiet )) || printf '%s\n' "${state}"
                    [[ "${state}" == active ]] ;;
                show)
                    if [[ -n "$(scope_refresh "${scope}")" ]]; then printf '/system.slice/%s\n' "${scope}"; else printf '\n'; fi ;;
                kill)
                    # --kill-whom=all --signal=SIG: every member, again until none that can die is left.
                    sig=TERM
                    for a in "${args[@]}"; do if [[ "${a}" == --signal=* ]]; then sig="${a#--signal=}"; fi; done
                    if [[ -z "$(scope_members "${scope}")" ]]; then
                        printf 'Failed to kill unit %s: Unit %s not loaded.\n' "${scope}" "${scope}" >&2
                        exit 1
                    fi
                    scope_kill "${scope}" "${sig}"
                    scope_refresh "${scope}" >/dev/null ;;
                *) printf 'stub systemctl: unsupported %s for a scope\n' "${verb}" >&2; exit 99 ;;
            esac
            exit
        fi
        case "${verb}" in
            is-active)
                state="$(get_unit "${args[0]}" active inactive)"
                (( quiet )) || printf '%s\n' "${state}"
                [[ "${state}" == active ]] ;;
            is-enabled)
                # Older systemd prints nothing (and fails) for a unit file that does not exist; units
                # whose $unit.loadstate says not-found, and that have no .enabled file, do that.
                if [[ "$(get_unit "${args[0]}" loadstate loaded)" == not-found && ! -f "${unit_dir}/${args[0]}.enabled" ]]; then
                    printf 'Failed to get unit file state for %s: No such file or directory\n' "${args[0]}" >&2
                    exit 1
                fi
                state="$(get_unit "${args[0]}" enabled disabled)"
                printf '%s\n' "${state}"
                [[ "${state}" == enabled ]] ;;
            list-unit-files)
                # Every unit file the stub knows (a real host always has some).
                for f in "${unit_dir}"/*.enabled; do
                    [[ -e "${f}" ]] || continue
                    b="$(basename -- "${f}" .enabled)"; printf '%s %s enabled\n' "${b}" "$(get_unit "${b}" enabled x)"
                done ;;
            stop)
                for u in "${args[@]}"; do
                    set_unit "${u}" active inactive
                    if [[ -x "${STUB_DIR}/hook-stop-${u}" ]]; then "${STUB_DIR}/hook-stop-${u}"; fi
                    if [[ "${u}" == pi-health.service ]]; then
                        set_unit pihealth-helper.service active inactive
                        grep -v ':8002 ' "${STUB_DIR}/ss_out" >"${STUB_DIR}/ss_out.new" || true
                        mv "${STUB_DIR}/ss_out.new" "${STUB_DIR}/ss_out"
                    fi
                    if [[ "${u}" == caddy.service ]]; then grep -v ':8002 ' "${STUB_DIR}/ss_out" >"${STUB_DIR}/ss_out.new" || true; mv "${STUB_DIR}/ss_out.new" "${STUB_DIR}/ss_out"; fi
                done ;;
            start|restart)
                for u in "${args[@]}"; do
                    set_unit "${u}" active active
                    if [[ -x "${STUB_DIR}/hook-start-${u}" ]]; then "${STUB_DIR}/hook-start-${u}"; fi
                    if [[ "${u}" == pi-health.service ]]; then printf 'LISTEN 0 128 0.0.0.0:8002 0.0.0.0:*\n' >>"${STUB_DIR}/ss_out"; fi
                done ;;
            reload-or-restart)
                # Starts a stopped unit, reloads a running one.
                for u in "${args[@]}"; do set_unit "${u}" active active; done ;;
            try-reload-or-restart) ;;  # reloads a running unit, leaves a stopped one alone
            enable) for u in "${args[@]}"; do set_unit "${u}" enabled enabled; done ;;
            disable)
                for u in "${args[@]}"; do
                    set_unit "${u}" enabled disabled
                    if [[ -x "${STUB_DIR}/hook-disable-${u}" ]]; then "${STUB_DIR}/hook-disable-${u}"; fi
                done ;;
            show)
                prop=''
                for (( i=0; i<${#args[@]}; i++ )); do if [[ "${args[i]}" == -p ]]; then prop="${args[i+1]}"; fi; done
                case "${prop}" in
                    MainPID) printf '%s\n' "$(get_unit "${args[-1]}" mainpid 0)" ;;
                    ControlGroup) printf '%s\n' "$(get_unit "${args[-1]}" cgroup '')" ;;
                    LoadState) printf '%s\n' "$(get_unit "${args[-1]}" loadstate loaded)" ;;
                    *) printf '%s\n' "$(get_unit "${args[-1]}" restarts 0)" ;;
                esac ;;
            daemon-reload|reload|reset-failed) ;;
            *) printf 'stub systemctl: unsupported %s\n' "${verb}" >&2; exit 99 ;;
        esac ;;
    docker)
        verb="${1:-}"; shift || true
        case "${verb}" in
            ps)
                if [[ "$*" == *'{{.ID}} {{.Names}}'* ]]; then cat "${STUB_DIR}/docker_ps.txt"
                elif [[ "$*" == *'{{.Names}}'* ]]; then awk '{print $2}' "${STUB_DIR}/docker_ps.txt"
                else awk '{print $1}' "${STUB_DIR}/docker_ps.txt"; fi ;;
            inspect) cat "${STUB_DIR}/docker_started.txt" ;;
            *) printf 'stub docker: mutating or unknown verb %s\n' "${verb}" >&2; exit 99 ;;
        esac ;;
    ss) cat "${STUB_DIR}/ss_out" ;;
    free) cat "${STUB_DIR}/free_out" ;;
    dpkg)
        [[ " $* " == *" -i "* ]] || exit 99
        printf '%s\n' "${DEBIAN_FRONTEND:-}" >"${STUB_DIR}/dpkg_frontend"
        printf '%s\n' "$(cat "${STUB_DIR}/pkg_to_install")" >"${STUB_DIR}/pkg_version"
        for u in limeos-storaged limeos-containerd limeos-core; do set_unit "${u}.service" active active; set_unit "${u}.service" enabled enabled; done ;;
    dpkg-deb)
        # dpkg-deb -f FILE FIELD: the package file's control fields.
        [[ "${1:-}" == -f && -n "${3:-}" ]] || exit 99
        case "${3}" in
            Package) if [[ -f "${STUB_DIR}/deb_package" ]]; then cat "${STUB_DIR}/deb_package"; else echo limeos; fi ;;
            Version) if [[ -f "${STUB_DIR}/deb_version" ]]; then cat "${STUB_DIR}/deb_version"; else cat "${STUB_DIR}/pkg_to_install"; fi ;;
            *) exit 99 ;;
        esac ;;
    dpkg-query)
        pkg="${!#}"
        if [[ "${1:-}" == -L ]]; then [[ -f "${STUB_DIR}/pkg_files.${pkg}" ]] || exit 1; cat "${STUB_DIR}/pkg_files.${pkg}"; exit 0; fi
        if [[ "$*" == *Installed-Size* ]]; then [[ -f "${STUB_DIR}/pkg_info.${pkg}" ]] || exit 1; cat "${STUB_DIR}/pkg_info.${pkg}"; exit 0; fi
        [[ -f "${STUB_DIR}/pkg_version" ]] || exit 1
        cat "${STUB_DIR}/pkg_version" ;;
    caddy) ;;
    limeosctl) ;;
    curl)
        url=''; out=''; want_code=0; fail_flag=0; fmt=''
        while (( $# )); do
            case "$1" in
                -o) out="$2"; shift 2 ;;
                -w) want_code=1; fmt="$2"; shift 2 ;;
                -H) if [[ "$2" == @- ]]; then cat >>"${STUB_DIR}/curl_headers"; else printf '%s\n' "$2" >>"${STUB_DIR}/curl_headers"; fi; shift 2 ;;
                --resolve|--max-time|-b) shift 2 ;;
                --*) shift ;;
                -*) if [[ "$1" == *f* ]]; then fail_flag=1; fi; shift ;;
                *) url="$1"; shift ;;
            esac
        done
        code=000; body=''
        while read -r pattern c b; do
            # shellcheck disable=SC2254
            case "${url}" in ${pattern}) code="${c}"; body="${b:-}"; break ;; esac
        done <"${STUB_DIR}/curl.map"
        if [[ -n "${out}" && "${out}" != /dev/null ]]; then printf '%s' "${body}" >"${out}"; fi
        if (( want_code )) && [[ "${fmt}" == *time_total* ]]; then
            # "<code> <time_total> <time_appconnect> <time_connect>"; times from $STUB_DIR/curl_times.
            times='0.004 0.002 0.001'
            if [[ -f "${STUB_DIR}/curl_times" ]]; then times="$(cat "${STUB_DIR}/curl_times")"; fi
            printf '%s %s' "${code}" "${times}"
        elif (( want_code )); then printf '%s' "${code}"; fi
        # curl_exit: fail after the headers and body, as a transfer cut short does.
        if [[ -f "${STUB_DIR}/curl_exit" ]]; then exit "$(cat "${STUB_DIR}/curl_exit")"; fi
        if [[ "${code}" == 000 ]]; then exit 7; fi
        if (( fail_flag )) && (( code >= 400 )); then exit 22; fi ;;
    *) exit 99 ;;
esac
