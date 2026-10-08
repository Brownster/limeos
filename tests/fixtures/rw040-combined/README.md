# RW-040 combined guest fixtures

These are the inputs for `tests/qualification/rw040-combined/`. Everything they
describe is created inside the owned disposable guest and destroyed with it.

| File | Purpose |
|---|---|
| `layout.json` | Guest disks (serials, filesystems, fixed UUIDs), directories, a bind alias, a symlink source, tmpfs, a named volume, a second UID, host tasks (nondumpable root, another UID, root without capabilities) and containers. Containers cover running, stopped, mountless, cross-UID, nondumpable, file-source, volume, tmpfs and swappable-device consumers, plus transition and unsupported-backing consumers. |
| `cases.json` | The brief's acceptance matrix: each case's membership, modes, required integrator prerequisites, the guest stages that prepare it, and the expected evidence |
| `packages-2fd7420.json` | Package and binary SHA-256 values for the exact-source amd64 packages of `2fd74209`, copied from the integrator's reviewed capture of CI run 37746766303 |
| `probe-contract.md` | A draft interface the integrator may adopt for its reserved probe; not an implementation |

Fixture notes:
- **Container image:** a local `docker import` of Debian's `busybox-static`
  binary. Nothing is pulled from a registry.
- **Nondumpable container:** it runs as UID 1990 and executes a setuid-root
  busybox copy. Secure exec leaves the task nondumpable, which the guest checks
  through its `/proc` ownership.
- **Namespace-switch container:** it alone runs without Docker's default
  AppArmor profile, because busybox `unshare` must mount. It is an observed
  target, not an observer; no observer's confinement changes.
- **Btrfs consumer:** it exists only during its transition, so it can't turn a
  supported collection into an unsupported one by accident.
- **XFS disk size:** 320 MiB, because current xfsprogs refuses XFS filesystems
  smaller than 300 MB.
