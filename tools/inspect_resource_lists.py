#!/usr/bin/env python3
"""Read bounded plaintext IFS lists; never mount, decompress or merge patches.

The 2.6.9.7 loading report suggests offset0xac. Treat it as a candidate: require
nifs magic, an ASCII .lst header, and consecutive path lines. An unrecognized
list says nothing about package health or completion. Results are a union of
observed names, not the effective patched filesystem or an asset dependency graph.
"""
import argparse
import hashlib
import json
from pathlib import Path

from clientlib import DEFAULT_WORKSPACE, REPO, checked_sample

LIMIT = 2 * 1024 * 1024
PREFIXES = (b"main/", b"hires/", b"client_shipRetail/")


def read_list(data):
    if data[:4] != b"nifs":
        raise ValueError("not an IFS resource")
    start = 0xac
    end = data.find(b"\r\n", start, start + 256)
    header = data[start:end] if end >= 0 else b""
    if not header.endswith(b".lst") or any(n < 32 or n > 126 for n in header):
        return {"recognized": False}
    paths = []
    offset = end + 2
    while offset < len(data):
        end = data.find(b"\r\n", offset, offset + 1024)
        line = data[offset:end] if end >= 0 else b""
        if not line.startswith(PREFIXES) or any(n < 32 or n > 126 for n in line):
            break
        paths.append({"file_offset": offset, "path": line.decode("ascii")})
        offset = end + 2
    return {"recognized": True, "header": header.decode("ascii"),
            "paths": paths, "scan_end_offset": offset,
            "read_limit_reached": len(data) == LIMIT and offset + 1024 >= len(data)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=DEFAULT_WORKSPACE)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    checked_sample(args.workspace, "CODOL/codoMP_client_shipRetail.exe")
    inventory = REPO / "fixtures/resources-3.3.3.3.json"
    packages = []
    for expected in json.loads(inventory.read_text())["resources"]:
        name = expected["name"]
        if Path(name).name != name:
            raise ValueError("invalid inventory filename")
        path = args.workspace / "CODOL/IIPS/IIPSDownload" / name
        if path.stat().st_size != expected["size"]:
            raise ValueError("resource size differs from recorded inventory: " + name)
        with path.open("rb") as source:
            data = source.read(LIMIT)
        packages.append({"package": name, "size": expected["size"],
                         "prefix_sha256": hashlib.sha256(data).hexdigest(), **read_list(data)})
    result = {"schema_version": 1, "version": "3.3.3.3", "read_only": True,
              "inventory_sha256": hashlib.sha256(inventory.read_bytes()).hexdigest(),
              "effective_patch_view": False, "block_integrity_verified": False,
              "read_limit_per_package": LIMIT, "packages": packages}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"output": str(args.output), "packages": len(packages),
                      "recognized_lists": sum(p["recognized"] for p in packages),
                      "observed_paths": sum(len(p.get("paths", [])) for p in packages)}))


if __name__ == "__main__":
    main()
