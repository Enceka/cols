"""Shared sample identity checks for the local, read-only research tools."""
import hashlib
import json
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
DEFAULT_WORKSPACE = REPO.parent
MANIFEST = REPO / "fixtures/client-3.3.3.3.json"


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def manifest():
    return json.loads(MANIFEST.read_text(encoding="utf-8"))


def checked_sample(workspace, relative):
    expected = next(f for f in manifest()["files"] if f["path"] == relative)
    path = workspace / relative
    if path.stat().st_size != expected["size"] or sha256(path) != expected["sha256"]:
        raise ValueError("sample identity mismatch: " + str(path))
    return path


def decode_config(data):
    if data.startswith((b"\xff\xfe", b"\xfe\xff")):
        return data.decode("utf-16")
    if b"\0" in data[:64]:
        return data.decode("utf-16le")
    return data.decode("gb18030")


def matches_client_image(actual, executable, prefix):
    """Wine can report the PE's Unix path or its mapped DOS Z: path."""
    expected = str(Path(executable).resolve())
    if actual == expected:
        return True
    if not actual.startswith(("Z:\\", "z:\\")):
        return False
    drive = Path(prefix) / "dosdevices/z:"
    if not drive.is_symlink() or drive.resolve() != Path("/"):
        return False
    return actual[2:].replace("\\", "/") == expected
