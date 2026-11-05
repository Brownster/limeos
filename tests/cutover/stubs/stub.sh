#!/usr/bin/env bash
# One stub for systemctl, docker, ss, free, curl, dpkg, dpkg-query, caddy, sleep and
# limeosctl. Copied under each tool's name into a PATH directory by the tests.
# State lives in $STUB_DIR; every call is appended to $STUB_DIR/calls.log.
set -Eeuo pipefail
tool="$(basename -- "$0")"
call="${tool} $*"
printf '%s\n' "${call}" >>"${STUB_DIR}/calls.log"
if [[ -f "${STUB_DIR}/fail.txt" ]] && grep -qxF -- "${call}" "${STUB_DIR}/fail.txt"; then
    printf 'stub: injected failure for: %s\n' "${call}" >&2
    exit 1
fi
unit_dir="${STUB_DIR}/units"
set_unit() { printf '%s\n' "${3}" >"${unit_dir}/${1}.${2}"; }
get_unit() { if [[ -f "${unit_dir}/${1}.${2}" ]]; then cat "${unit_dir}/${1}.${2}"; else printf '%s\n' "${3}"; fi; }

case "${tool}" in
    sleep) exit 0 ;;
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
                    if [[ "${u}" == pi-health.service ]]; then printf 'LISTEN 0 128 0.0.0.0:8002 0.0.0.0:*\n' >>"${STUB_DIR}/ss_out"; fi
                done ;;
            enable) for u in "${args[@]}"; do set_unit "${u}" enabled enabled; done ;;
            disable) for u in "${args[@]}"; do set_unit "${u}" enabled disabled; done ;;
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
        [[ "${1:-}" == -i ]] || exit 99
        printf '%s\n' "$(cat "${STUB_DIR}/pkg_to_install")" >"${STUB_DIR}/pkg_version"
        for u in limeos-storaged limeos-containerd limeos-core; do set_unit "${u}.service" active active; set_unit "${u}.service" enabled enabled; done ;;
    dpkg-query)
        [[ -f "${STUB_DIR}/pkg_version" ]] || exit 1
        cat "${STUB_DIR}/pkg_version" ;;
    caddy) ;;
    limeosctl) ;;
    curl)
        url=''; out=''; want_code=0; fail_flag=0
        while (( $# )); do
            case "$1" in
                -o) out="$2"; shift 2 ;;
                -w) want_code=1; shift 2 ;;
                --resolve|--max-time) shift 2 ;;
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
        (( want_code )) && printf '%s' "${code}"
        if [[ "${code}" == 000 ]]; then exit 7; fi
        if (( fail_flag )) && (( code >= 400 )); then exit 22; fi ;;
    *) exit 99 ;;
esac
