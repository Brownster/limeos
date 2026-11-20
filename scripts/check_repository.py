#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"Check dependency direction, deferred features, module reachability and build-output hygiene."

import re
import subprocess
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parents[1]
ALLOWED = {
    "domain": set(),
    "contracts": {"domain"},
    "policy": {"domain"},
    "identity": set(),
    "persistence": {"domain", "identity", "policy"},
    "api": {"domain", "contracts"},
    "executor-protocol": {"contracts", "domain"},
    "observations": {"contracts", "domain", "executor-protocol"},
    "executor-container": {"domain", "identity"},
    "executor-storage": {"domain", "identity"},
    "backup-archive": {"domain"},
}
DEPENDENCY_KINDS = ["dependencies", "dev-dependencies", "build-dependencies"]
# Source files deliberately outside every module graph, relative to the
# repository root, each with the reason. Keep this empty where possible.
UNREACHABLE_SOURCE_EXCEPTIONS: dict[str, str] = {}


def dependency_sections(config):
    """Every dependency table of a manifest: package, target-specific and workspace."""
    sections = [config, *config.get("target", {}).values()]
    for section in sections:
        for kind in DEPENDENCY_KINDS:
            yield from section.get(kind, {}).items()
    yield from config.get("workspace", {}).get("dependencies", {}).items()


def feature_closure(features, start):
    """All feature entries reachable from `start` within one manifest's [features]."""
    seen, stack = set(), [start]
    while stack:
        entry = stack.pop()
        if entry in seen:
            continue
        seen.add(entry)
        if entry in features:
            stack.extend(features[entry])
    return seen


def enables_deferred(entry):
    """True for `deferred`, `dep/deferred` and `dep?/deferred`."""
    return entry == "deferred" or entry.split("/", 1)[-1] == "deferred" and "/" in entry


def deferred_violations(name, config):
    """Deferred code must be reachable only through the crate's own `deferred` feature.

    No feature other than `deferred` may enable it, directly or transitively;
    `default` therefore cannot; and no dependency entry may turn it on.
    """
    problems = []
    features = config.get("features", {})
    for feature in features:
        if feature == "deferred":
            continue
        reached = sorted(e for e in feature_closure(features, feature) if enables_deferred(e))
        if reached:
            problems.append(f"{name}: feature {feature!r} enables deferred via {reached}")
    for dependency, details in dependency_sections(config):
        if isinstance(details, dict) and "deferred" in details.get("features", []):
            problems.append(f"{name}: dependency {dependency} enables deferred unconditionally")
    return problems


MOD = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(r#)?(?P<name>\w+)\s*(?P<end>[;{])"
)
PATH = re.compile(r'#\[path\s*=\s*"(?P<path>[^"]+)"\]')


def module_files(root):
    """Files reached from one crate root through `mod` declarations and `#[path]`."""
    reached, pending = set(), [root.resolve()]
    while pending:
        current = pending.pop()
        if current in reached or not current.is_file():
            continue
        reached.add(current)
        mod_rs = current.name in ("mod.rs", "lib.rs", "main.rs") or current == root.resolve()
        base = current.parent if mod_rs else current.parent / current.stem
        inline: list[tuple[str, int]] = []  # (name, brace depth at open)
        depth, path = 0, None
        for raw in current.read_text(errors="ignore").splitlines():
            line = raw.split("//", 1)[0]
            attribute = PATH.search(line)
            if attribute:
                path = attribute.group("path")
            match = MOD.match(line)
            if match:
                name, end = match.group("name"), match.group("end")
                directory = base.joinpath(*(n for n, _ in inline))
                if end == ";":
                    if path is not None:
                        anchor = current.parent if not inline else directory
                        pending.append((anchor / path).resolve())
                    else:
                        pending.extend([directory / f"{name}.rs", directory / name / "mod.rs"])
                else:
                    inline.append((name, depth))
                path = None
            elif line.strip() and not line.strip().startswith("#["):
                path = None
            depth += line.count("{") - line.count("}")
            while inline and depth <= inline[-1][1]:
                inline.pop()
    return reached


def crate_roots(manifest):
    crate = manifest.parent
    config = tomllib.loads(manifest.read_text())
    roots = [crate / "src/lib.rs", crate / "src/main.rs", *(crate / "src/bin").glob("*.rs")]
    for kind in ["bin", "lib", "test", "example", "bench"]:
        targets = config.get(kind, [])
        for target in targets if isinstance(targets, list) else [targets]:
            if "path" in target:
                roots.append(crate / target["path"])
    return [root for root in roots if root.is_file()]


def unreachable_sources(manifest):
    """`.rs` files under the crate's src/ that no crate root reaches."""
    src = (manifest.parent / "src").resolve()
    reached = set()
    for root in crate_roots(manifest):
        reached |= module_files(root)
    return sorted(p for p in src.rglob("*.rs") if p.resolve() not in reached)


def workspace_manifests():
    return [
        ROOT / "Cargo.toml",
        *sorted(ROOT.glob("crates/*/Cargo.toml")),
        *sorted(ROOT.glob("bins/*/Cargo.toml")),
        *sorted(ROOT.glob("spikes/*/Cargo.toml")),
    ]


def main():
    discovered = {
        manifest.parent.name for manifest in (ROOT / "crates").glob("*/Cargo.toml")
    }
    assert discovered == ALLOWED.keys(), (
        f"crate boundary registry mismatch: unregistered crates {sorted(discovered - ALLOWED.keys())}; "
        f"missing crates {sorted(ALLOWED.keys() - discovered)}"
    )
    for crate, allowed in ALLOWED.items():
        config = tomllib.loads((ROOT / "crates" / crate / "Cargo.toml").read_text())
        # Tests, build scripts, aliases and target-specific sections must preserve
        # the same boundary as the production dependency list.
        deps = {
            details.get("package", name) if isinstance(details, dict) else name
            for section in [config, *config.get("target", {}).values()]
            for kind in DEPENDENCY_KINDS
            for name, details in section.get(kind, {}).items()
        }
        actual = {dep.removeprefix("limeos-") for dep in deps if dep.startswith("limeos-")}
        assert actual <= allowed, (
            f"{crate}: dependency direction violation {actual - allowed}"
        )
        if crate in ["domain", "policy", "contracts", "identity"]:
            assert not {"axum", "rusqlite", "reqwest", "hyper"} & deps, (
                f"{crate}: adapter dependency in domain"
            )
        assert config["lints"]["workspace"], f"{crate}: unsafe-code policy not inherited"
    # Deferred code (docs/deferred-code.md) is reachable only through each
    # crate's own `deferred` feature, so the default build never contains it.
    problems = []
    for manifest in workspace_manifests():
        name = manifest.parent.relative_to(ROOT).as_posix() or "workspace"
        problems += deferred_violations(name, tomllib.loads(manifest.read_text()))
    assert not problems, "deferred feature leak:\n" + "\n".join(problems)
    # A source file outside the module graph is never compiled or tested.
    stray = [
        path.relative_to(ROOT).as_posix()
        for manifest in workspace_manifests()[1:]
        for path in unreachable_sources(manifest)
    ]
    stray = [path for path in stray if path not in UNREACHABLE_SOURCE_EXCEPTIONS]
    assert not stray, "source files outside every module graph: " + ", ".join(stray)
    listing = subprocess.run(
        ["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=False
    )
    if listing.returncode == 0:
        for file in listing.stdout.splitlines():
            assert not any(
                part in ["target", "node_modules", "dist", "__pycache__", ".ruff_cache"]
                for part in Path(file).parts
            ), f"tracked build output: {file}"
    assert (ROOT / "Cargo.lock").is_file(), "Cargo.lock is required"
    assert not (ROOT / "Docs").exists(), "Only one docs root is allowed"
    print("Repository dependency and hygiene checks passed.")


if __name__ == "__main__":
    main()
