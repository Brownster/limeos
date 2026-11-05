#!/usr/bin/env bash
# T5: stop the frozen writers, install the pinned Rust package, start the Rust
# services behind an HTTPS Caddy site, health-check, and roll back (return.sh)
# automatically on any failed step. Never touches limeops-*, limeos-agent,
# flask-shutdown, docker or containers. Needs explicit operator approval.
set -Eeuo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
# shellcheck source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

MAX_SNAPSHOT_AGE=1800
SITE_NAME=limeos-cutover.caddy
ARMED=0
SNAP=''
SITE_DIR='/etc/caddy/conf.d'

fct_usage() {
    cat <<'USAGE'
Usage: activate.sh SNAPSHOT_DIR --yes-i-have-operator-approval
                   --package PATH --expected-version VERSION --host HOST
                   [--port 8002] [--tls internal | --tls-cert FILE --tls-key FILE]
                   [--site-dir /etc/caddy/conf.d] [--caddyfile /etc/caddy/Caddyfile]
                   [--template FILE] [--dry-run] [--help]

PATH is a limeos .deb or a directory of .deb files. HOST is the name or IPv4
address browsers use (core.json "origin" must be https://HOST:PORT). The default
TLS mode is internal (Caddy local CA). --dry-run prints every command and changes
nothing. Any failed step after the first change runs return.sh and exits 1.
USAGE
}

fct_abort() {
    printf 'FAIL: %s\n' "$*" >&2
    if (( ARMED )); then
        printf 'ROLLBACK: running return.sh\n' >&2
        "${SCRIPT_DIR}/return.sh" "${SNAP}" --site-dir "${SITE_DIR}" >&2 || printf 'ROLLBACK INCOMPLETE: follow docs/p06-cutover-runbook.md\n' >&2
    fi
    exit 1
}

# Mutating step: from the first one on, any failure rolls back.
fct_do() {
    ARMED=1
    fct_run "$@" || fct_abort "command failed: $*"
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
    for (( attempt=0; attempt<30; attempt++ )); do
        if "$@" >/dev/null 2>&1; then printf 'ok: %s\n' "${label}"; return 0; fi
        sleep 1
    done
    fct_abort "health check failed: ${label}"
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
fct_units_active() { local unit; for unit in "$@"; do systemctl is-active --quiet "${unit}" || return 1; done; }
fct_docker_matches() {
    [[ "$(fct_docker_ids)" == "$(cat -- "${SNAP}/docker-ps.txt")" ]] \
        && [[ "$(fct_docker_started)" == "$(cat -- "${SNAP}/docker-started.txt")" ]]
}
fct_untouched_match() {
    [[ "$(fct_unit_report | grep '^untouched ')" == "$(grep '^untouched ' "${SNAP}/units.txt")" ]]
}

fct_main() {
    local approved=0 package='' version='' host='' port="${FROZEN_PORT_DEFAULT}" tls=internal cert='' key=''
    local caddyfile='/etc/caddy/Caddyfile' template="${SCRIPT_DIR}/../../packaging/Caddyfile.cutover.example"
    while (( $# )); do
        case "${1}" in
            --yes-i-have-operator-approval) approved=1; shift ;;
            --dry-run) DRY_RUN=1; shift ;;
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
    fct_need "package not found: ${package}" test -e "${CUTOVER_ROOT}${package}"
    fct_need 'caddy is not installed (install it separately first)' command -v caddy
    fct_need "${caddyfile} must contain: import ${SITE_DIR}/*.caddy" grep -Eqx "[[:space:]]*import[[:space:]]+${SITE_DIR}/\*(\.caddy)?" "${CUTOVER_ROOT}${caddyfile}"
    fct_need "core.json origin must be ${site}" fct_core_origin_ok "${site}"
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

    local -a debs=()
    if [[ -d "${CUTOVER_ROOT}${package}" ]]; then
        mapfile -t debs < <(find "${CUTOVER_ROOT}${package}" -maxdepth 1 -name '*.deb' | sort)
        (( ${#debs[@]} > 0 )) || { [[ ${DRY_RUN} == 1 ]] || fct_die "no .deb files in ${package}"; }
        debs=("${debs[@]#"${CUTOVER_ROOT}"}")
    else
        debs=("${package}")
    fi

    fct_log "activate: frozen writers stop, Rust services start, HTTPS at ${site}"
    local unit
    # 1. Stop and disable only the frozen writers (pihealth-helper is PartOf pi-health).
    fct_do systemctl stop limeos-metrics-collector.timer limeos-package-reconcile.timer
    fct_do systemctl stop pi-health.service
    fct_do systemctl stop pihealth-helper.service
    for unit in "${FROZEN_UNITS[@]}"; do fct_do systemctl disable "${unit}"; done
    if (( ! DRY_RUN )); then
        fct_state_hashes >"${SNAP}/state.poststop.sha256"
        local flag hash path
        while read -r flag hash path; do
            [[ "${flag}" == S ]] || continue
            grep -qxF -- "S ${hash} ${path}" "${SNAP}/state.poststop.sha256" || fct_abort "state file changed before the cutover: ${path}"
        done <"${SNAP}/state.sha256"
    else
        printf '[dry-run] hash state files again and compare strict entries with the snapshot\n'
    fi
    if (( ! DRY_RUN )) && ! fct_port_free "${port}"; then fct_abort "port ${port} is still in use after the frozen app stopped"; fi

    # 2. Install the pinned package; its postinst starts the services too.
    fct_do dpkg -i "${debs[@]}"
    if (( DRY_RUN )); then printf '[dry-run] check installed limeos version is %s\n' "${version}"
    elif ! fct_installed_version_ok "${version}"; then fct_abort "installed limeos version is not ${version}"; fi
    fct_do systemctl daemon-reload
    for unit in "${RUST_UNITS[@]}"; do fct_do systemctl enable "${unit}"; done
    for unit in "${RUST_UNITS[@]}"; do fct_do systemctl start "${unit}"; done
    fct_retry 'Rust services active' fct_units_active "${RUST_UNITS[@]}"

    # 3. HTTPS proxy on the public port, now that the frozen app has released it.
    fct_do install -d -m 0755 -- "${CUTOVER_ROOT}${SITE_DIR}"
    if (( DRY_RUN )); then
        printf '[dry-run] install -m 0644 <rendered site> %s%s/%s\n' "${CUTOVER_ROOT}" "${SITE_DIR}" "${SITE_NAME}"
    else
        ARMED=1
        install -m 0644 -- "${rendered}" "${CUTOVER_ROOT}${SITE_DIR}/${SITE_NAME}" || fct_abort 'cannot install the Caddy site'
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
    fct_retry 'docker ps and StartedAt identical to the snapshot' fct_docker_matches
    fct_retry 'untouched units unchanged' fct_untouched_match
    fct_log "activate complete: ${site}"
}

fct_main "$@"
