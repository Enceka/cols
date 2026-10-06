#!/usr/bin/env python3
"""Verify fixed sample hashes, string anchors and the local IIPS file inventory.

Standard library only. File/header matching does NOT validate cached IIPS blocks.
--hash-resources additionally checks all resource content against the recorded baseline.
"""
import argparse
import configparser
import json
import sys
from pathlib import Path

from clientlib import DEFAULT_WORKSPACE, REPO, checked_sample, decode_config, manifest, sha256


def verify(workspace, hash_resources=False):
    expected = manifest()
    resource_baseline = json.loads((REPO / "fixtures/resources-3.3.3.3.json").read_text())
    expected_resources = {r["name"]: r for r in resource_baseline["resources"]}
    failures = []
    verified = []
    for item in expected["files"]:
        try:
            checked_sample(workspace, item["path"])
            verified.append(item["path"])
        except (OSError, ValueError) as error:
            failures.append(str(error))
    executable = workspace / "CODOL/codoMP_client_shipRetail.exe"
    if executable.exists():
        data = executable.read_bytes()
        for marker in expected["markers"]:
            offset = int(marker["offset"], 16)
            value = marker["ascii"].encode("ascii")
            if data[offset:offset + len(value)] != value:
                failures.append("marker mismatch: " + marker["ascii"])
    cfg = configparser.ConfigParser(interpolation=None)
    cfg.read_string(decode_config((workspace / "CODOL/IIPS/IIPSFilelist.ini").read_bytes()))
    names = [cfg[s]["FileName"] for s in cfg.sections() if s.startswith("FileItem")]
    folder = workspace / "CODOL/IIPS/IIPSDownload"
    actual = {p.name for p in folder.glob("*.ifs")}
    missing, extra = sorted(set(names) - actual), sorted(actual - set(names))
    if len(names) != expected["resource_count"] or int(cfg["FileList"]["FileCount"]) != len(names):
        failures.append("resource count mismatch")
    if len(names) != len(set(names)):
        failures.append("duplicate resource names in index")
    if missing or extra:
        failures.append("resource filename set mismatch")
    if set(names) != set(expected_resources):
        failures.append("resource baseline filename set mismatch")
    resources = []
    for name in sorted(set(names) & actual):
        path = folder / name
        with path.open("rb") as source:
            magic = source.read(4)
        if magic != b"nifs":
            failures.append("unexpected IIPS magic: " + name)
        entry = {"name": name, "size": path.stat().st_size, "magic_hex": magic.hex()}
        baseline = expected_resources.get(name, {})
        if entry["size"] != baseline.get("size"):
            failures.append("resource size mismatch: " + name)
        if hash_resources:
            entry["sha256"] = sha256(path)
            if entry["sha256"] != baseline.get("sha256"):
                failures.append("resource SHA-256 mismatch: " + name)
        resources.append(entry)
    return {
        "schema_version": 1, "verified": not failures, "version": expected["version"],
        "verified_files": verified, "failures": failures,
        "resources_present": len(actual), "missing": missing, "extra": extra,
        "resource_block_integrity_verified": False, "resources": resources,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=DEFAULT_WORKSPACE)
    parser.add_argument("--hash-resources", action="store_true")
    args = parser.parse_args()
    try:
        result = verify(args.workspace, args.hash_resources)
    except (OSError, ValueError, KeyError, configparser.Error) as error:
        result = {"verified": False, "failures": [str(error)]}
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if result["verified"] else 1


if __name__ == "__main__":
    sys.exit(main())
