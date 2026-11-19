#!/usr/bin/env bash
# One stub for systemctl, systemd-run, docker, ss, free, curl, dpkg, dpkg-deb, dpkg-query, caddy, sleep and
# limeosctl. sleep runs $STUB_DIR/hook-sleep when present (measure.sh's idle window). Copied under each tool's name into a PATH directory by the tests.
# State lives in $STUB_DIR; every call is appended to $STUB_DIR/calls.log.
set -Eeuo pipefail
tool="$(basename -- "$0")"
call="${tool} $*"
printf '%s\n' "${call}" >>"${STUB_DIR}/calls.log"
if [[ -f "${STUB_DIR}/fail.txt" ]] && grep -qxF -- "${call}" "${STUB_DIR}/fail.txt"; then
    printf 'stub: injected failure for: %s\n' "${call}" >&2
    exit 1
fi
# "SIGNAL pattern" lines in signal.txt: send SIGNAL to the calling script when the call matches
# the glob pattern, then carry on with the call.
if [[ -f "${STUB_DIR}/signal.txt" ]]; then
    while read -r sig pattern; do
        # shellcheck disable=SC2053
        if [[ "${call}" == ${pattern} ]]; then kill -s "${sig}" "${PPID}"; fi
    done <"${STUB_DIR}/signal.txt"
fi
unit_dir="${STUB_DIR}/units"
set_unit() { printf '%s\n' "${3}" >"${unit_dir}/${1}.${2}"; }
get_unit() { if [[ -f "${unit_dir}/${1}.${2}" ]]; then cat "${unit_dir}/${1}.${2}"; else printf '%s\n' "${3}"; fi; }

case "${tool}" in
    systemd-run)
        # Run the command in place, as the real --scope does, minus the transient unit.
        while (( $# )) && [[ "${1}" != -- ]]; do shift; done
        shift
        exec "$@" ;;
    sleep) if [[ -x "${STUB_DIR}/hook-sleep" ]]; then "${STUB_DIR}/hook-sleep"; fi; exit 0 ;;
    systemctl)
        verb="${1:-}"; shift || true
        quiet=0; args=()
        for a in "$@"; do if [[ "${a}" == --quiet ]]; then quiet=1; else args+=("${a}"); fi; done
        case "${verb}" in
            is-active)
                state="$(get_unit "${args[0]}" active inactive)"
                (( quiet )) || printf '%s\n' "${state}"
                [[ "${state}" == active ]] ;;
            is-enabled)
                state="$(get_unit "${args[0]}" enabled disabled)"
                printf '%s\n' "${state}"
                [[ "${state}" == enabled ]] ;;
            list-unit-files)
                for f in "${unit_dir}"/limeops-*.enabled; do
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
                    *) printf '%s\n' "$(get_unit "${args[-1]}" restarts 0)" ;;
                esac ;;
            daemon-reload|reload) ;;
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
        if [[ "${code}" == 000 ]]; then exit 7; fi
        if (( fail_flag )) && (( code >= 400 )); then exit 22; fi ;;
    *) exit 99 ;;
esac
