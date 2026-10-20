"""Read the candidate's schema expectation from its trusted Rust source only."""

import re
from pathlib import Path

MAX_SOURCE_BYTES = 128 * 1024
SOURCE_NAME = "current-persistence-lib.rs"


def source_authority_schema(path: Path) -> int:
    """Require one bounded, canonical public u32 schema declaration."""
    with Path(path).open("rb") as stream:
        source = stream.read(MAX_SOURCE_BYTES + 1)
    if len(source) > MAX_SOURCE_BYTES:
        raise ValueError("authority schema source exceeds bound")
    declarations = re.findall(
        rb"(?m)^[ \t]*(?:pub(?:\([^\r\n)]*\))?[ \t]+)?const[ \t]+SCHEMA_VERSION\b[^\r\n]*",
        source,
    )
    if len(declarations) != 1:
        raise ValueError("exactly one authority schema declaration required")
    version = re.fullmatch(
        rb"pub const SCHEMA_VERSION: u32 = ([1-9][0-9]{0,9});",
        declarations[0].strip(),
    )
    if version is None:
        raise ValueError("malformed authority schema declaration")
    schema = int(version[1])
    if schema > 0xFFFFFFFF:
        raise ValueError("malformed authority schema declaration")
    return schema


def current_authority_schema() -> int:
    """Use the checkout source or the explicit source delivered by the VM runner.

    The runner replaces its delivered file with an optional build bundle's
    actual source before invoking a fixture. Missing or invalid input fails;
    there is no numeric default or lookup of an installed database.
    """
    directory = Path(__file__).resolve().parent
    if directory.name == "privileged_vm" and directory.parent.name == "tests":
        source = directory.parent.parent / "crates/persistence/src/lib.rs"
    else:
        source = directory / SOURCE_NAME
    return source_authority_schema(source)
