#!/usr/bin/env bash
# The shell and Python suites whose code runs on the production OS, run on that OS: Debian 12's
# bash 5.2, mawk, coreutils, util-linux, procps and Python 3.11 instead of the CI runner's
# Ubuntu 24.04 (bash 5.3 hid a return.sh defect that only bash 5.2 shows). Runs inside a
# debian:12 container as root, installs what the suites need, then runs them as an unprivileged
# user (the stub harnesses model a non-root operator and a few cases differ under root).
#
#   docker run --rm -v "$PWD:/src" -w /src -e TEST_UID="$(id -u)" IMAGE bash scripts/debian12_suites.sh
#
# Covered: shellcheck over the cutover and maintainer scripts; tests/cutover (preflight,
# activate, return and measure against stubbed systemctl, docker, dpkg and a fake /proc);
# tests/repository/test_maintainer_scripts.py (postinst, prerm, postrm against a stubbed
# systemctl under DPKG_ROOT); tests/reference (the guest, reference-host and probe-build scripts,
# which run under Debian 12's Python). Left out, and why: docs/plans/2026-11-19-production-readiness-plan.md,
# Phase 4.
set -Eeuo pipefail

fct_die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

fct_main() {
    [[ "${EUID}" -eq 0 ]] || fct_die 'run as root inside the debian:12 container'
    grep -qx 'VERSION_ID="12"' /etc/os-release || fct_die 'not Debian 12'
    local uid="${TEST_UID:-1000}"
    [[ "${uid}" =~ ^[1-9][0-9]*$ ]] || fct_die "invalid TEST_UID: ${uid}"

    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    # python3 for the harnesses and measure.sh's report, procps for ps (the stubs and return.sh),
    # the linter, locales so the en_GB.UTF-8 collation case runs instead of being skipped, and git
    # for the probe-artifact tests (git init in a temp directory; no history is read).
    apt-get install -y -qq --no-install-recommends python3 procps shellcheck locales git >/dev/null
    sed -i 's/^# *en_GB.UTF-8 UTF-8/en_GB.UTF-8 UTF-8/' /etc/locale.gen
    locale-gen >/dev/null

    printf 'bash %s, %s, %s, shellcheck %s\n' "${BASH_VERSION}" "$(python3 --version)" \
        "$(dpkg-query -W -f='mawk ${Version}' mawk)" "$(shellcheck --version | awk '/^version:/ {print $2}')"

    useradd --uid "${uid}" --create-home --shell /bin/bash limeos-test 2>/dev/null \
        || id -u limeos-test >/dev/null || fct_die "could not create a test user with uid ${uid}"

    shellcheck scripts/cutover/*.sh scripts/debian12_suites.sh \
        packaging/debian/postinst packaging/debian/prerm packaging/debian/postrm
    # PYTHONDONTWRITEBYTECODE: nothing is written into the mounted checkout.
    runuser -u limeos-test -- env PYTHONDONTWRITEBYTECODE=1 HOME=/home/limeos-test bash -Eeuo pipefail -c '
        python3 -m unittest discover -s tests/cutover -p "test_*.py"
        python3 -m unittest discover -s tests/repository -p "test_maintainer_scripts.py"
        python3 -m unittest discover -s tests/reference -p "test_*.py"
    '
}

fct_main "$@"
