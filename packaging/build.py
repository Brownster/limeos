#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"Build a root-owned Debian package from locked release binaries and built UI."

import argparse
import hashlib
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=["amd64", "arm64"], required=True)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--version", default="0.1.0")
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    if not args.version or not all(c.isalnum() or c in ".+-~" for c in args.version):
        parser.error("invalid Debian version")
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="limeos-package-") as temporary:
        stage = Path(temporary)
        lib = stage / "usr/lib/limeos"
        lib.mkdir(parents=True)
        for name in [
            "limeos-core",
            "limeos-executor",
            "limeosctl",
            "limeos-password-worker",
        ]:
            shutil.copyfile(args.binaries / name, lib / name)
            (lib / name).chmod(0o755)
        if not (ROOT / "frontend/dist/index.html").is_file():
            raise SystemExit("Build the frontend before packaging.")
        shutil.copytree(ROOT / "frontend/dist", lib / "ui")
        shutil.copytree(ROOT / "contracts/generated", lib / "contracts")
        units = stage / "lib/systemd/system"
        units.mkdir(parents=True)
        for unit in (ROOT / "packaging/systemd").glob("*.service"):
            shutil.copyfile(unit, units / unit.name)
        etc = stage / "etc/limeos"
        etc.mkdir(parents=True)
        shutil.copyfile(ROOT / "packaging/config/core.json", etc / "core.json")
        documentation = stage / "usr/share/doc/limeos"
        documentation.mkdir(parents=True)
        shutil.copyfile(
            ROOT / "packaging/Caddyfile.example", documentation / "Caddyfile.example"
        )
        shutil.copyfile(
            ROOT / "docs/p01-operations.md", documentation / "p01-operations.md"
        )
        control = stage / "DEBIAN"
        control.mkdir()
        (control / "control").write_text(f"""Package: limeos
Version: {args.version}
Architecture: {args.arch}
Maintainer: LimeOS maintainers <maintainers@limeos.invalid>
Depends: libc6 (>= 2.36), libgcc-s1, adduser, bash, systemd
Section: admin
Priority: optional
Description: Secure local host-management foundation
 Rust identity, policy, durable state and bounded executor services.
""")
        (control / "conffiles").write_text("/etc/limeos/core.json\n")
        for name in ["postinst", "prerm", "postrm"]:
            shutil.copyfile(ROOT / "packaging/debian" / name, control / name)
            (control / name).chmod(0o755)
        for path in stage.rglob("*"):
            if path.is_symlink():
                raise SystemExit("Package payload must not contain symlinks.")
            if path.is_dir():
                path.chmod(0o755)
            elif path.name not in [
                "limeos-core",
                "limeos-executor",
                "limeosctl",
                "limeos-password-worker",
                "postinst",
                "prerm",
                "postrm",
            ]:
                path.chmod(0o644)
        checksums = []
        for path in sorted(stage.rglob("*")):
            relative = path.relative_to(stage)
            if path.is_file() and relative.parts[0] != "DEBIAN":
                with path.open("rb") as stream:
                    digest = hashlib.file_digest(stream, "md5").hexdigest()
                checksums.append(f"{digest}  {relative}")
        (control / "md5sums").write_text("\n".join(checksums) + "\n")
        subprocess.run(
            [
                "dpkg-deb",
                "--root-owner-group",
                "--build",
                str(stage),
                str(args.output / f"limeos_{args.version}_{args.arch}.deb"),
            ],
            check=True,
        )


if __name__ == "__main__":
    main()
