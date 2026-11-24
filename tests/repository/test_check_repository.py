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

    def test_text_that_is_not_code_declares_nothing(self):
        lib = "\n".join([
            "//! mod inner_doc;",
            "/*",
            "mod in_block;",
            "/* nested */",
            "mod after_nested;",
            "*/",
            "/// mod outer_doc;",
            "const S: &str = \"",
            "mod in_string;",
            "\";",
            "const R: &str = r#\"",
            "mod in_raw;",
            "\"#;",
            "const R2: &str = r##\"quote \"# then",
            "mod in_raw_hashes;",
            "\"##;",
            "const B: &[u8] = b\"\\",
            "mod in_bytes;\";",
            "const BR: &[u8] = br#\"",
            "mod in_raw_bytes;",
            "\"#;",
            # Literals that a naive stripper misreads, each followed by a real module.
            "const Q: char = '\"';",
            "mod after_quote_char;",
            "const E: char = '\\'';",
            "mod after_escaped_char;",
            "fn f<'a>(x: &'a str) -> &'a str { x }",
            "mod after_lifetime;",
            "const O: &str = \"/*\";",
            "mod after_comment_opener_in_string;",
            "",
        ])
        decoys = ["inner_doc", "in_block", "after_nested", "outer_doc", "in_string", "in_raw",
                  "in_raw_hashes", "in_bytes", "in_raw_bytes"]
        real = ["after_quote_char", "after_escaped_char", "after_lifetime",
                "after_comment_opener_in_string"]
        files = {"src/lib.rs": lib}
        files.update({f"src/{name}.rs": "" for name in decoys + real})
        root = self.crate(files)
        self.assertEqual(sorted(self.stray(root)), sorted(f"src/{name}.rs" for name in decoys))

    def test_lib_path_replaces_the_conventional_root(self):
        root = self.crate({
            "src/lib.rs": "mod leftover_child;\n",
            "src/leftover_child.rs": "",
            "src/real.rs": "mod part;\n",
            "src/part.rs": "",
        })
        (root / "Cargo.toml").write_text('[package]\nname = "x"\n[lib]\npath = "src/real.rs"\n')
        self.assertEqual(sorted(self.stray(root)), ["src/leftover_child.rs", "src/lib.rs"])

    def test_test_and_build_roots_reach_into_src(self):
        root = self.crate({
            "src/lib.rs": "",
            "tests/it.rs": '#[path = "../src/test_helper.rs"]\nmod helper;\n',
            "src/test_helper.rs": "",
            "build.rs": '#[path = "src/build_helper.rs"]\nmod helper;\n',
            "src/build_helper.rs": "",
            "examples/demo.rs": '#[path = "../src/example_helper.rs"]\nmod helper;\n',
            "src/example_helper.rs": "",
        })
        self.assertEqual(self.stray(root), [])

    def test_manifest_overrides_and_disables_discovery(self):
        root = self.crate({
            "src/main.rs": "mod only_from_main;\n",
            "src/only_from_main.rs": "",
            "src/entry.rs": "",
            "src/bin/tool.rs": "",
            "src/bin/multi/main.rs": "mod sub;\n",
            "src/bin/multi/sub.rs": "",
            "tests/named.rs": '#[path = "../src/named_helper.rs"]\nmod helper;\n',
            "src/named_helper.rs": "",
            "tools/gen.rs": '#[path = "../src/gen_helper.rs"]\nmod helper;\n',
            "src/gen_helper.rs": "",
        })
        (root / "Cargo.toml").write_text(
            '[package]\nname = "x"\nautobins = false\nautotests = false\nbuild = "tools/gen.rs"\n'
            '[[bin]]\nname = "x"\npath = "src/entry.rs"\n'
            '[[bin]]\nname = "multi"\n'
            '[[test]]\nname = "named"\n'
        )
        # `x` names src/entry.rs, so src/main.rs is no root; autobins = false
        # drops src/bin/tool.rs; `multi` and `named` resolve by name.
        self.assertEqual(
            sorted(self.stray(root)), ["src/bin/tool.rs", "src/main.rs", "src/only_from_main.rs"]
        )

    def test_include_follows_literal_paths_only(self):
        root = self.crate({
            "src/lib.rs": (
                'include!("generated.rs");\n'
                'include!(concat!(env!("OUT_DIR"), "/bindings.rs"));\n'
                'const DATA: &str = include_str!("data.rs");\n'
                "mod nested {\n    std::include!(\"nested_part.rs\");\n}\n"
            ),
            # An included file resolves its own `mod` next to itself.
            "src/generated.rs": "mod from_generated;\n",
            "src/from_generated.rs": "",
            "src/nested_part.rs": "",
            "src/data.rs": "",
        })
        # include_str! reads data; the file is no module and is reported.
        self.assertEqual(self.stray(root), ["src/data.rs"])

    def test_path_attribute_forms(self):
        root = self.crate({
            "src/lib.rs": (
                '#[cfg_attr(feature = "alt", path = "alt/imp.rs")]\nmod imp;\n'
                '#[cfg_attr(unix, allow(dead_code), cfg_attr(test, path = "deep.rs"))]\nmod shallow;\n'
                '#[path = "renamed"]\nmod inline {\n    mod nested;\n    #[path = "x.rs"]\n    mod pathed;\n}\n'
                '#[path =\n    "multi_line.rs"]\npub(in crate) mod multi;\n'
                '#[path = "dir/file.rs"]\nmod file;\n'
                "#[path = r\"raw.rs\"]\nmod raw;\n"
                "mod r#match;\n"
            ),
            "src/alt/imp.rs": "",
            "src/imp.rs": "",
            "src/deep.rs": "",
            "src/shallow.rs": "",
            "src/renamed/nested.rs": "",
            "src/renamed/x.rs": "",
            "src/multi_line.rs": "",
            # A #[path] file is treated as a mod.rs: its `mod` is a sibling.
            "src/dir/file.rs": "mod sibling;\n",
            "src/dir/sibling.rs": "",
            "src/raw.rs": "",
            "src/match.rs": "",
            "src/a.rs": '#[path = "top.rs"]\nmod top;\n#[path = "in_a"]\nmod block {\n    mod leaf;\n}\n',
        })
        (root / "src/lib.rs").write_text((root / "src/lib.rs").read_text() + "mod a;\n")
        (root / "src/top.rs").write_text("")
        (root / "src/in_a").mkdir()
        (root / "src/in_a/leaf.rs").write_text("")
        self.assertEqual(self.stray(root), [])

    def test_unsupported_forms_fail_loudly(self):
        for text in [
            '#[cfg_attr(unix, path = "elsewhere")]\nmod inline {\n    mod nested;\n}\n',
            'macro_rules! declare { ($name:ident) => { mod $name; } }\n',
            '#[path = concat!("a", ".rs")]\nmod a;\n',
            "/* unterminated\nmod a;\n",
            'const S: &str = "unterminated;\n',
        ]:
            root = self.crate({"src/lib.rs": text, "src/a.rs": ""})
            with self.assertRaises(check.UnsupportedSource, msg=text):
                self.stray(root)

    def test_repository_has_no_stray_sources(self):
        for manifest in check.workspace_manifests()[1:]:
            self.assertEqual(check.unreachable_sources(manifest), [], manifest)


if __name__ == "__main__":
    unittest.main()
