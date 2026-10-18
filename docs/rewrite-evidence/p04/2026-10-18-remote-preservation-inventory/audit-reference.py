# /// script
# dependencies = []
# ///
"""Bind the remote preservation matrix to frozen source without executing it."""

import ast
import gzip
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = next(
    parent
    for parent in Path(__file__).resolve().parents
    if (parent / "Cargo.toml").is_file() and (parent / ".git").exists()
)
REFERENCE = Path("/home/marc/Documents/github/pi-health")
PACKET = ROOT / "docs/rewrite-evidence/p04/2026-10-18-remote-preservation-inventory"
FILES = {
    "base": "storage_plugins/remote_base.py",
    "sshfs": "storage_plugins/sshfs_plugin.py",
    "rclone": "storage_plugins/rclone_plugin.py",
    "seed": "seedbox_service.py",
    "helper": "pihealth_helper.py",
    "api": "storage_plugins/__init__.py",
    "disk": "disk_manager.py",
    "manager": "plugin_manager.py",
    "registry": "storage_plugins/registry.py",
    "adapter": "storage_capability_adapters.py",
    "capability": "capability_registry_service.py",
    "paths": "runtime_paths.py",
    "ui": "frontend/src/lib/mounts.ts",
    "page": "frontend/src/pages/mounts-page.tsx",
}
EXTRA = [
    "tests/test_sshfs_plugin.py",
    "tests/test_rclone_plugin.py",
    "tests/test_seedbox_service.py",
    "tests/e2e/test_v2_mounts_parity.py",
]


def git(*args: str, root: Path = ROOT) -> str:
    return subprocess.check_output(["git", *args], cwd=root, text=True).strip()


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write_json(name: str, data: Any) -> None:
    (PACKET / name).write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")


def index(tree: ast.Module) -> dict[str, ast.AST]:
    found: dict[str, ast.AST] = {}
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)):
            found[node.name] = node
            if isinstance(node, ast.ClassDef):
                for child in node.body:
                    if isinstance(child, ast.FunctionDef):
                        found[f"{node.name}.{child.name}"] = child
    return found


def returned_literal(node: ast.AST) -> Any:
    returns = [child for child in ast.walk(node) if isinstance(child, ast.Return)]
    assert len(returns) == 1
    assert returns[0].value is not None
    return ast.literal_eval(returns[0].value)


def dictionary_fields(node: ast.Dict) -> dict[str, str]:
    return {
        str(ast.literal_eval(key)): ast.unparse(value)
        for key, value in zip(node.keys, node.values, strict=True)
        if key is not None
    }


def assigned_dictionary(node: ast.AST, name: str) -> dict[str, str]:
    matches = [
        child.value
        for child in ast.walk(node)
        if isinstance(child, ast.Assign)
        and any(isinstance(t, ast.Name) and t.id == name for t in child.targets)
        and isinstance(child.value, ast.Dict)
    ]
    assert len(matches) == 1
    return dictionary_fields(matches[0])


def get_calls(node: ast.AST, variable: str) -> list[dict[str, str]]:
    calls = []
    for child in ast.walk(node):
        if (
            isinstance(child, ast.Call)
            and isinstance(child.func, ast.Attribute)
            and isinstance(child.func.value, ast.Name)
            and child.func.value.id == variable
            and child.func.attr == "get"
            and child.args
            and isinstance(child.args[0], ast.Constant)
        ):
            calls.append(
                {
                    "key": str(child.args[0].value),
                    "default_expression": ast.unparse(child.args[1])
                    if len(child.args) > 1
                    else "None",
                }
            )
    return calls


assert git("rev-parse", "HEAD", root=REFERENCE) == (
    "80593b29443ea9b81eb60ed4e5c3c3ac7a236204"
)
assert not git(
    "diff", "--name-only", "HEAD", "--", *FILES.values(), *EXTRA, root=REFERENCE
)
assert not git(
    "diff",
    "--name-only",
    "8b31f990103bdc7ce959048abba1b6c051fc847b",
    "--",
    "bins",
    "crates",
    "frontend",
    "tests",
    "Cargo.toml",
    "Cargo.lock",
)
inventory = json.loads((PACKET / "inventory.json").read_text())
rows = inventory["rows"]
assert [row["id"] for row in rows] == [f"RM-{i:03}" for i in range(1, 31)]
assert not inventory["retirements_accepted"]
assert all(row["status"] == "unimplemented_unqualified" for row in rows)
texts = {alias: (REFERENCE / path).read_text() for alias, path in FILES.items()}
trees = {
    alias: ast.parse(text)
    for alias, text in texts.items()
    if FILES[alias].endswith(".py")
}
symbols = {alias: index(tree) for alias, tree in trees.items()}
references: dict[str, list[str]] = {}
for row in rows:
    assert row["intent"] and row["frozen"] and row["rewrite"] and row["acceptance"]
    for source in row["sources"]:
        references.setdefault(source, []).append(row["id"])

anchors = {}
for source, owners in sorted(references.items()):
    alias, symbol = source.split(":", 1)
    path = FILES[alias]
    lines = texts[alias].splitlines(keepends=True)
    if symbol == "@file":
        start, end = 1, len(lines)
    else:
        node = symbols[alias][symbol]
        assert isinstance(node, (ast.FunctionDef, ast.ClassDef))
        start = min([node.lineno, *[d.lineno for d in node.decorator_list]])
        end = node.end_lineno
        assert end is not None
    anchors[source] = {
        "file": path,
        "line": start,
        "end_line": end,
        "source_sha256": sha("".join(lines[start - 1 : end]).encode()),
        "file_sha256": sha((REFERENCE / path).read_bytes()),
        "rows": owners,
    }

required = set()
for alias, cls in [
    ("base", "RemoteMountPlugin"),
    ("sshfs", "SSHFSPlugin"),
    ("rclone", "RclonePlugin"),
    ("seed", "SeedboxService"),
]:
    for symbol, node in symbols[alias].items():
        if not symbol.startswith(cls + ".") or symbol.split(".")[-1].startswith("_"):
            continue
        assert isinstance(node, ast.FunctionDef)
        if any(ast.unparse(d) == "abstractmethod" for d in node.decorator_list):
            continue
        required.add(f"{alias}:{symbol}")
required.update(
    "helper:" + name
    for name in symbols["helper"]
    if name.startswith(("cmd_sshfs_", "cmd_rclone_", "cmd_seedbox_"))
)
routes = []
for alias in ["api", "disk"]:
    for name, node in symbols[alias].items():
        if not isinstance(node, ast.FunctionDef):
            continue
        for decorator in node.decorator_list:
            if not (
                isinstance(decorator, ast.Call)
                and isinstance(decorator.func, ast.Attribute)
                and decorator.func.attr == "route"
                and decorator.args
            ):
                continue
            path = ast.literal_eval(decorator.args[0])
            if not path.startswith(("/api/storage/mounts/", "/api/disks/seedbox")):
                continue
            methods = next(k.value for k in decorator.keywords if k.arg == "methods")
            source = f"{alias}:{name}"
            required.add(source)
            routes.append(
                {
                    "source": source,
                    "path": path,
                    "methods": ast.literal_eval(methods),
                    "declared_decorators": [
                        ast.unparse(d) for d in node.decorator_list
                    ],
                    "proof": "declarations only; no authentication or installed route proof",
                }
            )
missing = sorted(required - references.keys())
assert not missing, f"Unmapped public surface: {missing}"
assert len(routes) == 10

dispatch = {}
for node in ast.walk(trees["helper"]):
    if isinstance(node, ast.Dict):
        for key, value in zip(node.keys, node.values, strict=True):
            if (
                isinstance(key, ast.Constant)
                and isinstance(key.value, str)
                and key.value.startswith(("sshfs_", "rclone_", "seedbox_"))
                and isinstance(value, ast.Name)
                and value.id.startswith("cmd_")
            ):
                dispatch[key.value] = value.id
assert set(dispatch.values()) == {
    name
    for name in symbols["helper"]
    if name.startswith(("cmd_sshfs_", "cmd_rclone_", "cmd_seedbox_"))
}

examples = json.loads((PACKET / "reconciliation-cases.json").read_text())
assert len(examples["cases"]) == 7
assert len({case["id"] for case in examples["cases"]}) == 7
for case in examples["cases"]:
    assert set(case["rows"]) <= {row["id"] for row in rows}
    assert case["required_findings"] and case["expected_readiness"]

schemas = {
    alias: returned_literal(symbols[alias][cls + ".get_schema"])
    for alias, cls in [("sshfs", "SSHFSPlugin"), ("rclone", "RclonePlugin")]
}
sshfs_record = assigned_dictionary(
    symbols["helper"]["cmd_sshfs_configure"], "mount_config"
)
rclone_appends = [
    n.args[0]
    for n in ast.walk(symbols["helper"]["cmd_rclone_configure"])
    if isinstance(n, ast.Call)
    and isinstance(n.func, ast.Attribute)
    and isinstance(n.func.value, ast.Name)
    and n.func.value.id == "mounts"
    and n.func.attr == "append"
    and n.args
    and isinstance(n.args[0], ast.Dict)
]
assert len(rclone_appends) == 1
rclone_record = dictionary_fields(rclone_appends[0])
assert not {"auth_type", "ssh_key_path"} & sshfs_record.keys()
assert not {"options", "access_key_id", "secret_access_key"} & rclone_record.keys()
seed_default = next(
    n.value
    for n in ast.walk(symbols["seed"]["SeedboxService._default_config"])
    if isinstance(n, ast.Return) and isinstance(n.value, ast.Dict)
)
defaults = next(
    n.value
    for n in trees["manager"].body
    if isinstance(n, ast.Assign)
    and any(isinstance(t, ast.Name) and t.id == "BUILTIN_DEFAULTS" for t in n.targets)
)
constants = {}
for node in trees["helper"].body:
    if isinstance(node, ast.Assign):
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id.startswith(
                ("SEEDBOX_", "RCLONE_", "SSHFS_")
            ):
                constants[target.id] = ast.literal_eval(node.value)
write_json(
    "source-extraction.json",
    {
        "method": "AST/literal data only; no frozen imports, helpers or tests executed",
        "schemas": schemas,
        "helper_saved_fields_and_expressions": {
            "sshfs": sshfs_record,
            "rclone": rclone_record,
        },
        "seedbox_default_field_expressions": dictionary_fields(seed_default),
        "provider_defaults": ast.literal_eval(defaults),
        "helper_constants": constants,
        "helper_get_calls": {
            family: {
                "params": get_calls(
                    symbols["helper"][f"cmd_{family}_configure"], "params"
                ),
                "options": get_calls(
                    symbols["helper"][f"cmd_{family}_configure"], "options"
                ),
            }
            for family in ["sshfs", "rclone", "seedbox"]
        },
        "routes": routes,
        "helper_dispatch_declarations": dispatch,
    },
)
write_json("anchors.json", anchors)
write_json(
    "coverage.json",
    {
        "matrix_rows": 30,
        "required_public_surface_count": len(required),
        "required_public_surface": sorted(required),
        "resolved_anchor_count": len(anchors),
        "unmapped_required_surface": missing,
        "retirements_accepted": [],
        "registered_rust_remote_operations": 0,
        "installed_acceptance_credit": 0,
        "functional_tests_executed": 0,
        "limitation": "static surface coverage cannot prove row prose or runtime correctness; deployed usage not collected",
    },
)
(PACKET / "coverage.md").write_text(
    "# Frozen remote public-surface coverage\n\n"
    "Static declarations only; every required symbol maps to at least one preservation row.\n"
    "No frozen code was imported/executed; this is not runtime/API/authentication proof.\n\n"
    "| Symbol | Source | Rows |\n| --- | --- | --- |\n"
    + "".join(
        f"| `{source}` | [{anchor['file']}:{anchor['line']}]({REFERENCE / anchor['file']}:{anchor['line']}) | {', '.join(anchor['rows'])} |\n"
        for source, anchor in sorted(anchors.items())
    )
)
(PACKET / "REFERENCE_INPUTS.sha256").write_text(
    "".join(
        f"{sha((REFERENCE / path).read_bytes())}  {path}\n"
        for path in sorted([*FILES.values(), *EXTRA])
    )
)
plans = [
    "docs/plans/2026-10-14-engineer-p04-storage-backup-runtime.md",
    "docs/plans/2026-10-14-p04-parallel-delivery.md",
    "docs/plans/2026-10-04-rust-rewrite-roadmap.md",
    "docs/plans/2026-10-04-rust-rewrite-defect-register.md",
]
(PACKET / "CONTEXT_INPUTS.sha256").write_text(
    "".join(f"{sha((ROOT / path).read_bytes())}  {path}\n" for path in plans)
)
write_json(
    "manifest.json",
    {
        "inventory_capture_head": git("rev-parse", "HEAD"),
        "tested_product_source": "8b31f990103bdc7ce959048abba1b6c051fc847b",
        "frozen_source": inventory["frozen_source"],
        "reference_input_count": len(FILES) + len(EXTRA),
        "structural_checks": "30 unique rows; all anchors resolved; complete selected public methods/commands/routes; 12 dispatch entries; saved-field omissions; 10 declared routes; 7 synthetic example references",
        "functional_tests": 0,
        "python": sys.version,
        "uv": subprocess.check_output(["uv", "--version"], text=True).strip(),
        "scope": "A3b inventory/contract input only; no registered remote implementation or installed acceptance",
    },
)
print(
    f"Mapped {len(required)} public symbols through {len(anchors)} anchors to 30 rows; no frozen execution"
)
sys.stdout.flush()
(PACKET / "audit-reference.py").write_bytes(Path(__file__).read_bytes())
for log in (ROOT / ".cache/a-assistance/remote-inventory").glob("*.log"):
    (PACKET / (log.name + ".gz")).write_bytes(gzip.compress(log.read_bytes(), mtime=0))
(PACKET / "SHA256SUMS").write_text(
    "".join(
        f"{sha(path.read_bytes())}  {path.name}\n"
        for path in sorted(PACKET.iterdir())
        if path.is_file() and path.name != "SHA256SUMS"
    )
)
