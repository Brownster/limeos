"""Closed source/artifact checks for the CI-only probe producer."""

import copy
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "rw040_probe_artifact",
    Path(__file__).resolve().parents[2] / "scripts/build_rw040_probe_artifact.py",
)
producer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(producer)


def artifact(root):
    return {
        "reason": "compiler-artifact",
        "target": {
            "name": "combined_read_probe",
            "kind": ["test"],
            "src_path": str(root / producer.PROBE),
        },
        "manifest_path": str(root / "bins/executor/Cargo.toml"),
        "profile": {"opt_level": "3", "test": True, "debug_assertions": False},
        "executable": str(root / "target/release/deps/combined_read_probe-a012bc"),
    }


def messages(*records):
    return "\n".join(json.dumps(record) for record in records)


def elf(machine=62):
    data = bytearray(64)
    data[:7] = b"\x7fELF\x02\x01\x01"
    data[18:20] = machine.to_bytes(2, "little")
    return bytes(data)


class ProbeArtifactTests(unittest.TestCase):
    def test_selects_only_the_exact_completed_release_test(self):
        root = Path("/build")
        record = artifact(root)
        unrelated = copy.deepcopy(record)
        unrelated["target"]["name"] = "another_test"
        text = messages(
            unrelated, record, {"reason": "build-finished", "success": True}
        )
        self.assertEqual(
            producer.select_executable(text, root), Path(record["executable"])
        )

    def test_absent_multiple_and_unfinished_artifacts_refuse(self):
        root = Path("/build")
        record = artifact(root)
        success = {"reason": "build-finished", "success": True}
        for records in [
            (success,),
            (record, record, success),
            (record,),
            (record, success, success),
            (record, {"reason": "build-finished", "success": False}),
            (record, {"reason": "build-finished", "success": 1}),
        ]:
            with self.subTest(records=records), self.assertRaises(ValueError):
                producer.select_executable(messages(*records), root)

    def test_wrong_name_kind_source_package_profile_and_output_paths_refuse(self):
        root = Path("/build")
        good = artifact(root)
        for field, value in [
            ("name", "combined_read_probe_fake"),
            ("kind", ["bin"]),
            ("src_path", "/build/bins/executor/tests/other.rs"),
        ]:
            record = copy.deepcopy(good)
            record["target"][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                producer.select_executable(
                    messages(record, {"reason": "build-finished", "success": True}),
                    root,
                )
        variants = [
            {"manifest_path": "/build/bins/core/Cargo.toml"},
            {"profile": {"opt_level": "0", "test": True, "debug_assertions": True}},
            {"executable": None},
            {"executable": "/tmp/combined_read_probe-a012bc"},
            {"executable": "/build/target/release/deps/another_test-a012bc"},
            {"executable": "/build/target/release/deps/../combined_read_probe-a012bc"},
        ]
        for fields in variants:
            with (
                self.subTest(fields=fields),
                self.assertRaises((ValueError, TypeError)),
            ):
                producer.select_executable(
                    messages(
                        good | fields, {"reason": "build-finished", "success": True}
                    ),
                    root,
                )

    def test_malformed_compiler_records_refuse(self):
        for text in [
            "not JSON",
            "[]",
            messages({"reason": "compiler-artifact", "target": []}),
        ]:
            with self.subTest(text=text), self.assertRaises((ValueError, TypeError)):
                producer.select_executable(text, Path("/build"))

    def test_elf_architecture_source_hash_and_commit_identity_refuse(self):
        source = (producer.ROOT / producer.PROBE).read_bytes()
        manifest = producer.supply_manifest("a" * 40, source, elf())
        self.assertEqual(
            set(manifest),
            {
                "contract",
                "source_commit",
                "source_sha256",
                "binary",
                "binary_sha256",
                "toolchain",
                "profile",
                "library_source",
            },
        )
        self.assertEqual(manifest["library_source"], manifest["source_commit"])
        for commit, data, binary in [
            ("a" * 39, source, elf()),
            ("A" * 40, source, elf()),
            ("a" * 40, source + b"changed", elf()),
            ("a" * 40, source, elf(183)),
            ("a" * 40, source, b"\x7fELF"),
            ("a" * 40, source, b"\x7fELF\x02\x02" + elf()[6:]),
        ]:
            with (
                self.subTest(commit=commit, machine=binary[18:20]),
                self.assertRaises(ValueError),
            ):
                producer.supply_manifest(commit, data, binary)

    def test_glibc_and_dynamic_libraries_are_debian_12_compatible(self):
        self.assertEqual(
            producer.glibc_requirements("GLIBC_2.34 GLIBC_2.2.5 GLIBC_2.36"),
            ["2.2.5", "2.34", "2.36"],
        )
        for text in [
            "GLIBC_2.37",
            "GLIBC_2.36 GLIBC_PRIVATE",
            "GLIBC_2.36 GLIBC_ABI_FUTURE",
            "no version information",
        ]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                producer.glibc_requirements(text)
        base = "(NEEDED) Shared library: [libgcc_s.so.1]\n(NEEDED) Shared library: [libc.so.6]\n"
        self.assertEqual(
            producer.needed_libraries(base), ["libc.so.6", "libgcc_s.so.1"]
        )
        for text in [
            base + "(NEEDED) Shared library: [libssl.so.3]",
            base + base,
            "(NEEDED) Shared library: [libc.so.6]",
        ]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                producer.needed_libraries(text)

    def test_published_source_binary_and_manifest_mutations_refuse(self):
        source = (producer.ROOT / producer.PROBE).read_bytes()
        manifest = producer.supply_manifest("a" * 40, source, elf())
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = {
                "manifest.json": (
                    json.dumps(manifest, indent=2, sort_keys=True) + "\n"
                ).encode(),
                "combined_read_probe": elf(),
                "combined_read_probe.rs": source,
            }
            for name, data in original.items():
                (root / name).write_bytes(data)
            producer.verify_supply(root, manifest)
            for name, data in original.items():
                (root / name).write_bytes(data + b"changed")
                with self.subTest(name=name), self.assertRaises(ValueError):
                    producer.verify_supply(root, manifest)
                (root / name).write_bytes(data)
            (root / "combined_read_probe").unlink()
            (root / "combined_read_probe").symlink_to(root / "combined_read_probe.rs")
            with self.assertRaises(ValueError):
                producer.verify_supply(root, manifest)


if __name__ == "__main__":
    unittest.main()
