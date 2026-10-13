"""Local provenance refusals; no SSH, apt install or ARM64 execution."""

import copy
import gzip
import io
import json
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
import warnings
import zipfile
from datetime import UTC, datetime
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import ci_provenance as ci
import qualification as q
import record_run


def records():
    source = "a" * 40
    run = {
        "id": 12,
        "head_sha": source,
        "status": "completed",
        "conclusion": "success",
        "repository": {"id": 3, "full_name": ci.REPOSITORY},
        "head_repository": {"id": 3, "full_name": ci.REPOSITORY},
        "head_commit": {"id": source},
        "path": ".github/workflows/ci.yml",
        "url": f"{ci.API}/actions/runs/12",
        "html_url": f"{ci.WEB}/actions/runs/12",
    }
    job = {
        "id": 13,
        "run_id": 12,
        "head_sha": source,
        "status": "completed",
        "conclusion": "success",
        "run_url": run["url"],
        "url": f"{ci.API}/actions/jobs/13",
        "html_url": f"{ci.WEB}/actions/runs/12/job/13",
        "name": "check (ubuntu-24.04-arm)",
        "labels": ["ubuntu-24.04-arm"],
        "steps": [
            {"name": name, "number": n, "status": "completed", "conclusion": "success"}
            for n, name in enumerate(ci.REQUIRED_GATES, 1)
        ],
    }
    artifact = {
        "id": 14,
        "name": "debian-ubuntu-24.04-arm",
        "url": f"{ci.API}/actions/artifacts/14",
        "archive_download_url": f"{ci.API}/actions/artifacts/14/zip",
        "digest": "sha256:" + "b" * 64,
        "workflow_run": {
            "id": 12,
            "head_sha": source,
            "repository_id": 3,
            "head_repository_id": 3,
        },
    }
    return run, job, artifact, source


class CiIdentityTests(unittest.TestCase):
    def test_successful_native_records_preserve_observations(self):
        run, job, artifact, source = records()
        before = copy.deepcopy((run, job, artifact))
        ci.validate_ci_records(run, job, artifact, source)
        self.assertEqual((run, job, artifact), before)

    def test_cross_repository_source_run_job_and_architecture_refuse(self):
        mutations = [
            ("run", "repository", {"id": 3, "full_name": "elsewhere/limeos"}),
            ("run", "head_sha", "c" * 40),
            ("run", "path", ".github/workflows/other.yml"),
            ("run", "conclusion", "failure"),
            ("job", "head_sha", "c" * 40),
            ("job", "run_id", 99),
            ("job", "run_url", "https://example.org"),
            ("job", "html_url", f"{ci.WEB}/actions/runs/12/job/99"),
            ("job", "name", "check (ubuntu-24.04)"),
            ("job", "labels", ["ubuntu-24.04"]),
            ("artifact", "name", "debian-ubuntu-24.04"),
            (
                "artifact",
                "workflow_run",
                {
                    "id": 99,
                    "head_sha": "a" * 40,
                    "repository_id": 3,
                    "head_repository_id": 3,
                },
            ),
            ("artifact", "archive_download_url", "https://example.org/archive.zip"),
            ("artifact", "digest", "sha256:invalid"),
        ]
        for label, field, value in mutations:
            run, job, artifact, source = records()
            {"run": run, "job": job, "artifact": artifact}[label][field] = value
            with self.subTest(label=label, field=field), self.assertRaises(ValueError):
                ci.validate_ci_records(run, job, artifact, source)

    def test_every_gate_missing_failed_skipped_or_reordered_refuses(self):
        for n in range(len(ci.REQUIRED_GATES)):
            for mode in ("missing", "failed", "skipped", "order"):
                run, job, artifact, source = records()
                if mode == "missing":
                    del job["steps"][n]
                elif mode == "order":
                    job["steps"][n]["number"] = 1000
                    if n == len(ci.REQUIRED_GATES) - 1:
                        job["steps"][n]["number"] = 1
                else:
                    job["steps"][n]["conclusion"] = mode
                with self.subTest(gate=n, mode=mode), self.assertRaises(ValueError):
                    ci.validate_ci_records(run, job, artifact, source)

    def test_duplicate_gate_name_and_number_refuse(self):
        for mode in ("name", "number"):
            run, job, artifact, source = records()
            if mode == "name":
                job["steps"].append(copy.deepcopy(job["steps"][0]))
            else:
                job["steps"][1]["number"] = job["steps"][0]["number"]
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                ci.validate_ci_records(run, job, artifact, source)

    def test_unknown_and_ambiguous_build_provenance_refuses(self):
        from test_harness import build_fixture

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "build.json"
            for change in (
                {"build_kind": "cross-compiled"},
                {"ci": {}},
                {"provenance_version": 1},
            ):
                path.write_text(json.dumps(build_fixture() | change))
                with self.subTest(change=change), self.assertRaises(ValueError):
                    q.load_build(path)
            for version in (0, 2, True):
                build = build_fixture() | {
                    "build_kind": ci.BUILD_KIND,
                    "provenance_version": version,
                }
                path.write_text(json.dumps(build))
                with (
                    self.subTest(version=version),
                    self.assertRaisesRegex(ValueError, "version"),
                ):
                    q.load_build(path)
            path.write_text(
                json.dumps(
                    build_fixture()
                    | {"build_kind": ci.BUILD_KIND, "provenance_version": 1}
                )
            )
            with self.assertRaisesRegex(ValueError, "ambiguous"):
                q.load_build(path)


class InputBytesTests(unittest.TestCase):
    def test_input_symlink_and_path_escape_refuse(self):
        for name in ("../outside", "/absolute", "x/../y", "./x", "", "."):
            with self.subTest(path=name), self.assertRaises(ValueError):
                ci.relative(name)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "real").write_text("bytes")
            (root / "alias").symlink_to(root / "real")
            with self.assertRaisesRegex(ValueError, "symlink"):
                ci.file_map(root)

    def test_changed_archive_digest_precedes_package_inspection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run, job, artifact, source = records()
            archive = root / "artifact.zip"
            archive.write_bytes(b"wrong archive")
            artifact["size_in_bytes"] = archive.stat().st_size
            for name, value in (("run", run), ("job", job), ("artifact", artifact)):
                (root / f"{name}.json").write_text(json.dumps(value))
            with patch.object(ci, "package_payload") as inspect:
                with self.assertRaisesRegex(ValueError, "digest/size"):
                    ci.inspect_ci(root, source)
                inspect.assert_not_called()

    def test_extra_escaped_duplicate_and_symlink_zip_entries_refuse(self):
        for mode in ("extra", "escape", "duplicate", "symlink"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                run, job, artifact, source = records()
                archive = root / "artifact.zip"
                with warnings.catch_warnings(), zipfile.ZipFile(archive, "w") as zipped:
                    warnings.simplefilter("ignore", UserWarning)
                    for name in ci.ORIGINALS:
                        member = zipfile.ZipInfo(name)
                        if mode == "symlink":
                            member.external_attr = 0o120777 << 16
                        zipped.writestr(member, b"package")
                    if mode != "symlink":
                        zipped.writestr(
                            {
                                "extra": "extra",
                                "escape": "../outside",
                                "duplicate": "limeos_0.4.4_arm64.deb",
                            }[mode],
                            b"package",
                        )
                artifact.update(
                    digest="sha256:" + q.sha(archive),
                    size_in_bytes=archive.stat().st_size,
                )
                for name, value in (("run", run), ("job", job), ("artifact", artifact)):
                    (root / f"{name}.json").write_text(json.dumps(value))
                with patch.object(ci, "package_payload") as inspect:
                    with self.assertRaises(ValueError):
                        ci.inspect_ci(root, source)
                    inspect.assert_not_called()

    def bundle(self, root, *, extra=None, schema=9, member_type=None):
        runtime = {
            ".github/workflows/ci.yml": b"workflow",
            "packaging/Containerfile": b"container",
            "packaging/build.py": b"builder",
            "crates/persistence/src/lib.rs": f"pub const SCHEMA_VERSION: u32 = {schema};".encode(),
        }
        manifest = {
            "identity": {
                "source_commit": "a" * 40,
                "fixtures_commit": "b" * 40,
                "package_version": "0.4.8+arm64.3",
                "architecture": "arm64",
                "authority_schema": 9,
            },
            "runtime_source_sha256": {n: ci.digest(d) for n, d in runtime.items()},
            "frontend_dist_sha256": {"frontend/dist/index.html": ci.digest(b"ui")},
            "fixtures_sha256": {"test.py": ci.digest(b"fixture")},
        }
        members = {"source/" + n: d for n, d in runtime.items()} | {
            "source/frontend/dist/index.html": b"ui",
            "fixtures/test.py": b"fixture",
            "source-manifest.json": json.dumps(manifest).encode(),
        }
        if extra:
            members[extra] = b"unknown"
        path = root / "source.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            for name, data in members.items():
                member = tarfile.TarInfo(name)
                member.size = len(data)
                if member_type and name == "fixtures/test.py":
                    member.type, member.linkname, member.size = (
                        member_type,
                        "outside",
                        0,
                    )
                archive.addfile(member, io.BytesIO(data))
        manifest["bundle_sha256"] = q.sha(path)
        return path, manifest

    def test_bundle_all_bytes_schema_file_set_and_pipeline_bind(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for options in (
                {},
                {"extra": "unknown"},
                {"extra": "../outside"},
                {"schema": 8},
                {"member_type": tarfile.SYMTYPE},
            ):
                bundle, manifest = self.bundle(root, **options)
                with (
                    patch.object(ci, "WORKFLOW_SHA256", ci.digest(b"workflow")),
                    patch.object(ci, "CONTAINERFILE_SHA256", ci.digest(b"container")),
                ):
                    if options:
                        with self.assertRaises(ValueError):
                            ci.verify_bundle(bundle, manifest)
                    else:
                        ci.verify_bundle(bundle, manifest)
                if not options:
                    with self.assertRaisesRegex(ValueError, "unreviewed"):
                        ci.verify_bundle(bundle, manifest)
                    manifest["fixtures_sha256"]["test.py"] = "e" * 64
                    with self.assertRaises(ValueError):
                        ci.verify_bundle(bundle, manifest)


@unittest.skipUnless(
    shutil.which("dpkg-deb"), "dpkg-deb required for local package byte test"
)
class PackageBytesTests(unittest.TestCase):
    def package(self, root, machine=183):
        tree = root / "tree"
        (tree / "DEBIAN").mkdir(parents=True)
        (tree / "DEBIAN/control").write_text(
            "Package: limeos\nVersion: 0.4.4\nArchitecture: arm64\nMaintainer: Test <test@example.invalid>\nDescription: Synthetic ELF headers; never executable\n"
        )
        runtime = tree / "usr/lib/limeos"
        runtime.mkdir(parents=True)
        for name in q.BINARIES:
            (runtime / name).write_bytes(
                b"\x7fELF\x02\x01"
                + bytes(12)
                + machine.to_bytes(2, "little")
                + bytes(44)
            )
        (runtime / "ui").mkdir()
        (runtime / "ui/index.html").write_text("ui")
        package = root / "package.deb"
        subprocess.run(
            ["dpkg-deb", "--build", "--root-owner-group", str(tree), str(package)],
            check=True,
            capture_output=True,
        )
        return package

    def test_all_four_actual_elf_headers_hashes_and_package_controls(self):
        with tempfile.TemporaryDirectory() as directory:
            package = self.package(Path(directory))
            observed, _ = ci.package_payload(package)
            self.assertEqual(set(observed["binaries"]), set(q.BINARIES))
            self.assertTrue(
                all(b["machine"] == "AArch64" for b in observed["binaries"].values())
            )
            self.assertEqual(
                observed["control"],
                "Package: limeos\nVersion: 0.4.4\nArchitecture: arm64\n",
            )
            self.assertEqual(
                observed["frontend"], {"frontend/dist/index.html": ci.digest(b"ui")}
            )

    def test_amd64_header_refuses_despite_arm64_control(self):
        with tempfile.TemporaryDirectory() as directory:
            package = self.package(Path(directory), machine=62)
            with self.assertRaisesRegex(ValueError, "AArch64"):
                ci.package_payload(package)


class RepositoryTests(unittest.TestCase):
    def repository(self, root):
        package = root / "pool/main/limeos_0.4.4_arm64.deb"
        package.parent.mkdir(parents=True)
        package.write_bytes(b"package")
        index = f"Package: limeos\nVersion: 0.4.4\nArchitecture: arm64\nFilename: pool/main/{package.name}\nSize: 7\nSHA256: {q.sha(package)}\n\n".encode()
        hashes = []
        release_root = root / "dists/stable"
        for architecture in ("arm64", "amd64"):
            base = release_root / f"main/binary-{architecture}/Packages"
            base.parent.mkdir(parents=True)
            base.write_bytes(index)
            base.with_suffix(".gz").write_bytes(gzip.compress(index, mtime=0))
            for path in (base, base.with_suffix(".gz")):
                hashes.append(
                    f" {q.sha(path)} {path.stat().st_size} {path.relative_to(release_root).as_posix()}\n"
                )
        release = (
            "Origin: LimeOS\nSHA256:\n"
            + "".join(hashes)
            + "Valid-Until: Thu, 01 Jan 2037 00:00:00 UTC\n"
        )
        (release_root / "Release").write_text(release)
        (release_root / "InRelease").write_text(
            "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\n"
            + release
            + "-----BEGIN PGP SIGNATURE-----\nfixture\n"
        )
        (release_root / "Release.gpg").write_text("fixture signature")
        (root / "limeos-archive-keyring.gpg").write_text("fixture public key")
        return "A" * 40

    def signature(self, fingerprint):
        return SimpleNamespace(
            returncode=0,
            stdout=f"[GNUPG:] VALIDSIG {fingerprint} other-status-fields\n",
            stderr="",
        )

    def test_invalid_fingerprint_refuses_before_gpg(self):
        with patch.object(ci.subprocess, "run") as command:
            for fingerprint in ("invalid", "a" * 40, "A" * 39):
                with (
                    self.subTest(fingerprint=fingerprint),
                    self.assertRaises(ValueError),
                ):
                    ci.verify_repository(Path("/not-read"), fingerprint)
            command.assert_not_called()

    def test_signed_index_package_and_validity_bindings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            key = self.repository(root)
            with patch.object(
                ci.subprocess, "run", return_value=self.signature(key)
            ) as command:
                self.assertEqual(ci.verify_repository(root, key), ci.file_map(root))
                self.assertEqual(command.call_count, 2)
                with self.assertRaisesRegex(ValueError, "expired"):
                    ci.verify_repository(
                        root, key, now=datetime(2038, 1, 1, tzinfo=UTC)
                    )
                (root / "pool/main/limeos_0.4.4_arm64.deb").write_bytes(b"changed")
                with self.assertRaisesRegex(ValueError, "package index"):
                    ci.verify_repository(root, key)

    def test_bad_signature_wrong_key_and_unbound_index_refuse(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            key = self.repository(root)
            for result in (
                SimpleNamespace(returncode=1, stdout="", stderr="bad signature"),
                self.signature("B" * 40),
            ):
                with (
                    patch.object(ci.subprocess, "run", return_value=result),
                    self.assertRaisesRegex(ValueError, "signature/fingerprint"),
                ):
                    ci.verify_repository(root, key)
            index = root / "dists/stable/main/binary-arm64/Packages"
            index.write_text("changed index")
            index.with_suffix(".gz").write_bytes(
                gzip.compress(index.read_bytes(), mtime=0)
            )
            with (
                patch.object(ci.subprocess, "run", return_value=self.signature(key)),
                self.assertRaisesRegex(ValueError, "Release index"),
            ):
                ci.verify_repository(root, key)

    def test_unexpected_private_or_duplicate_repository_files_refuse(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            key = self.repository(root)
            (root / "private-key").write_text("unexpected")
            with patch.object(ci.subprocess, "run") as command:
                with self.assertRaisesRegex(ValueError, "public test repository"):
                    ci.verify_repository(root, key)
                command.assert_not_called()

    def test_frozen_apt_ftparchive_self_entry_binds_only_its_written_header(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            key = self.repository(root)
            release_path = root / "dists/stable/Release"
            header = "Origin: LimeOS\n"
            old = release_path.read_text()
            release = old.replace(
                "SHA256:\n",
                "MD5Sum:\nSHA256:\n"
                + f" {ci.digest(header.encode())} {len(header)} Release\n",
            )
            release_path.write_text(release)
            signed = root / "dists/stable/InRelease"
            signed.write_text(signed.read_text().replace(old, release))
            with patch.object(ci.subprocess, "run", return_value=self.signature(key)):
                self.assertEqual(ci.verify_repository(root, key), ci.file_map(root))
                wrong = release.replace(ci.digest(header.encode()), "0" * 64)
                release_path.write_text(wrong)
                signed.write_text(signed.read_text().replace(release, wrong))
                with self.assertRaisesRegex(ValueError, "Release header"):
                    ci.verify_repository(root, key)


class BuildBindingTests(unittest.TestCase):
    def build(self, root):
        inputs = root / "inputs"
        for name in ("ci", "packaging", "repo/pool/main"):
            (inputs / name).mkdir(parents=True)
        for name in ("run.json", "job.json", "artifact.json", "artifact.zip"):
            (inputs / "ci" / name).write_text("retained observation")
        for profile in ("standard", "shadow"):
            (inputs / "packaging" / f"{profile}.txt").write_text(
                "package comparison passed"
            )
        (inputs / "source.tar.gz").write_bytes(b"source bundle")
        run, job, artifact, source = records()
        identity = {
            "source_commit": source,
            "fixtures_commit": "b" * 40,
            "package_version": "0.4.8+arm64.3",
            "authority_schema": 9,
            "architecture": "arm64",
            "qualification_kind": "native-ci-build-kvm-install",
        }
        binaries = {
            n: {"sha256": "c" * 64, "machine": "AArch64", "bytes": 64}
            for n in q.BINARIES
        }
        frontend = {"frontend/dist/index.html": "d" * 64}
        contracts = {"contracts/generated/authority-v9.sql": "e" * 64}
        original = {"binaries": binaries, "frontend": frontend, "contracts": contracts}
        observed_ci = {
            "observations": {"run": run, "job": job, "artifact": artifact},
            "packages": {n: original for n in ci.ORIGINALS},
        }
        manifest = {
            "identity": identity,
            "runtime_source_sha256": {"packaging/build.py": "f" * 64, **contracts},
            "frontend_dist_sha256": frontend,
            "fixtures_sha256": {"tests/fixture.py": "1" * 64},
            "frontend_build": {
                "kind": "native-ci",
                "run_id": 12,
                "job_id": 13,
                "artifact_zip_sha256": "b" * 64,
            },
            "bundle_sha256": q.sha(inputs / "source.tar.gz"),
        }
        path = root / "build-result.json"
        manifest_path = inputs / "source-manifest.json"
        manifest_path.write_text(json.dumps(manifest))
        names = {f"{p}_0.4.8+arm64.3_arm64.deb": p for p in ("limeos", "limeos-shadow")}
        controls = {}
        for name, package in names.items():
            (inputs / "repo/pool/main" / name).write_text("candidate package")
            controls[name] = (
                f"Package: {package}\nVersion: 0.4.8+arm64.3\nArchitecture: arm64\n"
            )
        build = {
            "build_kind": ci.BUILD_KIND,
            "provenance_version": 1,
            **{
                k: manifest[k]
                for k in (
                    "identity",
                    "runtime_source_sha256",
                    "frontend_dist_sha256",
                    "fixtures_sha256",
                )
            },
            "source_manifest_sha256": q.sha(manifest_path),
            "ci": observed_ci,
            "binaries": binaries,
            "packages": {n: q.sha(inputs / "repo/pool/main" / n) for n in names},
            "package_control": controls,
            "package_payloads": {
                n: {"data": {"file": {"sha256": "2" * 64}}} for n in names
            },
            "packaging": {
                "method": "frozen-builder-payload-comparison",
                "builder_sha256": "f" * 64,
                "steps": [
                    {
                        "profile": p,
                        "exit": 0,
                        "log_sha256": q.sha(inputs / "packaging" / f"{p}.txt"),
                    }
                    for p in ("standard", "shadow")
                ],
            },
            "signing_fingerprint": "A" * 40,
            "repository_sha256": ci.file_map(inputs / "repo"),
            "input_sha256": ci.file_map(inputs),
        }
        # Lower-level byte readers have their own real archive/package tests.
        # Only substitute them here to isolate cross-record comparisons.
        original_build = copy.deepcopy(build)

        def inspect(package):
            name = package.name
            return {
                **original,
                "control": original_build["package_control"][name],
                "payload": original_build["package_payloads"][name],
            }, {}

        return path, build, observed_ci, inspect

    def load(self, path, build, observed_ci, inspect, repo=None):
        path.write_text(json.dumps(build))
        with (
            patch.object(ci, "verify_bundle"),
            patch.object(ci, "inspect_ci", return_value=(observed_ci, {})),
            patch.object(
                ci, "verify_repository", side_effect=lambda r, k: ci.file_map(r)
            ),
            patch.object(ci, "package_payload", side_effect=inspect),
        ):
            return q.load_build(path, repo)

    def test_ci_build_needs_no_fabricated_guest_steps_or_test_count(self):
        with tempfile.TemporaryDirectory() as directory:
            path, build, observed_ci, inspect = self.build(Path(directory))
            self.assertEqual(self.load(path, build, observed_ci, inspect), build)
            self.assertNotIn("steps", build)
            self.assertNotIn("tests", build)

    def test_mixed_manifest_binary_package_builder_and_control_refuse(self):
        for field in (
            "source_commit",
            "fixtures_commit",
            "schema",
            "frontend",
            "fixtures",
            "runtime",
            "binary",
            "package",
            "control",
            "builder",
            "gate",
            "payload",
            "ci",
        ):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                path, build, observed_ci, inspect = self.build(Path(directory))
                build = copy.deepcopy(build)
                if field in ("source_commit", "fixtures_commit"):
                    build["identity"][field] = "9" * 40
                elif field == "schema":
                    build["identity"]["authority_schema"] = 8
                elif field in ("frontend", "fixtures", "runtime"):
                    build[
                        {
                            "frontend": "frontend_dist_sha256",
                            "fixtures": "fixtures_sha256",
                            "runtime": "runtime_source_sha256",
                        }[field]
                    ]["unexpected"] = "9" * 64
                elif field == "binary":
                    build["binaries"]["limeos-core"]["sha256"] = "9" * 64
                elif field == "package":
                    build["packages"][next(iter(build["packages"]))] = "9" * 64
                elif field == "control":
                    build["package_control"][next(iter(build["package_control"]))] = (
                        "wrong"
                    )
                elif field == "builder":
                    build["packaging"]["builder_sha256"] = "9" * 64
                elif field == "gate":
                    build["packaging"]["steps"][0]["exit"] = 1
                elif field == "payload":
                    build["package_payloads"][
                        next(iter(build["package_payloads"]))
                    ] = {}
                else:
                    build["ci"]["observations"]["run"]["id"] = 999
                with self.assertRaises(ValueError):
                    self.load(path, build, observed_ci, inspect)

    def test_changed_adjacent_input_and_external_repository_refuse(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path, build, observed_ci, inspect = self.build(root)
            (root / "inputs/ci/artifact.zip").write_text("changed")
            with self.assertRaisesRegex(ValueError, "input file manifest"):
                self.load(path, build, observed_ci, inspect)
            build["input_sha256"] = ci.file_map(root / "inputs")
            external = root / "external-repo"
            shutil.copytree(root / "inputs/repo", external)
            next(external.rglob("*.deb")).write_text("changed")
            with self.assertRaisesRegex(ValueError, "installed repository"):
                self.load(path, build, observed_ci, inspect, external)

    def test_recorder_distinguishes_ci_compile_from_untested_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path, build, observed_ci, inspect = self.build(root)
            self.load(path, build, observed_ci, inspect)
            argv = [
                "record_run.py",
                "--run",
                "ci-inputs",
                "--build-result",
                str(path),
                "--source-manifest",
                str(root / "inputs/source-manifest.json"),
            ]
            with (
                patch.object(record_run, "ROOT", root),
                patch.object(record_run, "load_build", return_value=build),
                patch.object(sys, "argv", argv),
            ):
                record_run.main()
            report = root / "docs/rewrite-evidence/arm64/ci-inputs"
            status = json.loads((report / "qualification-status.json").read_text())
            self.assertEqual(status["build_provenance"]["kind"], ci.BUILD_KIND)
            self.assertIn("GitHub CI", status["build_provenance"]["compilation"])
            self.assertEqual(
                status["build_provenance"]["installed_measurements"], "untested"
            )
            self.assertTrue(
                {"install", "storage", "container", "upgrade", "extra"}.issubset(
                    status["untested_gates"]
                )
            )
            self.assertEqual(
                (report / "build/ci/job.json").read_bytes(),
                (root / "inputs/ci/job.json").read_bytes(),
            )


if __name__ == "__main__":
    unittest.main()
