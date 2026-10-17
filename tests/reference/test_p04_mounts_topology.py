"""Kernel-shaped sysfs fixtures; no block device, guest or socket is opened."""

import importlib.util
import os
import tempfile
import unittest
from pathlib import Path

SOURCE = Path(__file__).resolve().parents[1] / "privileged_vm/p04_mounts_guest.py"
SPEC = importlib.util.spec_from_file_location("a1_topology_fixture", SOURCE)
guest = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guest)


class BlockTopologyTests(unittest.TestCase):
    def setUp(self):
        temporary_root = os.environ.get("TMPDIR")
        if temporary_root is None:
            temporary_root = SOURCE.parents[2] / ".cache/a1-topology-tests"
            temporary_root.mkdir(parents=True, exist_ok=True)
        directory = tempfile.TemporaryDirectory(dir=temporary_root)
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.sys = self.root / "sys"
        (self.sys / "dev/block").mkdir(parents=True)
        self.parent = self.node("252:0", self.sys / "devices/vde")
        self.partition = self.node("252:1", self.parent / "vde1", partition="1")

    def node(self, device, path, partition=None):
        path.mkdir(parents=True)
        (path / "dev").write_text(device + "\n")
        (path / "holders").mkdir()
        if partition is None:
            (path / "slaves").mkdir()
        else:
            (path / "partition").write_text(partition + "\n")
        (self.sys / "dev/block" / device).symlink_to(path)
        return path

    def family(self, device="252:1"):
        return guest.block_family(device, self.sys)

    def test_partition_without_slaves_keeps_parent(self):
        self.assertFalse((self.partition / "slaves").exists())
        self.assertEqual(self.family(), {"252:0", "252:1"})

    def test_partitions_keep_shared_parent_for_boot_swap_intersection(self):
        self.node("252:2", self.parent / "vde2", partition="2")
        self.assertEqual(self.family() & self.family("252:2"), {"252:0"})

    def test_missing_whole_device_or_holder_topology_refuses(self):
        for path in (
            self.parent / "slaves",
            self.parent / "holders",
            self.partition / "holders",
        ):
            with self.subTest(path=path.name, node=path.parent.name):
                path.rmdir()
                with self.assertRaises((OSError, ValueError)):
                    self.family()
                path.mkdir()

    def test_missing_partition_marker_cannot_authorize_absent_slaves(self):
        (self.partition / "partition").unlink()
        with self.assertRaises((OSError, ValueError)):
            self.family()

    def test_parent_identity_and_mapping_must_be_complete_and_exact(self):
        identity = self.parent / "dev"
        identity.unlink()
        with self.assertRaises((OSError, ValueError)):
            self.family()
        identity.write_text("252:0\n")
        mapping = self.sys / "dev/block/252:0"
        mapping.unlink()
        with self.assertRaises((OSError, ValueError)):
            self.family()
        other = self.sys / "devices/other"
        other.mkdir()
        (other / "dev").write_text("252:0\n")
        mapping.symlink_to(other)
        with self.assertRaises((OSError, ValueError)):
            self.family()
        mapping.unlink()
        mapping.symlink_to(self.parent)
        (self.parent / "partition").write_text("1\n")
        with self.assertRaises((OSError, ValueError)):
            self.family()

    def test_malformed_partition_and_parent_attributes_refuse(self):
        marker = self.partition / "partition"
        for value in ("0", "01", "unknown", "1" * 33):
            with self.subTest(partition=value):
                marker.write_text(value)
                with self.assertRaises((OSError, ValueError)):
                    self.family()
        marker.unlink()
        marker.symlink_to(self.parent / "dev")
        with self.assertRaises((OSError, ValueError)):
            self.family()
        marker.unlink()
        marker.write_text("1\n")
        for value in ("252:1", "unknown", "2" * 33 + ":0"):
            with self.subTest(parent=value):
                (self.parent / "dev").write_text(value)
                with self.assertRaises((OSError, ValueError)):
                    self.family()

    def test_existing_partition_slaves_are_still_walked(self):
        backing = self.node("253:0", self.sys / "devices/backing")
        (self.partition / "slaves").mkdir()
        (self.partition / "slaves/backing").symlink_to(backing)
        self.assertEqual(self.family(), {"252:0", "252:1", "253:0"})

    def test_dangling_slaves_and_escaping_device_refuse(self):
        slaves = self.partition / "slaves"
        slaves.symlink_to(self.partition / "missing")
        with self.assertRaises((OSError, ValueError)):
            self.family()
        slaves.unlink()
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "dev").write_text("253:0\n")
        (self.sys / "dev/block/253:0").symlink_to(outside)
        (self.parent / "slaves/outside").symlink_to(outside)
        with self.assertRaises((OSError, ValueError)):
            self.family()

    def test_relationship_and_graph_size_limits_remain_enforced(self):
        for index in range(9):
            (self.parent / f"holders/unknown-{index}").symlink_to(self.root / "absent")
        with self.assertRaisesRegex(ValueError, "ambiguous block relationships"):
            self.family("252:0")
        for item in (self.parent / "holders").iterdir():
            item.unlink()
        previous = self.parent
        for index in range(1, 18):
            current = self.node(f"253:{index}", self.sys / f"devices/disk{index}")
            (previous / "slaves/next").symlink_to(current)
            previous = current
        with self.assertRaisesRegex(ValueError, "block ancestry exceeds"):
            self.family("252:0")


if __name__ == "__main__":
    unittest.main()
