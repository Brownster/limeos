#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"Check dependency direction, deferred features, module reachability, build-output hygiene and the evidence policy."

import os
import re
import subprocess
import sys
from pathlib import Path
from typing import NamedTuple

import tomllib

# The evidence policy lives beside this script; make it importable when this
# module is loaded by path (the unit tests do that) as well as when run.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from evidence_bundles import tracked_violations  # noqa: E402

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


class UnsupportedSource(Exception):
    """A source form the module-graph check cannot resolve; it fails rather than guess."""

    def __init__(self, message, position=None):
        super().__init__(message)
        self.position = position


class Token(NamedTuple):
    kind: str  # "ident", "str" (a decoded string literal), "lit" (any other literal) or "punct"
    value: str
    position: int  # offset in the file, for error messages


# One alternative per token class, tried in order. Block comments and raw
# strings are finished by hand: a block comment nests and a raw string ends
# at a quote followed by as many `#` as opened it.
LEXEME = re.compile(
    r"""
      (?P<space>\s+)
    | (?P<comment>//[^\n]*)
    | (?P<block>/\*)
    | (?P<raw>(?P<prefix>[bc])?r(?P<hashes>\#*)")
    | (?P<prefixed>[bc]"(?:[^"\\]|\\.)*"|b'(?:\\.[^'\n]*|[^\\'\n])')
    | (?P<ident>(?:r\#)?[^\W\d]\w*)
    | (?P<number>\d\w*)
    | (?P<str>"(?:[^"\\]|\\.)*")
    | (?P<char>'(?:\\.[^'\n]*|[^\\'\n])')
    | (?P<lifetime>'[^\W\d]\w*)
    | (?P<punct>.)
    """,
    re.S | re.X,
)
BLOCK_EDGE = re.compile(r"/\*|\*/")
ESCAPE = re.compile(r"\\(?:\n\s*|u\{([0-9a-fA-F]+)\}|x([0-9a-fA-F]{2})|(.))", re.S)
ESCAPES = {"n": "\n", "r": "\r", "t": "\t", "\\": "\\", "0": "\0", "'": "'", '"': '"'}
OPEN, CLOSE = "([{", ")]}"


def decode(body):
    """The value of a non-raw string literal body; unknown escapes are kept as written."""

    def replace(match):
        code = match.group(1) or match.group(2)
        if code:
            return chr(int(code, 16))
        other = match.group(3)
        return "" if other is None else ESCAPES.get(other, match.group(0))

    return ESCAPE.sub(replace, body)


def tokenize(text):
    """Rust tokens of `text` without comments; every literal is a single token."""
    out, i, n = [], 0, len(text)
    if text.startswith("#!") and not text[2:].lstrip().startswith("["):
        i = n if (newline := text.find("\n")) < 0 else newline  # shebang line
    while i < n:
        match = LEXEME.match(text, i)
        kind, start, i = match.lastgroup, i, match.end()
        if kind in ("space", "comment"):
            continue
        if kind == "block":
            depth = 1
            for edge in BLOCK_EDGE.finditer(text, i):
                depth += 1 if edge.group() == "/*" else -1
                if depth == 0:
                    i = edge.end()
                    break
            else:
                raise UnsupportedSource("unterminated block comment", start)
        elif kind == "raw":
            close = '"' + match.group("hashes")
            end = text.find(close, i)
            if end < 0:
                raise UnsupportedSource("unterminated raw string", start)
            out.append(Token("lit" if match.group("prefix") else "str", text[i:end], start))
            i = end + len(close)
        elif kind == "str":
            out.append(Token("str", decode(match.group()[1:-1]), start))
        elif kind == "ident":
            out.append(Token("ident", match.group().removeprefix("r#"), start))
        elif kind == "punct":
            if match.group() in "\"'":
                raise UnsupportedSource("unterminated literal", start)
            out.append(Token("punct", match.group(), start))
        else:
            out.append(Token("lit", match.group(), start))
    return out


def is_token(tokens, index, kind, value):
    return index < len(tokens) and tokens[index].kind == kind and tokens[index].value == value


def opens(tokens, index):
    return index < len(tokens) and tokens[index].kind == "punct" and tokens[index].value in OPEN


def matching(tokens, index):
    """Index of the bracket closing the one at `index`."""
    depth = 0
    for j in range(index, len(tokens)):
        if opens(tokens, j):
            depth += 1
        elif tokens[j].kind == "punct" and tokens[j].value in CLOSE:
            depth -= 1
            if depth == 0:
                return j
    raise UnsupportedSource("unbalanced brackets", tokens[index].position)


def split_arguments(tokens):
    """Top-level comma-separated parts of a token run."""
    parts, start, j = [], 0, 0
    while j < len(tokens):
        if opens(tokens, j):
            j = matching(tokens, j)
        elif is_token(tokens, j, "punct", ","):
            parts.append(tokens[start:j])
            start = j + 1
        j += 1
    parts.append(tokens[start:])
    return [part for part in parts if part]


def path_value(body):
    """The string of a `path = "..."` attribute body; any other value fails."""
    if len(body) == 3 and is_token(body, 1, "punct", "=") and body[2].kind == "str":
        return body[2].value
    raise UnsupportedSource("#[path] value is not a plain string literal", body[0].position)


def path_attributes(attributes):
    """(`#[path]` value or None, paths that apply only under a `cfg_attr` predicate)."""
    explicit, conditional = None, []

    def conditional_paths(body):
        if is_token(body, 1, "punct", "("):
            for part in split_arguments(body[2 : matching(body, 1)])[1:]:
                if is_token(part, 0, "ident", "path"):
                    conditional.append(path_value(part))
                elif is_token(part, 0, "ident", "cfg_attr"):
                    conditional_paths(part)

    for body in attributes:
        if is_token(body, 0, "ident", "path"):
            explicit = path_value(body)
        elif is_token(body, 0, "ident", "cfg_attr"):
            conditional_paths(body)
    return explicit, conditional


def is_visibility(tokens):
    """True for nothing, `pub` or `pub(...)`: what may sit between attributes and `mod`."""
    if not tokens:
        return True
    if not is_token(tokens, 0, "ident", "pub"):
        return False
    return len(tokens) == 1 or (
        is_token(tokens, 1, "punct", "(") and is_token(tokens, len(tokens) - 1, "punct", ")")
    )


def file_edges(path, mod_rs):
    """(file, is mod-rs) pairs one source file brings into its crate.

    Follows rustc: `mod x;` looks in the module's directory (a non-mod-rs file
    `a.rs` owns `a/`); `#[path]` is relative to the file's own directory or the
    enclosing inline module's; a `#[path]` file and an `include!`d file behave
    as mod.rs files; and `#[path]` on an inline module names its directory.
    `cfg_attr(…, path = …)` on `mod x;` reaches both candidates.
    """
    text = path.read_text(errors="ignore")
    try:
        return module_edges(tokenize(text), path, mod_rs)
    except UnsupportedSource as error:
        line = text.count("\n", 0, error.position or 0) + 1
        raise UnsupportedSource(f"{path.as_posix()}:{line}: {error}") from None


def module_edges(tokens, path, mod_rs):
    # Module context: (directory, subdirectory a non-mod-rs file owns), with
    # the brace depth at which an inline module opened.
    scopes = [((path.parent, None if mod_rs else path.stem), -1)]
    edges, depth, attributes, attributes_end, i = [], 0, [], -1, 0
    while i < len(tokens):
        token = tokens[i]
        if token.kind == "punct" and token.value == "#":
            inner = is_token(tokens, i + 1, "punct", "!")
            bracket = i + 2 if inner else i + 1
            if is_token(tokens, bracket, "punct", "["):
                close = matching(tokens, bracket)
                if i != attributes_end:
                    attributes = []
                if not inner:
                    attributes.append(tokens[bracket + 1 : close])
                attributes_end = i = close + 1
                continue
        elif token.kind == "punct" and token.value == "{":
            depth += 1
        elif token.kind == "punct" and token.value == "}":
            depth -= 1
            while len(scopes) > 1 and depth <= scopes[-1][1]:
                scopes.pop()
        elif token.kind == "ident" and token.value == "include" and opens(tokens, i + 2) and (
            is_token(tokens, i + 1, "punct", "!")
        ):
            close = matching(tokens, i + 2)
            arguments = split_arguments(tokens[i + 3 : close])
            # A computed argument (concat!, env!) cannot be resolved here and
            # names generated code outside src/; it is skipped, not guessed.
            if len(arguments) == 1 and len(arguments[0]) == 1 and arguments[0][0].kind == "str":
                edges.append((path.parent / arguments[0][0].value, True))
            i = close + 1
            continue
        elif token.kind == "ident" and token.value == "mod":
            if is_token(tokens, i + 1, "punct", "$"):
                raise UnsupportedSource("module named by a macro variable", token.position)
            end = tokens[i + 2].value if i + 2 < len(tokens) else None
            if i + 1 < len(tokens) and tokens[i + 1].kind == "ident" and end in (";", "{"):
                applicable = attributes if is_visibility(tokens[max(attributes_end, 0) : i]) else []
                explicit, conditional = path_attributes(applicable)
                (directory, owned), _ = scopes[-1]
                module = tokens[i + 1].value
                if end == "{":
                    if conditional:
                        raise UnsupportedSource(
                            f"cfg_attr path on inline module {module}", token.position
                        )
                    if explicit is not None:
                        inner_directory = directory / explicit
                    else:
                        inner_directory = directory.joinpath(*filter(None, [owned, module]))
                    scopes.append(((inner_directory, None), depth))
                    depth += 1
                    i += 3
                    continue
                for value in ([explicit] if explicit is not None else []) + conditional:
                    edges.append((directory / value, True))
                if explicit is None:
                    base = directory / owned if owned else directory
                    edges += [(base / f"{module}.rs", False), (base / module / "mod.rs", True)]
        i += 1
    return edges


def module_files(root):
    """Files reached from one crate root through `mod`, `#[path]` and `include!`."""
    reached, pending = set(), [(root, True)]
    while pending:
        current, mod_rs = pending.pop()
        current = current.resolve()
        if current in reached or not current.is_file():
            continue
        reached.add(current)
        pending.extend(file_edges(current, mod_rs))
    return reached


# Target kind -> (directory Cargo searches, [package] key that turns discovery off).
TARGET_KINDS = {
    "bin": ("src/bin", "autobins"),
    "example": ("examples", "autoexamples"),
    "test": ("tests", "autotests"),
    "bench": ("benches", "autobenches"),
}


def discovered_targets(crate, kind, package_name):
    """Name -> path of the targets Cargo would infer for one kind."""
    directory = crate / TARGET_KINDS[kind][0]
    found = {}
    if kind == "bin" and (crate / "src/main.rs").is_file():
        found[package_name] = crate / "src/main.rs"
    found.update({path.stem: path for path in sorted(directory.glob("*.rs"))})
    found.update({path.parent.name: path for path in sorted(directory.glob("*/main.rs"))})
    return found


def crate_roots(manifest):
    """Every target root and build script of a package, as Cargo resolves them.

    Declared `path`s are roots; a declared target without one is found by
    name; discovery adds the conventional files unless `auto*` turns it off or
    a declared target of the same kind already has that name or path.
    """
    crate = manifest.parent
    config = tomllib.loads(manifest.read_text())
    package = config.get("package")
    if package is None:
        return []
    roots = []
    if "lib" in config:
        roots.append(crate / config["lib"].get("path", "src/lib.rs"))
    elif package.get("autolib", True):
        roots.append(crate / "src/lib.rs")
    for kind, (directory, auto) in TARGET_KINDS.items():
        discovered = discovered_targets(crate, kind, package.get("name"))
        names, paths = set(), set()
        for target in config.get(kind, []):
            name = target.get("name")
            if "path" in target:
                path = crate / target["path"]
            else:
                candidates = [crate / directory / f"{name}.rs", crate / directory / name / "main.rs"]
                path = discovered.get(name) or next((c for c in candidates if c.is_file()), None)
            names.add(name)
            if path is not None:
                paths.add(path.resolve())
                roots.append(path)
        if package.get(auto, True):
            roots += [
                path for name, path in discovered.items()
                if name not in names and path.resolve() not in paths
            ]
    build = package.get("build", True)
    if build is True:
        roots.append(crate / "build.rs")
    elif isinstance(build, str):
        roots.append(crate / build)
    elif isinstance(build, list):
        roots += [crate / script for script in build]
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
    try:
        stray = [
            path.relative_to(ROOT).as_posix()
            for manifest in workspace_manifests()[1:]
            for path in unreachable_sources(manifest)
        ]
    except UnsupportedSource as error:
        raise AssertionError(f"module graph check cannot resolve: {error}") from None
    stray = [path for path in stray if path not in UNREACHABLE_SOURCE_EXCEPTIONS]
    assert not stray, "source files outside every module graph: " + ", ".join(stray)
    listing = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=False
    )
    if listing.returncode == 0:
        for file in map(os.fsdecode, filter(None, listing.stdout.split(b"\0"))):
            assert not any(
                part in ["target", "node_modules", "dist", "__pycache__", ".ruff_cache"]
                for part in Path(file).parts
            ), f"tracked build output: {file}"
    # Raw evidence lives in release bundles (docs/rewrite-evidence/README.md).
    evidence = tracked_violations(ROOT)
    assert not evidence, "evidence policy violation: " + "; ".join(evidence[:20])
    assert (ROOT / "Cargo.lock").is_file(), "Cargo.lock is required"
    assert not (ROOT / "Docs").exists(), "Only one docs root is allowed"
    print("Repository dependency and hygiene checks passed.")


if __name__ == "__main__":
    main()
