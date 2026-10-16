"""Current schema expectations come from source, never installed DB state."""

import importlib
import runpy
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests/privileged_vm"
with patch.object(sys, "path", [str(FIXTURES), *sys.path]):
    schema = importlib.import_module("current_authority_schema")


class CurrentAuthoritySchemaTests(unittest.TestCase):
    def temporary(self):
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        return tempfile.TemporaryDirectory(dir=cache)

    def test_source_versions_are_read_without_relabeling_historical_sources(self):
        with self.temporary() as directory:
            source = Path(directory) / "lib.rs"
            for version in (6, 7, 8, 9, 10):
                with self.subTest(version=version):
                    text = f"pub const SCHEMA_VERSION: u32 = {version};\n"
                    source.write_text(text)
                    self.assertEqual(schema.source_authority_schema(source), version)
                    self.assertEqual(source.read_text(), text)

    def test_missing_ambiguous_malformed_and_oversized_source_refuse(self):
        with self.temporary() as directory:
            source = Path(directory) / "lib.rs"
            with self.assertRaises(FileNotFoundError):
                schema.source_authority_schema(source)
            invalid = [
                b"",
                b"pub const SCHEMA_VERSION: u32 = 0;",
                b"pub const SCHEMA_VERSION: u32 = 09;",
                b"pub const SCHEMA_VERSION: u32 = -1;",
                b"pub const SCHEMA_VERSION: u32 = 4294967296;",
                b"pub const SCHEMA_VERSION: u32 = true;",
                b"pub const SCHEMA_VERSION: u64 = 10;",
                b"pub const SCHEMA_VERSION: u32 = 10; // guessed",
                b"const SCHEMA_VERSION: u32 = 10;",
                b"pub const SCHEMA_VERSION: u32 = 10;\npub const SCHEMA_VERSION: u32 = 10;",
                b"pub const SCHEMA_VERSION: u32 = 10;\npub(crate) const SCHEMA_VERSION: u32 = 9;",
                b"pub const SCHEMA_VERSION: u32 = 10;\n"
                + b" " * schema.MAX_SOURCE_BYTES,
            ]
            for text in invalid:
                with self.subTest(source=text[:100]):
                    source.write_bytes(text)
                    with self.assertRaises(ValueError):
                        schema.source_authority_schema(source)

    def test_layout_selects_one_source_and_invalid_input_has_no_fallback(self):
        with self.temporary() as directory:
            root = Path(directory)
            checkout = root / "checkout/tests/privileged_vm"
            checkout.mkdir(parents=True)
            source = root / "checkout/crates/persistence/src/lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text("pub const SCHEMA_VERSION: u32 = 10;\n")
            (checkout / schema.SOURCE_NAME).write_text(
                "pub const SCHEMA_VERSION: u32 = 9;\n"
            )
            with patch.object(schema, "__file__", str(checkout / "helper.py")):
                self.assertEqual(schema.current_authority_schema(), 10)
                source.write_text("malformed")
                with self.assertRaises(ValueError):
                    schema.current_authority_schema()
            guest = root / "guest"
            guest.mkdir()
            delivered = guest / schema.SOURCE_NAME
            delivered.write_text("pub const SCHEMA_VERSION: u32 = 10;\n")
            with patch.object(schema, "__file__", str(guest / "helper.py")):
                self.assertEqual(schema.current_authority_schema(), 10)
                delivered.unlink()
                with self.assertRaises(FileNotFoundError):
                    schema.current_authority_schema()

    def test_reference_wrapper_uses_source_derived_schema_once(self):
        current = Mock(return_value=10)
        reference = SimpleNamespace(main=Mock())
        with patch.dict(
            sys.modules,
            {
                "current_authority_schema": SimpleNamespace(
                    current_authority_schema=current
                ),
                "p03_reference_guest": reference,
            },
        ):
            runpy.run_path(str(FIXTURES / "ci_reference_guest.py"), run_name="__main__")
        current.assert_called_once_with()
        reference.main.assert_called_once_with(
            package_version="0.4.4", authority_schema=10, qualify_upgrade=False
        )

    def test_container_wrapper_passes_explicit_manifest_schema_without_source_lookup(
        self,
    ):
        fixture = SimpleNamespace(
            main=Mock(side_effect=InterruptedError("stop fixture"))
        )
        current = Mock(side_effect=AssertionError("manifest must be explicit"))
        with patch.dict(
            sys.modules,
            {
                "current_authority_schema": SimpleNamespace(
                    current_authority_schema=current
                ),
                "p03_lifecycle_guest": fixture,
            },
        ):
            wrapper = runpy.run_path(str(FIXTURES / "p04_approved_container_guest.py"))
            with self.assertRaises(InterruptedError):
                wrapper["main"](package_version="candidate", authority_schema=9)
        current.assert_not_called()
        fixture.main.assert_called_once_with(
            package_version="candidate", authority_schema=9, qualify_upgrade=False
        )

    def test_current_storage_upgrade_keeps_original_schema7_artifact_identity(self):
        current = Mock(return_value=10)
        upgrade = Mock(side_effect=InterruptedError("stop before guest effects"))
        modules = {
            "current_authority_schema": SimpleNamespace(
                current_authority_schema=current
            ),
            "p03_guest": SimpleNamespace(eventually=Mock()),
            "p04_locks_guest": SimpleNamespace(upgrade=upgrade),
            "p04_planning_guest": SimpleNamespace(run=Mock(), http=Mock()),
            "p04_storage_guest": SimpleNamespace(),
            "p04_targets_guest": SimpleNamespace(EXECUTOR="fixture"),
            "p04_targets_package_guest": SimpleNamespace(),
        }
        with (
            patch.dict(sys.modules, modules),
            patch.object(sys, "argv", ["fixture", "candidate-repo", "result.json"]),
            patch("os.geteuid", return_value=0),
            patch.object(Path, "read_text", return_value="limeos-p01-test\n"),
        ):
            wrapper = runpy.run_path(str(FIXTURES / "p04_approved_guest.py"))
            with self.assertRaises(InterruptedError):
                wrapper["main"]()
        current.assert_called_once_with()
        upgrade.assert_called_once_with(
            Path("candidate-repo"),
            previous_version="0.4.3",
            previous_schema=7,
            authority_schema=10,
            previous_package_sha256="c5e49b2117a19d0983b5afb9a3ea756f479c2ab9dc67be7001758265737204b6",
            previous_core_sha256="99c2c8a2a19919e093476e33fbaae9fc0d4da8559bfa8b5a2151135b23243c33",
        )

    def test_current_dependencies_pass_source_schema_to_lifecycle(self):
        current = Mock(return_value=10)
        lifecycle = SimpleNamespace(
            main=Mock(side_effect=InterruptedError("stop before guest effects"))
        )
        with (
            patch.dict(
                sys.modules,
                {
                    "current_authority_schema": SimpleNamespace(
                        current_authority_schema=current
                    ),
                    "p03_guest": SimpleNamespace(
                        Proxy=SimpleNamespace(do_GET=Mock()), CORE_DB="fixture"
                    ),
                    "p03_lifecycle_guest": lifecycle,
                },
            ),
            patch("os.getuid", return_value=0),
            patch("socket.gethostname", return_value="limeos-p01-test"),
        ):
            wrapper = runpy.run_path(str(FIXTURES / "p04_dependencies_guest.py"))
            with self.assertRaises(InterruptedError):
                wrapper["main"](package_version="candidate")
        current.assert_called_once_with()
        lifecycle.main.assert_called_once_with(
            package_version="candidate", authority_schema=10, qualify_upgrade=False
        )

    def test_runner_delivers_checkout_then_actual_bundle_source_before_guest(self):
        text = (FIXTURES / "run.py").read_text()
        delivery = text.index('str(ROOT / "crates/persistence/src/lib.rs")')
        bundle = text.index(
            "install -m 0644 /root/build/source/crates/persistence/src/lib.rs"
        )
        invocation = text.index('shlex.quote("/root/" + args.guest_script.name)')
        self.assertLess(delivery, bundle)
        self.assertLess(bundle, invocation)
        self.assertIn('"root@127.0.0.1:/root/" + SOURCE_NAME', text)


if __name__ == "__main__":
    unittest.main()
