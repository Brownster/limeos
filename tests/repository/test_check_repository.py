"""Unit tests for scripts/check_repository.py: deferred features and module reachability."""

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[2] / "scripts/check_repository.py"
spec = importlib.util.spec_from_file_location("check_repository", SCRIPT)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


class DeferredFeatureTests(unittest.TestCase):
    def violations(self, config):
        return check.deferred_violations("crate", config)

    def test_own_deferred_feature_may_forward(self):
        config = {"features": {"deferred": ["dep:a", "a/deferred", "b?/deferred"]}}
        self.assertEqual(self.violations(config), [])

    def test_default_enabling_deferred_directly(self):
        self.assertTrue(self.violations({"features": {"default": ["deferred"], "deferred": []}}))

    def test_default_enabling_deferred_through_another_feature(self):
        config = {"features": {"default": ["x"], "x": ["y"], "y": ["deferred"], "deferred": []}}
        problems = self.violations(config)
        self.assertEqual(len(problems), 3)  # default, x and y all reach it
        self.assertTrue(any("'default'" in p for p in problems))

    def test_non_default_feature_forwarding_a_dependency_deferred(self):
        config = {"features": {"probe": ["limeos-domain/deferred"]}}
        self.assertTrue(self.violations(config))
        config = {"features": {"probe": ["extra"], "extra": ["limeos-domain?/deferred"]}}
        self.assertTrue(self.violations(config))

    def test_feature_cycles_terminate(self):
        config = {"features": {"a": ["b"], "b": ["a"]}}
        self.assertEqual(self.violations(config), [])

    def test_dependency_entries_in_every_section(self):
        entry = {"path": "../a", "features": ["deferred"]}
        for config in [
            {"dependencies": {"a": entry}},
            {"dev-dependencies": {"a": entry}},
            {"target": {"cfg(unix)": {"build-dependencies": {"a": entry}}}},
            {"workspace": {"dependencies": {"a": entry}}},
        ]:
            self.assertTrue(self.violations(config), config)

    def test_unrelated_names_are_not_matched(self):
        config = {"features": {"probe": ["dep:deferred-extra", "a/deferred-extra"]}}
        self.assertEqual(self.violations(config), [])


class ReachabilityTests(unittest.TestCase):
    def crate(self, files):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / "Cargo.toml").write_text('[package]\nname = "x"\n')
        for name, text in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return root

    def stray(self, root):
        return [p.relative_to(root.resolve()).as_posix() for p in check.unreachable_sources(root / "Cargo.toml")]

    def test_every_declared_form_is_reached(self):
        root = self.crate({
            "src/lib.rs": 'pub mod a;\nmod b;\n#[cfg(feature = "deferred")]\npub(crate) mod c;\n#[path = "elsewhere/d.rs"]\nmod d;\nmod inline {\n    mod e;\n}\n',
            "src/a.rs": "mod nested;\n",
            "src/a/nested.rs": "",
            "src/b/mod.rs": "mod child;\n",
            "src/b/child.rs": "",
            "src/c.rs": "#[cfg(test)]\n#[path = \"c_tests.rs\"]\nmod tests;\n",
            "src/c_tests.rs": "",
            "src/elsewhere/d.rs": "",
            "src/inline/e.rs": "",
        })
        self.assertEqual(self.stray(root), [])

    def test_undeclared_files_are_reported(self):
        root = self.crate({
            "src/lib.rs": "mod a;\n// mod commented;\n",
            "src/a.rs": "",
            "src/commented.rs": "",
            "src/orphan.rs": "mod inner;\n",
            "src/orphan/inner.rs": "",
        })
        self.assertEqual(
            sorted(self.stray(root)), ["src/commented.rs", "src/orphan.rs", "src/orphan/inner.rs"]
        )

    def test_manifest_target_paths_are_roots(self):
        root = self.crate({
            "src/main.rs": "",
            # A crate root resolves `mod` next to itself, like lib.rs.
            "src/worker.rs": "mod part;\n",
            "src/part.rs": "",
        })
        (root / "Cargo.toml").write_text('[package]\nname = "x"\n[[bin]]\nname = "w"\npath = "src/worker.rs"\n')
        self.assertEqual(self.stray(root), [])

    def test_repository_has_no_stray_sources(self):
        for manifest in check.workspace_manifests()[1:]:
            self.assertEqual(check.unreachable_sources(manifest), [], manifest)


if __name__ == "__main__":
    unittest.main()
