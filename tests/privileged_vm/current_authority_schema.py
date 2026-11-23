"""Read the candidate's schema expectation from its trusted Rust source only.

The current source declares its schema as the last entry of the migration
table, `MIGRATIONS` in `crates/persistence/src/migrations.rs`
(`SCHEMA_VERSION` in `lib.rs` is computed from it). Older sources declared
`pub const SCHEMA_VERSION: u32 = N;` in `crates/persistence/src/lib.rs`;
those are still read, so historical candidates keep their own schema.
"""

import re
from pathlib import Path

MAX_SOURCE_BYTES = 128 * 1024
# The delivered copy of whichever source declares the schema (the migration
# table, or an older candidate's lib.rs).
SOURCE_NAME = "current-persistence-schema.rs"
MIGRATIONS_SOURCE = "crates/persistence/src/migrations.rs"
LEGACY_SOURCE = "crates/persistence/src/lib.rs"

TABLE_START = re.compile(rb"(?m)^[ \t]*pub\(crate\)[ \t]+const[ \t]+MIGRATIONS:[ \t]*&\[Migration\][ \t]*=[ \t]*&\[[ \t]*$")
TABLE_END = re.compile(rb"(?m)^\];[ \t]*$")
# An entry's version: the first argument of `plain(` or `checked(`, or the
# `version:` field of a `Migration { .. }` literal.
ENTRY = re.compile(rb"\b(?:plain|checked)\(\s*([0-9]+)\s*,|\bMigration\s*\{\s*version:\s*([0-9]+)\s*,")


def _read(path: Path) -> bytes:
    with Path(path).open("rb") as stream:
        source = stream.read(MAX_SOURCE_BYTES + 1)
    if len(source) > MAX_SOURCE_BYTES:
        raise ValueError(f"authority schema source exceeds bound: {path}")
    return source


def _table_schema(source: bytes, path: Path) -> int:
    starts = TABLE_START.findall(source)
    if len(starts) != 1:
        raise ValueError(
            f"{path}: expected exactly one `pub(crate) const MIGRATIONS: &[Migration] = &[` line, found {len(starts)}"
        )
    body = source[TABLE_START.search(source).end():]
    end = TABLE_END.search(body)
    if end is None:
        raise ValueError(f"{path}: the MIGRATIONS table has no closing `];` line")
    body = body[: end.start()]
    versions = [int(a or b) for a, b in ENTRY.findall(body)]
    if not versions:
        raise ValueError(
            f"{path}: no entries found in the MIGRATIONS table (looked for `plain(N,`, `checked(N,` and `Migration {{ version: N,`)"
        )
    if versions != list(range(1, len(versions) + 1)):
        raise ValueError(f"{path}: MIGRATIONS versions are not 1, 2, 3 ... in order: {versions}")
    if body.count(b"include_str!(") != len(versions):
        raise ValueError(
            f"{path}: {len(versions)} MIGRATIONS entries but {body.count(b'include_str!(')} `include_str!(` sources"
        )
    if versions[-1] > 0xFFFFFFFF:
        raise ValueError(f"{path}: authority schema out of range")
    return versions[-1]


def _legacy_schema(source: bytes, path: Path) -> int:
    declarations = re.findall(
        rb"(?m)^[ \t]*(?:pub(?:\([^\r\n)]*\))?[ \t]+)?const[ \t]+SCHEMA_VERSION\b[^\r\n]*",
        source,
    )
    if len(declarations) != 1:
        raise ValueError(
            f"{path}: exactly one authority schema declaration required (looked for `pub const SCHEMA_VERSION: u32 = N;`, or a MIGRATIONS table)"
        )
    if b"MIGRATIONS" in declarations[0]:
        raise ValueError(
            f"{path}: SCHEMA_VERSION is computed from the migration table here; read {MIGRATIONS_SOURCE} instead"
        )
    version = re.fullmatch(
        rb"pub const SCHEMA_VERSION: u32 = ([1-9][0-9]{0,9});",
        declarations[0].strip(),
    )
    if version is None:
        raise ValueError(
            f"{path}: malformed authority schema declaration (expected `pub const SCHEMA_VERSION: u32 = N;`)"
        )
    schema = int(version[1])
    if schema > 0xFFFFFFFF:
        raise ValueError(f"{path}: malformed authority schema declaration")
    return schema


def source_authority_schema(path: Path) -> int:
    """The schema one bounded source file declares: its MIGRATIONS table's
    last version, or an older source's canonical `SCHEMA_VERSION` line."""
    source = _read(path)
    if TABLE_START.search(source) is not None or b"const MIGRATIONS" in source:
        return _table_schema(source, Path(path))
    return _legacy_schema(source, Path(path))


def schema_source(root: Path) -> Path:
    """The file of a source tree that declares its schema."""
    root = Path(root)
    table = root / MIGRATIONS_SOURCE
    return table if table.exists() else root / LEGACY_SOURCE


def tree_authority_schema(root: Path) -> int:
    """The schema a source tree declares."""
    return source_authority_schema(schema_source(root))


def current_authority_schema() -> int:
    """Use the checkout source or the explicit source delivered by the VM runner.

    The runner replaces its delivered file with an optional build bundle's
    actual source before invoking a fixture. Missing or invalid input fails;
    there is no numeric default or lookup of an installed database.
    """
    directory = Path(__file__).resolve().parent
    if directory.name == "privileged_vm" and directory.parent.name == "tests":
        return tree_authority_schema(directory.parent.parent)
    return source_authority_schema(directory / SOURCE_NAME)
