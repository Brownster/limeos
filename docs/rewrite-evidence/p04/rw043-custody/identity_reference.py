#!/usr/bin/env python3
"""Independent reference for the custody identity encoding, version 1.

Implements docs/p04-backup-archive-custody.md from its text alone, without the
Rust source. Given custody record.json files, it recomputes both identities
and checks them against the record's own binding. Given --policy JSON files,
it prints the policy identity. Exit 0 only when every record matches.
"""

import argparse
import hashlib
import json
import sys

FORMATS = {"tar_gzip": 1, "tar_zstd": 2}
KINDS = {"file": 1, "directory": 2}
POLICY_DOMAIN = "limeos.backup-custody.policy-identity.v1"
MANIFEST_DOMAIN = "limeos.backup-custody.manifest-identity.v1"


class Encoder:
    def __init__(self, domain):
        self.out = bytearray()
        self.str(domain)

    def u8(self, v):
        self.out += v.to_bytes(1, "big")

    def u32(self, v):
        self.out += v.to_bytes(4, "big")

    def u64(self, v):
        self.out += v.to_bytes(8, "big")

    def str(self, v):
        data = v.encode("utf-8")
        self.u64(len(data))
        self.out += data

    def digest(self):
        return hashlib.sha256(self.out).hexdigest(), len(self.out)


def policy_identity(p):
    e = Encoder(POLICY_DOMAIN)
    e.u64(p["revision"])
    e.u64(len(p["formats"]))
    for f in p["formats"]:
        e.u8(FORMATS[f])
    lim = p["limits"]
    for key, width in [
        ("max_compressed_bytes", 64), ("max_decompressed_bytes", 64), ("max_file_bytes", 64),
        ("max_entries", 64), ("max_path_bytes", 32), ("max_path_depth", 32),
        ("max_component_bytes", 32), ("max_metadata_bytes", 64),
        ("max_total_metadata_bytes", 64), ("max_zstd_window_log", 32),
    ]:
        (e.u64 if width == 64 else e.u32)(lim[key])
    e.u64(len(p["resources"]))
    for r in p["resources"]:
        e.str(r["id"])
        e.str(r["destination_root"])
    e.u64(len(p["legacy_mappings"]))
    for m in p["legacy_mappings"]:
        e.str(m["archive_prefix"])
        e.str(m["resource"])
        e.str(m["resource_prefix"])
    return e.digest()


def manifest_identity(m):
    e = Encoder(MANIFEST_DOMAIN)
    e.u32(m["manifest_version"])
    e.str(m["archive_sha256"])
    e.u8(FORMATS[m["format"]])
    for key in ["policy_revision", "compressed_bytes", "decompressed_bytes", "header_count",
                "coalesced_self_hardlinks", "file_count", "directory_count", "file_bytes"]:
        e.u64(m[key])
    e.u64(len(m["entries"]))
    for entry in m["entries"]:
        e.str(entry["resource"])
        e.str(entry["relative_path"])
        e.u8(KINDS[entry["kind"]])
        e.u64(entry["size"])
        if entry["sha256"] is None:
            e.u8(0)
        else:
            e.u8(1)
            e.str(entry["sha256"])
        e.u32(entry["archived_permissions"])
        e.str(entry["archive_path"])
    return e.digest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("records", nargs="*", help="custody record.json files")
    parser.add_argument("--policy", action="append", default=[], help="policy JSON files")
    args = parser.parse_args()
    ok = True
    for path in args.policy:
        with open(path, encoding="utf-8") as f:
            digest, size = policy_identity(json.load(f))
        print(json.dumps({"policy": path, "policy_identity": digest, "encoded_bytes": size}))
    for path in args.records:
        with open(path, encoding="utf-8") as f:
            record = json.load(f)
        policy, policy_bytes = policy_identity(record["policy"])
        manifest, manifest_bytes = manifest_identity(record["manifest"])
        binding = record["binding"]
        match = policy == binding["policy_identity"] and manifest == binding["manifest_identity"]
        ok &= match
        print(json.dumps({"record": path, "policy_identity": policy, "manifest_identity": manifest,
                          "policy_encoded_bytes": policy_bytes, "manifest_encoded_bytes": manifest_bytes,
                          "matches_binding": match}))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
