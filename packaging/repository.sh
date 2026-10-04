#!/usr/bin/env bash
# Usage: repository.sh DIRECTORY SIGNING_KEY_FINGERPRINT DEB...
# Requires an existing operator-owned key; never generates a production key.
set -Eeuo pipefail
fct_on_error() { printf '%s\n' 'Repository signing failed; do not publish the incomplete repository.' >&2; }
trap fct_on_error ERR
fct_main() {
    if (( $# < 3 )); then printf '%s\n' 'Usage: repository.sh DIRECTORY SIGNING_KEY_FINGERPRINT DEB...' >&2; return 2; fi
    local repository="${1}" key="${2}" package architecture
    shift 2
    [[ "${key}" =~ ^[A-Fa-f0-9]{40,64}$ ]] || return 2
    mkdir -p "${repository}/pool/main" "${repository}/dists/stable/main/binary-amd64" "${repository}/dists/stable/main/binary-arm64"
    for package in "$@"; do cp -- "${package}" "${repository}/pool/main/"; done
    cd "${repository}"
    for architecture in amd64 arm64; do
        apt-ftparchive -a "${architecture}" packages pool/main >"dists/stable/main/binary-${architecture}/Packages"
        gzip -n -k -f "dists/stable/main/binary-${architecture}/Packages"
    done
    apt-ftparchive -o APT::FTPArchive::Release::Origin=LimeOS -o APT::FTPArchive::Release::Label=LimeOS \
        -o APT::FTPArchive::Release::Suite=stable -o APT::FTPArchive::Release::Codename=stable \
        -o APT::FTPArchive::Release::Architectures='amd64 arm64' -o APT::FTPArchive::Release::Components=main \
        release dists/stable >dists/stable/Release
    printf 'Valid-Until: %s\n' "$(date -u -d '+7 days' '+%a, %d %b %Y %H:%M:%S UTC')" >>dists/stable/Release
    gpg --batch --yes --local-user "${key}" --clearsign --output dists/stable/InRelease dists/stable/Release
    gpg --batch --yes --local-user "${key}" --detach-sign --armor --output dists/stable/Release.gpg dists/stable/Release
    gpg --batch --export "${key}" >limeos-archive-keyring.gpg
}
fct_main "$@"
