# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Keep exact guest sources in verified archives instead of duplicating them."""
import gzip
import hashlib
import io
from pathlib import Path
import shutil
import tarfile

root = Path(__file__).resolve().parents[2]
packet = root / "docs/rewrite-evidence/p04/2026-10-20-a1-namespace-budget"
old_hashes = (packet / "artifacts.sha256").read_bytes()
(packet / "artifacts-before-compression.sha256.gz").write_bytes(gzip.compress(old_hashes, mtime=0))
groups = ["candidate/guest", "diagnostic-candidate/guest", "proc-witness-guest", "proc-witness-v2-guest", "proc-witness-v3-guest"]
for relative in groups:
    directory = packet / relative
    files = sorted(directory.iterdir())
    assert files and all(path.is_file() and not path.is_symlink() and path.suffix == ".py" for path in files)
    expected = {path.name: path.read_bytes() for path in files}
    destination = directory.with_name(directory.name + ".tar.gz")
    assert not destination.exists()
    with destination.open("xb") as output:
        with gzip.GzipFile(fileobj=output, mode="wb", filename="", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for name, data in expected.items():
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    info.mode = 0o644
                    info.mtime = 0
                    archive.addfile(info, io.BytesIO(data))
    with tarfile.open(destination, "r:gz") as archive:
        observed = {item.name: archive.extractfile(item).read() for item in archive.getmembers()}
    assert observed == expected
    for path in files:
        assert path.read_bytes() == expected[path.name]
        path.unlink()
    directory.rmdir()
readme = packet / "README.md"
text = readme.read_text()
text += "\nGuest source directories are stored as verified `*.tar.gz` snapshots to avoid\nduplicating the same fixtures in Git. Every member was compared byte for byte\nbefore replacing the loose copy. Extract the appropriate archive to reproduce\nits recorded guest directory. `artifacts-before-compression.sha256.gz` preserves\nthe original loose-file checksums; `compress-packet.py` records this final step.\n"
readme.write_text(text)
shutil.copyfile(Path(__file__), packet / "compress-packet.py")
hashes = []
for path in sorted(packet.rglob("*")):
    if path.is_file() and path.name != "artifacts.sha256":
        hashes.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(packet)}\n")
(packet / "artifacts.sha256").write_text("".join(hashes))
print(f"Verified {len(groups)} guest archives; {len(hashes)} final packet files")
