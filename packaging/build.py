#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"Build a root-owned Debian package from locked release binaries and built UI."

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def profile_text(text: str, shadow: bool) -> str:
    if not shadow:
        return text
    for prefix in ["/usr/lib/limeos", "/var/lib/limeos", "/etc/limeos", "/run/limeos-"]:
        text = text.replace(prefix, prefix.replace("limeos", "limeos-shadow"))
    # Account/service names, preserving executable basenames following '/'.
    text = re.sub(
        r"(?<!/)\blimeos-(core|containerd|storaged|assistant|rpc|host-access|container-access)\b",
        r"limeos-shadow-\1",
        text,
    )
    text = text.replace("dpkg --configure limeos.", "dpkg --configure limeos-shadow.")
    text = text.replace("StateDirectory=limeos/", "StateDirectory=limeos-shadow/")
    return text


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=["amd64", "arm64"], required=True)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--version", default="0.1.0")
    parser.add_argument("--profile", choices=["standard", "shadow"], default="standard")
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    if not args.version or not all(c.isalnum() or c in ".+-~" for c in args.version):
        parser.error("invalid Debian version")
    args.output.mkdir(parents=True, exist_ok=True)
    shadow = args.profile == "shadow"
    package = "limeos-shadow" if shadow else "limeos"
    with tempfile.TemporaryDirectory(prefix="limeos-package-") as temporary:
        stage = Path(temporary)
        lib = stage / "usr/lib" / package
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
            name = (
                unit.name.replace("limeos-", "limeos-shadow-") if shadow else unit.name
            )
            content = profile_text(unit.read_text(), shadow)
            if shadow and unit.name == "limeos-core.service":
                content = content.replace(
                    "ExecStart=/usr/lib/limeos-shadow/limeos-core",
                    "ExecStart=/usr/lib/limeos-shadow/limeos-core --state-dir /var/lib/limeos-shadow/core --config /etc/limeos-shadow/core.json --socket /run/limeos-shadow-core/core.sock",
                )
            if shadow and unit.name == "limeos-containerd.service":
                content = content.replace(
                    "/executors/container\n", "/executors/container read-only\n", 1
                )
            (units / name).write_text(content)
        etc = stage / "etc" / package
        etc.mkdir(parents=True)
        config = json.loads((ROOT / "packaging/config/core.json").read_text())
        if shadow:
            config.update(
                listen="127.0.0.1:8004",
                origin="https://limeos-shadow.localhost:8444",
                host_socket="/run/limeos-shadow-storaged/executor.sock",
                container_socket="/run/limeos-shadow-containerd/executor.sock",
            )
        (etc / "core.json").write_text(json.dumps(config, indent=2) + "\n")
        documentation = stage / "usr/share/doc" / package
        documentation.mkdir(parents=True)
        caddy = profile_text((ROOT / "packaging/Caddyfile.example").read_text(), shadow)
        if shadow:
            caddy = caddy.replace(
                "https://localhost {", "https://limeos-shadow.localhost:8444 {"
            ).replace("127.0.0.1:8003", "127.0.0.1:8004")
        (documentation / "Caddyfile.example").write_text(caddy)
        shutil.copyfile(
            ROOT / "docs/p01-operations.md", documentation / "p01-operations.md"
        )
        shutil.copyfile(
            ROOT / "docs/p02-operations.md", documentation / "p02-operations.md"
        )
        shutil.copyfile(
            ROOT / "docs/p03-operations.md", documentation / "p03-operations.md"
        )
        shutil.copyfile(
            ROOT / "docs/p03-compose-planning.md",
            documentation / "p03-compose-planning.md",
        )
        control = stage / "DEBIAN"
        control.mkdir()
        (control / "control").write_text(f"""Package: {package}
Version: {args.version}
Architecture: {args.arch}
Maintainer: LimeOS maintainers <maintainers@limeos.invalid>
Depends: libc6 (>= 2.36), libgcc-s1, adduser, bash, systemd, util-linux
Section: admin
Priority: optional
Description: Secure host observations and approved container operations
 Rust identity, durable authority, shared observations and bounded executors.
""")
        (control / "conffiles").write_text(f"/etc/{package}/core.json\n")
        for name in ["postinst", "prerm", "postrm"]:
            (control / name).write_text(
                profile_text((ROOT / "packaging/debian" / name).read_text(), shadow)
            )
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
                str(args.output / f"{package}_{args.version}_{args.arch}.deb"),
            ],
            check=True,
        )


if __name__ == "__main__":
    main()
