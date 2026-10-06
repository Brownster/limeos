#!/usr/bin/env bash
# Regenerate the GNU tar backup-archive fixtures from synthetic trees.
#
# Each archive is produced with the frozen helper's own compression flags
# (`tar -czf` and `tar -I zstd -cf`, zstd's default level) and fixed metadata,
# so the bytes are reproducible and the zstd window matches real backups.
# Contents are synthetic: no credentials, databases or real .env values.
#
# Usage: tests/fixtures/backup-archives/generate.sh   (writes into this directory)
set -Eeuo pipefail

fct_main() {
  local out work
  out="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  work="$(mktemp -d)"
  # Expand now: the trap runs after this function's locals are gone.
  # shellcheck disable=SC2064
  trap "rm -rf '${work}'" EXIT

  local -a tar_opts=(
    --format=gnu --sort=name --mtime='2026-01-01 00:00:00Z'
    --owner=0 --group=0 --numeric-owner --mode='go-w'
  )

  fct_tree "${work}/root"

  # Valid: the plugin-archive layout and a primary layout without overlapping
  # sources, in both formats. Members carry no leading "/", exactly as GNU tar
  # stored the frozen helper's absolute sources.
  tar "${tar_opts[@]}" -C "${work}/root" -czf "${out}/legacy-valid.tar.gz" \
    etc/limeos home/pi/docker opt/stacks var/lib/limeos
  tar "${tar_opts[@]}" -C "${work}/root" -I zstd \
    -cf "${out}/legacy-valid.tar.zst" etc/limeos home/pi/docker opt/stacks var/lib/limeos

  # Valid: POSIX/pax format, which adds informational pax records.
  tar "${tar_opts[@]/--format=gnu/--format=posix}" \
    --pax-option='delete=atime,delete=ctime' -C "${work}/root" \
    -czf "${out}/legacy-posix.tar.gz" etc/limeos opt/stacks

  # Rejected today: the frozen helper listed /etc/limeos and also files inside
  # it, so GNU tar stores the second copy as a hard link to itself.
  tar "${tar_opts[@]}" -C "${work}/root" -I zstd \
    -cf "${out}/legacy-primary-overlap.tar.zst" \
    etc/limeos etc/limeos/media_layout.json etc/limeos/credentials.env opt/stacks

  # Rejected: unsupported member types created by the real tool.
  local evil="${work}/evil"
  mkdir -p "${evil}/opt/stacks/media"
  ln -s ../../../etc/shadow "${evil}/opt/stacks/media/escape"
  tar "${tar_opts[@]}" -C "${evil}" -czf "${out}/symlink-escape.tar.gz" opt/stacks
  rm "${evil}/opt/stacks/media/escape"
  mkfifo "${evil}/opt/stacks/media/pipe"
  tar "${tar_opts[@]}" -C "${evil}" -czf "${out}/fifo.tar.gz" opt/stacks
  rm "${evil}/opt/stacks/media/pipe"
  truncate -s 8M "${evil}/opt/stacks/media/sparse.img"
  tar "${tar_opts[@]}" -S -C "${evil}" -czf "${out}/sparse.tar.gz" opt/stacks
  rm "${evil}/opt/stacks/media/sparse.img"

  # Rejected: absolute and parent-relative member names (GNU tar keeps them
  # only with -P, which a hostile archive author can use).
  echo '{}' > "${evil}/opt/stacks/media/x.json"
  tar "${tar_opts[@]}" -P --transform='s,^,/,' -C "${evil}" \
    -czf "${out}/absolute-name.tar.gz" opt/stacks/media/x.json
  tar "${tar_opts[@]}" -P --transform='s,^,../../,' -C "${evil}" \
    -czf "${out}/parent-name.tar.gz" opt/stacks/media/x.json

  # Rejected: a small zstd archive that declares a 64 MiB file of zeros.
  local bomb="${work}/bomb"
  mkdir -p "${bomb}/opt/stacks"
  head -c 67108864 /dev/zero > "${bomb}/opt/stacks/zeros.bin"
  tar "${tar_opts[@]}" -C "${bomb}" -I zstd \
    -cf "${out}/zeros-64m.tar.zst" opt/stacks

  (cd "${out}" && sha256sum ./*.tar.* > SHA256SUMS)
  tar --version | head -1 > "${out}/TOOLS"
  zstd --version >> "${out}/TOOLS"
  gzip --version | head -1 >> "${out}/TOOLS"
}

fct_tree() {
  local root="$1" long
  mkdir -p "${root}/etc/limeos/storage_plugins" "${root}/home/pi/docker/sonarr" \
    "${root}/opt/stacks/media" "${root}/var/lib/limeos/storage_plugins"
  printf '{"version":1,"listen":"127.0.0.1:8003"}\n' > "${root}/etc/limeos/core.json"
  printf '{"profile":"single_disk"}\n' > "${root}/etc/limeos/media_layout.json"
  printf 'PIHEALTH_USER=synthetic\n' > "${root}/etc/limeos/credentials.env"
  printf '{"pools":[]}\n' > "${root}/etc/limeos/storage_plugins/mergerfs.json"
  printf '<Config><Port>8989</Port></Config>\n' > "${root}/home/pi/docker/sonarr/config.xml"
  printf 'services:\n  sonarr:\n    image: example/sonarr@sha256:%064d\n' 0 \
    > "${root}/opt/stacks/media/compose.yaml"
  printf '{"last_sync":null}\n' > "${root}/var/lib/limeos/storage_plugins/snapraid.json"
  # A member name longer than 100 bytes needs a GNU long-name record.
  long="${root}/opt/stacks/media/$(printf 'nested-directory-%02d/' 1 2 3 4 5)"
  mkdir -p "${long}"
  printf 'long path\n' > "${long}/settings-with-a-deliberately-long-file-name.json"
}

fct_main "$@"
