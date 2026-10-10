"""Local regressions for the RW-040 guest harness; no guest, Docker or root needed.

python3 -m unittest discover -s tests/qualification/rw040-combined -p 'test_*.py'
"""

import importlib.util
import io
import json
import os
import re
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
FIXTURES = ROOT / "tests/fixtures/rw040-combined"
BASE = "2fd74209e1238c1374837ab431ef670020726dc2"


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


runner = load("rw040_run_guest", HERE / "run_guest.py")
sys.path.insert(0, str(HERE / "guest"))
guest = load("rw040_combined_guest", HERE / "guest/combined_guest.py")
adapter = guest.probe_adapter
LAYOUT = json.loads((FIXTURES / "layout.json").read_text())
CASES = json.loads((FIXTURES / "cases.json").read_text())


def alive(pid: int) -> bool:
    try:
        state = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
    except OSError:
        return False
    return state != "Z"


class GuestTransportTests(unittest.TestCase):
    def test_qemu_has_no_forwarding_shares_or_network_consoles(self):
        with tempfile.TemporaryDirectory() as tmp:
            argv = runner.qemu_argv(Path(tmp), "rw040-qual-test", LAYOUT, 1536, 2)
        joined = " ".join(argv)
        for token in (
            "hostfwd",
            "guestfwd",
            "-virtfs",
            "-fsdev",
            "smb=",
            "tcp:",
            "telnet:",
            "ssh",
        ):
            self.assertNotIn(token, joined)
        self.assertIn("-nodefaults", argv)
        self.assertIn("-no-reboot", argv)
        self.assertIn("rw040-qual-test,process=rw040-qual-test", argv)
        self.assertTrue(
            any(
                a.startswith("unix:") and a.endswith("qmp.sock,server=on,wait=off")
                for a in argv
            )
        )
        self.assertTrue(any("id=inputs,readonly=on" in a for a in argv))
        for disk in LAYOUT["disks"]:
            self.assertIn(f"serial={disk['serial']}", joined)

    def test_forwarding_and_shares_are_refused(self):
        for bad in (
            ["-netdev", "user,id=n,hostfwd=tcp::2222-:22"],
            ["-virtfs", "local,path=/"],
            ["-chardev", "socket,id=c,port=4444"],
            ["-serial", "telnet:127.0.0.1:4444"],
        ):
            with self.assertRaises(SystemExit, msg=bad):
                runner.check_argv(["qemu-system-x86_64", *bad])

    def test_seed_has_no_credentials_and_runs_one_program_then_powers_off(self):
        text = runner.user_data()
        self.assertNotIn("ssh_authorized_keys", text)
        self.assertNotIn("passwd", text)
        self.assertIn("ssh_pwauth: false", text)
        self.assertIn("[systemctl, mask, --now, ssh.service, ssh.socket]", text)
        self.assertIn("/dev/disk/by-id/virtio-rw040-inputs", text)
        self.assertIn("--results /dev/disk/by-id/virtio-rw040-results", text)
        self.assertTrue(text.rstrip().endswith("- [systemctl, poweroff]"))


class InputVerificationTests(unittest.TestCase):
    def test_packages_and_image_must_match_their_records(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / "a.deb").write_bytes(b"package")
            good = {
                "packages": [
                    {"file": "a.deb", "sha256": runner.sha256(directory / "a.deb")}
                ]
            }
            self.assertEqual(
                runner.verify_packages(directory, good)[0]["file"], "a.deb"
            )
            bad = {"packages": [{"file": "a.deb", "sha256": "0" * 64}]}
            with self.assertRaises(SystemExit):
                runner.verify_packages(directory, bad)
            image = directory / "image.qcow2"
            image.write_bytes(b"image")
            sums = directory / "SHA512SUMS"
            sums.write_text(f"{runner.sha512(image)}  image.qcow2\n")
            self.assertEqual(runner.verify_image(image, sums), runner.sha512(image))
            sums.write_text(f"{'0' * 128}  image.qcow2\n")
            with self.assertRaises(SystemExit):
                runner.verify_image(image, sums)
            sums.write_text("")
            with self.assertRaises(SystemExit):
                runner.verify_image(image, sums)

    def test_inputs_disk_is_a_deterministic_root_owned_tar(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / "a.deb").write_bytes(b"package")
            manifest = {"packages": [{"file": "a.deb", "sha256": "x"}], "run_id": "t"}
            runner.inputs_tar(directory / "one.tar", manifest, directory)
            runner.inputs_tar(directory / "two.tar", manifest, directory)
            self.assertEqual(
                runner.sha256(directory / "one.tar"),
                runner.sha256(directory / "two.tar"),
            )
            with tarfile.open(directory / "one.tar") as tar:
                members = {m.name: m for m in tar}
                self.assertEqual(
                    json.loads(tar.extractfile("manifest.json").read()), manifest
                )
            for name in [
                "guest/combined_guest.py",
                "guest/env_probe.py",
                "guest/selfcheck_probe.py",
                "fixtures/layout.json",
                "fixtures/cases.json",
                "packages/a.deb",
            ]:
                self.assertIn(name, members)
                self.assertEqual((members[name].uid, members[name].mtime), (0, 0))
            self.assertEqual(members["guest/combined_guest.py"].mode, 0o755)
            self.assertEqual(os.path.getsize(directory / "one.tar") % 512, 0)


def fake_deb(path: Path, files: dict[str, bytes], compress: str) -> None:
    import lzma

    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode="w") as tar:
        for name, body in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(body)
            tar.addfile(info, io.BytesIO(body))
    payload = data.getvalue()
    member = "data.tar"
    if compress == "xz":
        payload, member = lzma.compress(payload), "data.tar.xz"

    def ar_member(name: str, body: bytes) -> bytes:
        header = (
            f"{name:<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(body):<10}`\n".encode()
        )
        return header + body + (b"\n" if len(body) % 2 else b"")

    path.write_bytes(
        b"!<arch>\n" + ar_member("debian-binary", b"2.0\n") + ar_member(member, payload)
    )


class PackageDerivationTests(unittest.TestCase):
    def test_binary_hashes_come_from_the_package_payload(self):
        for compress in ("none", "xz"):
            with tempfile.TemporaryDirectory() as tmp, self.subTest(compress):
                directory = Path(tmp)
                fake_deb(
                    directory / "limeos_9_amd64.deb",
                    {
                        "./usr/lib/limeos/limeos-executor": b"executor",
                        "./usr/lib/limeos/frontend/index.html": b"<html>",
                        "./etc/limeos/core.json": b"{}",
                    },
                    compress,
                )
                manifest = runner.derive_package_manifest(directory, BASE)
                (package,) = manifest["packages"]
                self.assertEqual(manifest["tested_source"], BASE)
                self.assertEqual(
                    package["binaries"],
                    {"limeos-executor": runner.hashlib.sha256(b"executor").hexdigest()},
                )
                self.assertEqual(
                    package["sha256"], runner.sha256(directory / "limeos_9_amd64.deb")
                )

    def test_non_debian_input_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "bad.deb").write_bytes(b"not an archive")
            with self.assertRaises(ValueError):
                runner.derive_package_manifest(Path(tmp), BASE)


def results_image(
    directory: Path, members: list[tuple[tarfile.TarInfo, bytes]]
) -> Path:
    path = directory / "results.img"
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.USTAR_FORMAT) as tar:
        for info, data in members:
            tar.addfile(info, io.BytesIO(data) if info.isfile() else None)
    path.write_bytes(buffer.getvalue() + bytes(1 << 16))
    return path


def file_member(name: str, data: bytes) -> tuple[tarfile.TarInfo, bytes]:
    info = tarfile.TarInfo(name)
    info.size = len(data)
    return info, data


def done_bytes():
    return json.dumps(
        {
            "run_id": "unit",
            "stages": {key: "passed" for key in runner.EXPECTED_STAGES},
            "seconds": 1,
        }
    ).encode()


class ResultExtractionTests(unittest.TestCase):
    def test_valid_results_are_copied(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            image = results_image(
                directory,
                [
                    file_member("done.json", done_bytes()),
                    file_member("ground-truth/engine.json", b"{}"),
                ],
            )
            names = runner.extract_results(image, directory / "out", "unit")
            self.assertEqual(sorted(names), ["done.json", "ground-truth/engine.json"])
            self.assertEqual(
                (directory / "out/ground-truth/engine.json").read_bytes(), b"{}"
            )

    def test_unsafe_or_incomplete_results_are_refused(self):
        link = tarfile.TarInfo("escape")
        link.type, link.linkname = tarfile.SYMTYPE, "/etc/passwd"
        big = tarfile.TarInfo("big.json")
        big.size = runner.MAX_RESULT_MEMBER + 1
        cases = {
            "parent": [file_member("../x", b""), file_member("done.json", b"{}")],
            "absolute": [file_member("/tmp/x", b""), file_member("done.json", b"{}")],
            "symlink": [(link, b""), file_member("done.json", b"{}")],
            "missing done": [file_member("platform.json", b"{}")],
            "odd name": [file_member("a b.json", b""), file_member("done.json", b"{}")],
        }
        for label, members in cases.items():
            with tempfile.TemporaryDirectory() as tmp, self.subTest(label):
                image = results_image(Path(tmp), members)
                with self.assertRaises(ValueError):
                    runner.extract_results(image, Path(tmp) / "out", "unit")
                self.assertFalse((Path(tmp) / "x").exists())
                self.assertFalse((Path(tmp) / "out").exists())
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "results.img"
            buffer = io.BytesIO()
            with tarfile.open(fileobj=buffer, mode="w") as tar:
                tar.addfile(big, io.BytesIO(bytes(big.size)))
            path.write_bytes(buffer.getvalue())
            with self.assertRaises(ValueError):
                runner.extract_results(path, Path(tmp) / "out", "unit")

    def test_extensions_are_rejected_before_payload_read(self):
        for kind in (tarfile.XHDTYPE, tarfile.GNUTYPE_LONGNAME, tarfile.GNUTYPE_SPARSE):
            with tempfile.TemporaryDirectory() as tmp, self.subTest(kind=kind):
                info = tarfile.TarInfo("extension")
                info.type, info.size = kind, 1 << 30
                path = Path(tmp) / "bad.img"
                path.write_bytes(info.tobuf(format=tarfile.USTAR_FORMAT))
                with self.assertRaisesRegex(ValueError, "extensions"):
                    runner.extract_results(path, Path(tmp) / "out", "unit")
                self.assertFalse((Path(tmp) / "out").exists())

    def test_duplicate_and_zero_member_count_are_bounded(self):
        for members in (
            [
                file_member("done.json", done_bytes()),
                file_member("done.json", done_bytes()),
            ],
            [
                file_member(f"zero-{i}", b"")
                for i in range(runner.MAX_RESULT_MEMBERS + 1)
            ],
        ):
            with tempfile.TemporaryDirectory() as tmp:
                image = results_image(Path(tmp), members)
                with self.assertRaises(ValueError):
                    runner.extract_results(image, Path(tmp) / "out", "unit")
                self.assertFalse((Path(tmp) / "out").exists())
                self.assertFalse(list(Path(tmp).glob(".rw040-results-*")))

    def test_payload_padding_end_and_trailing_data_must_be_complete(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            good = results_image(
                root, [file_member("done.json", done_bytes())]
            ).read_bytes()
            payload_end = 512 + len(done_bytes())
            variants = [good[: payload_end - 1], good[: 1024 + 512], good[:2048] + b"x"]
            padding = bytearray(good)
            padding[payload_end] = 1
            variants.append(bytes(padding))
            for index, data in enumerate(variants):
                with self.subTest(index=index):
                    path = root / f"bad-{index}.img"
                    path.write_bytes(data)
                    with self.assertRaises(ValueError):
                        runner.extract_results(path, root / f"out-{index}", "unit")
                    self.assertFalse((root / f"out-{index}").exists())

    def test_done_record_binds_exact_run_and_stage_contract(self):
        good = json.loads(done_bytes())
        for bad in (
            {**good, "run_id": "other"},
            {**good, "stages": {}},
            {**good, "stages": {"invented": "passed"}},
            {**good, "stages": {**good["stages"], "platform": "maybe"}},
        ):
            with tempfile.TemporaryDirectory() as tmp:
                image = results_image(
                    Path(tmp), [file_member("done.json", json.dumps(bad).encode())]
                )
                with self.assertRaises(ValueError):
                    runner.extract_results(image, Path(tmp) / "out", "unit")
                self.assertFalse((Path(tmp) / "out").exists())

    def test_result_destination_is_fresh(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = results_image(root, [file_member("done.json", done_bytes())])
            (root / "out").mkdir()
            with self.assertRaisesRegex(ValueError, "fresh"):
                runner.extract_results(image, root / "out", "unit")


HELPER = """
import importlib.util, sys, time
spec = importlib.util.spec_from_file_location("r", sys.argv[1])
r = importlib.util.module_from_spec(spec); spec.loader.exec_module(r)
from pathlib import Path
child = r.launch(["sleep", "60"], Path(sys.argv[2]))
print(child.pid, flush=True)
time.sleep(60)
"""


class OwnershipTests(unittest.TestCase):
    def test_parent_death_launcher_refuses_privileged_exec_images(self):
        with tempfile.TemporaryDirectory() as tmp:
            for mode in (0o4755, 0o2755):
                executable = Path(tmp) / "privileged"
                executable.write_text("owned test")
                executable.chmod(mode)
                with self.assertRaisesRegex(ValueError, "privileged executable"):
                    runner.launch([str(executable)], Path(tmp) / "log")
            with (
                mock.patch.object(
                    runner.os, "getxattr", return_value=b"file-capability"
                ),
                self.assertRaisesRegex(ValueError, "file capabilities"),
            ):
                runner.launch(["sleep", "1"], Path(tmp) / "log")

    def test_owned_child_dies_with_the_harness(self):
        with tempfile.TemporaryDirectory() as tmp:
            helper = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    HELPER,
                    str(HERE / "run_guest.py"),
                    str(Path(tmp) / "log"),
                ],
                stdout=subprocess.PIPE,
                text=True,
            )
            child = int(helper.stdout.readline())
            helper.stdout.close()
            self.assertTrue(alive(child))
            helper.kill()
            helper.wait()
            deadline = time.monotonic() + 5
            while alive(child) and time.monotonic() < deadline:
                time.sleep(0.05)
            self.assertFalse(alive(child), "owned child survived its harness")

    def test_stop_falls_back_to_kill_and_reaps(self):
        with tempfile.TemporaryDirectory() as tmp:
            process = runner.launch(["sleep", "60"], Path(tmp) / "log")
            actions = runner.stop(process, Path(tmp) / "missing.sock", grace=0.5)
            self.assertEqual(actions[-1], "sigkill")
            self.assertTrue(actions[0].startswith("qmp-quit-failed"))
            self.assertIsNotNone(process.returncode)
            self.assertFalse(alive(process.pid))

    def test_sweep_kills_only_an_exactly_identified_stale_guest(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            marker = "rw040-qual-sweep-test"
            stale = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(60)", marker]
            )
            other = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(60)", marker]
            )
            dead = subprocess.Popen(["true"])
            dead.wait()
            try:
                for name, pid, ticks in (
                    ("exact", stale.pid, runner.start_ticks(stale.pid)),
                    ("wrong-ticks", other.pid, runner.start_ticks(other.pid) + 1),
                ):
                    (root / name).mkdir()
                    (root / name / "owner.json").write_text(
                        json.dumps(
                            {
                                "harness_pid": dead.pid,
                                "harness_start_ticks": 0,
                                "marker": marker,
                                "qemu_pid": pid,
                                "qemu_start_ticks": ticks,
                            }
                        )
                    )
                live = root / "live"
                live.mkdir()
                (live / "owner.json").write_text(
                    json.dumps(
                        {
                            "harness_pid": os.getpid(),
                            "harness_start_ticks": runner.start_ticks(os.getpid()),
                            "marker": marker,
                        }
                    )
                )
                # Only private sleep children substitute for QEMU in this lifecycle test.
                with mock.patch.object(
                    runner,
                    "_qemu_matches",
                    side_effect=lambda pid, ticks, marker: (
                        runner.start_ticks(pid) == ticks
                    ),
                ):
                    actions = runner.sweep(root)
                stale.wait(5)
                self.assertFalse(alive(stale.pid))
                self.assertTrue(
                    alive(other.pid), "a PID with different start ticks was signalled"
                )
                self.assertFalse((root / "exact").exists())
                self.assertTrue((root / "wrong-ticks").exists())
                self.assertTrue(
                    live.exists(), "a run whose harness is alive was removed"
                )
                self.assertIn(
                    {"run": "exact", "action": f"observed stale qemu exit {stale.pid}"},
                    actions,
                )
            finally:
                for process in (stale, other):
                    process.kill()
                    process.wait()

    def test_parent_adoption_and_prctl_failure_exit_startup(self):
        for actual_parent, prctl_result in ((55, 0), (44, -1)):
            libc = mock.Mock()
            libc.prctl.return_value = prctl_result
            with (
                self.subTest(actual_parent=actual_parent, prctl=prctl_result),
                mock.patch.object(runner.ctypes, "CDLL", return_value=libc),
                mock.patch.object(runner.os, "getppid", return_value=actual_parent),
                mock.patch.object(
                    runner.os, "_exit", side_effect=RuntimeError("startup exit")
                ) as exiting,
            ):
                with self.assertRaisesRegex(RuntimeError, "startup exit"):
                    runner._pdeathsig(44)
                exiting.assert_called_once_with(127)
        libc = mock.Mock()
        libc.prctl.return_value = 0
        with (
            mock.patch.object(runner.ctypes, "CDLL", return_value=libc),
            mock.patch.object(runner.os, "getppid", return_value=44),
            mock.patch.object(runner.os, "_exit") as exiting,
        ):
            runner._pdeathsig(44)
            exiting.assert_not_called()

    def test_sweep_preserves_unknown_denied_pending_and_mismatched_identity(self):
        for variant in ("no-pid", "denied", "mismatch", "pending"):
            with tempfile.TemporaryDirectory() as tmp, self.subTest(variant=variant):
                root = Path(tmp)
                run_dir = root / "run"
                run_dir.mkdir()
                owner = {
                    "harness_pid": 1,
                    "harness_start_ticks": 0,
                    "marker": "rw040-qual-unit",
                    "qemu_pid": 42,
                    "qemu_start_ticks": 1,
                }
                if variant == "no-pid":
                    del owner["qemu_pid"]
                (run_dir / "owner.json").write_text(json.dumps(owner))
                with (
                    mock.patch.object(runner, "_owner_live", return_value=False),
                    mock.patch.object(
                        runner.os,
                        "pidfd_open",
                        side_effect=PermissionError("denied")
                        if variant == "denied"
                        else None,
                        return_value=99,
                    ),
                    mock.patch.object(runner, "_pidfd_exited", return_value=False),
                    mock.patch.object(
                        runner, "_qemu_matches", return_value=variant != "mismatch"
                    ),
                    mock.patch.object(runner.signal, "pidfd_send_signal") as send,
                    mock.patch.object(runner.os, "close"),
                ):
                    actions = runner.sweep(root)
                    self.assertTrue(run_dir.exists())
                    self.assertTrue(actions[0]["action"].startswith("kept:"))
                    if variant != "pending":
                        send.assert_not_called()

    def test_exit_between_identity_check_and_signal_cannot_hit_replacement(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            run_dir = root / "run"
            run_dir.mkdir()
            target = subprocess.Popen(["sleep", "60"])
            replacement = subprocess.Popen(["sleep", "60"])
            owner = {
                "harness_pid": 1,
                "harness_start_ticks": 0,
                "marker": "rw040-qual-unit",
                "qemu_pid": target.pid,
                "qemu_start_ticks": runner.start_ticks(target.pid),
            }
            (run_dir / "owner.json").write_text(json.dumps(owner))
            original_kill = os.kill

            def checked_then_exited(pid, ticks, marker):
                original_kill(target.pid, runner.signal.SIGKILL)
                target.wait()
                # A later numeric lookup now refers to an unrelated live process.
                owner["qemu_pid"] = replacement.pid
                return True

            try:
                with (
                    mock.patch.object(runner, "_owner_live", return_value=False),
                    mock.patch.object(
                        runner, "_qemu_matches", side_effect=checked_then_exited
                    ),
                    mock.patch.object(
                        runner.os,
                        "kill",
                        side_effect=AssertionError("numeric kill forbidden"),
                    ),
                ):
                    runner.sweep(root)
                self.assertTrue(alive(replacement.pid))
                self.assertFalse(run_dir.exists())
            finally:
                for process in (target, replacement):
                    process.kill()
                    process.wait()


class ConfinementReproductionTests(unittest.TestCase):
    unit = (ROOT / "packaging/systemd/limeos-storage-reader.service").read_text()

    def test_every_reader_confinement_setting_is_carried_over_unchanged(self):
        properties, unknown = guest.confinement_properties(self.unit)
        self.assertEqual(
            unknown, [], "the unit gained a setting the harness cannot compare"
        )
        values = dict(properties)
        self.assertEqual(values["LimitNOFILE"], "256")
        self.assertEqual(values["MemoryMax"], "64M")
        self.assertEqual(values["MemorySwapMax"], "0")
        self.assertEqual(values["TasksMax"], "16")
        self.assertEqual(values["CPUQuota"], "50%")
        self.assertEqual(values["NoNewPrivileges"], "yes")
        self.assertEqual(values["CapabilityBoundingSet"], "")
        self.assertEqual(values["AmbientCapabilities"], "")
        self.assertEqual(values["SystemCallFilter"], "@system-service")
        self.assertEqual(values["RestrictAddressFamilies"], "AF_UNIX")
        self.assertNotIn("ExecStart", values)
        lines = {line.strip() for line in self.unit.splitlines()}
        for key, value in properties:
            self.assertIn(f"{key}={value}", lines)

    def test_confined_command_adds_nothing_beyond_the_unit(self):
        class Ctx:
            confinement = guest.confinement_properties(self.unit)[0]

        argv = guest.confined("rw040-test", ["/bin/true"], Ctx)
        properties = [
            a.removeprefix("--property=") for a in argv if a.startswith("--property=")
        ]
        self.assertEqual(properties, [f"{k}={v}" for k, v in Ctx.confinement])
        self.assertEqual(argv[-1], "/bin/true")
        self.assertFalse(
            any("Capability" in p and not p.endswith("=") for p in properties)
        )


class FixtureConsistencyTests(unittest.TestCase):
    def test_layout_is_internally_consistent(self):
        serials = [d["serial"] for d in LAYOUT["disks"]]
        uuids = [d["uuid"] for d in LAYOUT["disks"]]
        self.assertEqual(len(serials), len(set(serials)))
        self.assertEqual(len(uuids), len(set(uuids)))
        for uuid in uuids:
            self.assertRegex(
                uuid,
                r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-8[0-9a-f]{3}-[0-9a-f]{12}$",
            )
        names = [
            c["name"]
            for group in (
                "containers",
                "transition_containers",
                "unsupported_containers",
            )
            for c in LAYOUT[group]
        ]
        self.assertEqual(len(names), len(set(names)))
        self.assertTrue(all(re.fullmatch(r"rw040-[a-z0-9-]+", n) for n in names))
        self.assertLessEqual(len(names), 64)
        known = {d["mount"] for d in LAYOUT["disks"] if d["mount"]} | set(
            LAYOUT["directories"]
        )
        known |= {b["target"] for b in LAYOUT["binds"]} | {
            s["path"] for s in LAYOUT["symlinks"]
        }
        known |= {"/srv/rw040/main/config/app.conf"}
        for group in ("containers", "transition_containers", "unsupported_containers"):
            for container in LAYOUT[group]:
                for mount in container["mounts"]:
                    if mount["type"] == "bind":
                        self.assertIn(mount["source"], known, container["name"])
                    else:
                        self.assertIn(mount["source"], LAYOUT["volumes"])

    def test_cases_reference_real_stages_and_known_prerequisites(self):
        stages = {name for name, _, _ in guest.STAGES}
        ids = [c["id"] for c in CASES["cases"]]
        self.assertEqual(len(ids), len(set(ids)))
        for case in CASES["cases"]:
            self.assertTrue(set(case["prepared_by"]) <= stages, case["id"])
            self.assertTrue(
                set(case["requires"])
                <= {"integrator-probe", "integrator-worker", "blocking-fixture"}
            )
            self.assertTrue(
                case["requires"],
                f"{case['id']} would run without any integrator prerequisite",
            )

    def test_package_record_is_bound_to_the_qualified_base(self):
        record = json.loads((FIXTURES / "packages-2fd7420.json").read_text())
        self.assertEqual(record["tested_source"], BASE)
        self.assertEqual(record["run_id"], 37746766303)
        self.assertEqual(
            {p["file"] for p in record["packages"]},
            {
                "limeos_0.4.4_amd64.deb",
                "limeos_0.4.4+ci.1_amd64.deb",
                "limeos-shadow_0.4.4_amd64.deb",
            },
        )

    def test_done_stages_match_the_guest_exactly(self):
        self.assertEqual(runner.EXPECTED_STAGES, {name for name, _, _ in guest.STAGES})


def probe_event(case="engine", phase="collected", status="ok", **changes):
    value = {
        "version": 1,
        "case": case,
        "phase": phase,
        "status": status,
        "stage": "owners.collected" if phase == "collected" else "owners.revalidated",
        "error": None,
        "effective_uid": 0,
        "observations": {
            "engine": {"declarations": {"observed_at": 100, "containers": []}}
        },
    }
    value.update(changes)
    return adapter.PREFIX + json.dumps(value).encode() + b"\n"


class ProbeReceiverTests(unittest.TestCase):
    summary = b"\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 1.00s\n\n"

    def test_exact_fragmented_events_then_successful_exit(self):
        called = []
        receiver = adapter.Events("engine", called.append)
        data = (
            b"running 1 test\ntest qualification_probe ... \n"
            + probe_event()
            + probe_event(phase="final")
            + self.summary
        )
        with mock.patch.object(adapter.time, "time", return_value=101.25):
            for index in range(0, len(data), 7):
                receiver.feed(data[index : index + 7])
        result = receiver.finish(0)
        self.assertEqual(result["final_status"], "ok")
        self.assertEqual(len(called), 1)
        self.assertEqual(bytes(receiver.raw), data)

    def test_typed_refusal_is_not_empty_success(self):
        receiver = adapter.Events("engine")
        receiver.feed(
            probe_event(
                phase="final",
                status="refused",
                stage="engine.collect",
                error={"library": "engine", "code": "Unavailable"},
                observations={},
            )
            + self.summary
        )
        self.assertEqual(receiver.finish(0)["final_status"], "refused")

    def test_missing_malformed_truncated_duplicate_and_late_events_refuse(self):
        valid_final = probe_event(phase="final")
        variants = [
            probe_event(),
            probe_event() + valid_final.rstrip(b"\n"),
            probe_event() + valid_final + valid_final,
            probe_event() + valid_final + probe_event(),
            adapter.PREFIX + b"{bad}\n",
            probe_event(version=2),
            probe_event(case="storage"),
            probe_event(effective_uid=1000),
            probe_event(observations={}),
            b"prefix " + valid_final,
            valid_final,
        ]
        for index, data in enumerate(variants):
            with self.subTest(index=index), self.assertRaises((ValueError, KeyError)):
                receiver = adapter.Events("engine")
                receiver.feed(data + self.summary)
                receiver.finish(0)

    def test_oversized_body_noise_and_duplicate_json_keys_refuse(self):
        for data in (
            adapter.PREFIX + bytes(adapter.MAX_EVENT + 1),
            b"x" * (adapter.MAX_NOISE + 1) + b"\n",
            adapter.PREFIX + b'{"version":1,"version":1}\n',
        ):
            with self.assertRaises(ValueError):
                adapter.Events("engine").feed(data)

    def test_selected_owner_keys_are_required_for_collected_and_final_success(self):
        for phase in ("collected", "final"):
            receiver = adapter.Events("engine-processes")
            with (
                self.assertRaisesRegex(ValueError, "selected-owner"),
                mock.patch.object(adapter.time, "time", return_value=101),
            ):
                if phase == "final":
                    receiver.feed(
                        probe_event(
                            case="engine-processes",
                            observations={
                                "engine": {"declarations": {"observed_at": 100}},
                                "processes": {},
                            },
                        )
                    )
                receiver.feed(probe_event(case="engine-processes", phase=phase))

    def test_prerequisite_cases_cannot_emit_provisional_collection(self):
        with self.assertRaises(ValueError):
            adapter.Events("contract").feed(probe_event(case="contract"))

    def test_json_success_cannot_override_failed_or_missing_libtest_exit(self):
        for exit_code, summary in ((1, self.summary), (0, b"")):
            receiver = adapter.Events("engine")
            with mock.patch.object(adapter.time, "time", return_value=101):
                receiver.feed(probe_event() + probe_event(phase="final") + summary)
            with self.assertRaises(ValueError):
                receiver.finish(exit_code)

    def test_original_age_and_fractional_delivery_expiry_are_conservative(self):
        for final_time, stamp in ((105.01, 100), (104, 101), (99.9, 100)):
            receiver = adapter.Events("engine")
            with mock.patch.object(adapter.time, "time", return_value=100):
                receiver.feed(probe_event())
            observed = {
                "engine": {"declarations": {"observed_at": stamp, "containers": []}}
            }
            with mock.patch.object(adapter.time, "time", return_value=final_time):
                receiver.feed(
                    probe_event(phase="final", observations=observed) + self.summary
                )
            with self.assertRaises(ValueError):
                receiver.finish(0)

    def test_late_storage_delivery_cannot_qualify_timestamp_free_report(self):
        with mock.patch.object(adapter.time, "monotonic", return_value=100):
            receiver = adapter.Events("storage")
            receiver.feed(
                probe_event(
                    case="storage",
                    stage="storage.collected",
                    observations={"storage": {}},
                )
            )
        with mock.patch.object(adapter.time, "monotonic", return_value=105.01):
            receiver.feed(
                probe_event(
                    case="storage",
                    phase="final",
                    stage="storage.revalidated",
                    observations={"storage": {}},
                )
                + self.summary
            )
        with self.assertRaisesRegex(ValueError, "conservative lifetime"):
            receiver.finish(0)

    def test_fixed_pause_selection_has_no_auth_or_clock_overrides(self):
        self.assertEqual(
            adapter.ARGV,
            ["--exact", "qualification_probe", "--nocapture", "--test-threads=1"],
        )
        self.assertEqual(
            adapter.probe_environment("engine", 5000),
            {
                "LIMEOS_RW040_PROBE_CASE": "engine",
                "LIMEOS_RW040_PROBE_SOCKET": "/run/docker.sock",
                "LIMEOS_RW040_PROBE_PAUSE_MS": "5000",
            },
        )
        for case, pause in (
            ("invented", 0),
            ("engine", 5001),
            ("engine-dependencies", 0),
        ):
            with self.assertRaises(ValueError):
                adapter.probe_environment(case, pause)

    def test_combined_remains_explicitly_blocked(self):
        receiver = adapter.Events("combined")
        receiver.feed(
            probe_event(
                case="combined",
                phase="final",
                status="blocked",
                stage="prerequisites",
                observations={"combined_worker": "unavailable"},
            )
            + self.summary
        )
        self.assertEqual(receiver.finish(0)["final_status"], "blocked")

    def test_sampler_rejects_dead_retained_identity_before_proc_read(self):
        sampler = guest.Sampler("private-unit")
        sampler.pid, sampler.pidfd, sampler.start_ticks = 12345, 999, 1
        with (
            mock.patch.object(adapter, "pidfd_exited", return_value=True),
            mock.patch.object(
                guest.Path,
                "read_text",
                side_effect=AssertionError("dead PID must not be sampled"),
            ),
            mock.patch.object(guest.os, "close"),
        ):
            sampler.run()
        self.assertEqual(sampler.samples, 0)
        self.assertIsNone(sampler.pidfd)

    def test_real_nonempty_serialized_shape_matches_complete_instances_and_pids(self):
        members = {
            "members": [
                {
                    "id": "a" * 64,
                    "image": "sha256:" + "b" * 64,
                    "started_at": "2026-10-09T12:00:00Z",
                    "running": True,
                    "pid": 123,
                },
                {
                    "id": "c" * 64,
                    "image": "sha256:" + "d" * 64,
                    "started_at": "2026-10-08T12:00:00Z",
                    "running": False,
                    "pid": 0,
                },
            ]
        }
        containers = [
            {
                "container": {
                    "resource": "container:" + item["id"],
                    "image": item["image"],
                    "started_at": item["started_at"],
                    "running": item["running"],
                },
                "mounts": [],
            }
            for item in members["members"]
        ]
        snapshot = {
            "declarations": {"containers": containers},
            "running": [{"container": containers[0]["container"], "pid": 123}],
        }
        self.assertEqual(
            guest.compare_membership(snapshot, members),
            {"complete_instances": True, "running_pid_bindings": True},
        )
        snapshot["running"][0]["pid"] = 124
        self.assertFalse(
            guest.compare_membership(snapshot, members)["running_pid_bindings"]
        )
        snapshot["declarations"]["containers"].pop()
        self.assertFalse(
            guest.compare_membership(snapshot, members)["complete_instances"]
        )

    def test_closure_is_ambiguous_without_observed_cgroup(self):
        with mock.patch.object(
            guest, "show", return_value={"ActiveState": "inactive", "MainPID": "0"}
        ):
            report = guest.unit_closure("private-unit", None)
        self.assertFalse(report["verified"])
        self.assertIn("ambiguity", report)


if __name__ == "__main__":
    unittest.main()
