#!/usr/bin/env bash
# Build the directory an operator copies to the host: the cutover scripts, the state-list helper,
# the Caddy site template and the runbook, flat, with a SHA256SUMS that preflight.sh checks there.
# Run from a checkout; changes nothing outside OUT_DIR.
set -Eeuo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "${SCRIPT_DIR}/lib.sh"
REPO="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"
FILES=(
    "scripts/cutover/lib.sh"
    "scripts/cutover/preflight.sh"
    "scripts/cutover/activate.sh"
    "scripts/cutover/return.sh"
    "scripts/cutover/measure.sh"
    "scripts/cutover/state_lists.py"
    "packaging/Caddyfile.cutover.example"
    "docs/p06-cutover-runbook.md"
)

fct_usage() {
    cat <<'USAGE'
Usage: bundle.sh OUT_DIR

Copies the cutover scripts (lib.sh, preflight.sh, activate.sh, return.sh,
measure.sh), state_lists.py, Caddyfile.cutover.example and the runbook into
OUT_DIR (created; it must not exist or be empty) and writes OUT_DIR/SHA256SUMS.
In that directory activate.sh finds the site template next to itself, and
preflight.sh refuses if any file no longer matches SHA256SUMS. Copy the whole
directory to the host (the runbook uses /root/limeos-cutover) and check it there:
  cd /root/limeos-cutover && sha256sum -c SHA256SUMS
USAGE
}

fct_main() {
    local out="${1:-}" file
    case "${out}" in
        -h|--help) fct_usage; return 0 ;;
        '') fct_usage >&2; fct_die 'an output directory is required' ;;
    esac
    (( $# == 1 )) || { fct_usage >&2; fct_die 'only one output directory is accepted'; }
    if [[ -e "${out}" && -n "$(ls -A -- "${out}" 2>/dev/null)" ]]; then fct_die "${out} exists and is not empty"; fi
    mkdir -p -- "${out}"
    for file in "${FILES[@]}"; do
        [[ -f "${REPO}/${file}" ]] || fct_die "${file} is missing from the checkout"
        install -m "$([[ "${file}" == *.sh ]] && echo 0755 || echo 0644)" -- "${REPO}/${file}" "${out}/${file##*/}"
    done
    (cd -- "${out}" && sha256sum -- "${FILES[@]##*/}" >SHA256SUMS)
    printf 'bundle: %s (%s files and SHA256SUMS)\n' "${out}" "${#FILES[@]}"
    printf 'copy the directory to the host, then check it there: cd /root/limeos-cutover && sha256sum -c SHA256SUMS\n'
}

fct_main "$@"
