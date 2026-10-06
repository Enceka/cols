#!/usr/bin/env python3
"""Make a NEW local-test public profile with the target's onboard_complete bit.

The source is unchanged. The bit index comes from the runtime UI table and its
location from the runtime schema; every other profile byte is preserved.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import struct
import zlib

from clientlib import manifest
from prepare_inventory import public_snapshot
from schema_fields import flatten, value


def mark_complete(arguments, fields, index):
    snapshot, tail = public_snapshot(arguments)
    projected = [f for f in fields.values() if f["projected"]]
    expected = b"".join(b"\x07" + snapshot[f["offset"]:f["offset"] + 4] for f in projected) + b"\0"
    if len(projected) != 125 or expected != tail:
        raise ValueError("public projection does not match snapshot")
    f = fields["uiNotificationFlags[%d]" % index]
    if f["type"] != 2 or f["projected"]:
        raise ValueError("onboarding field is not an unprojected Boolean")
    if value(snapshot, f):
        raise ValueError("onboard_complete is already set; source left unchanged")
    edited = bytearray(snapshot)
    edited[f["offset"]] |= 1 << f["bit"]
    crc = zlib.crc32(edited[4:]); edited[:4] = struct.pack("<I", crc)
    blob = zlib.compress(edited, 1)
    u32 = lambda n: b"\x08" + struct.pack("<I", n)
    output = u32(1) + u32(1) + u32(crc) + b"\x13" + u32(len(blob)) + blob + tail
    return output, {"field": "uiNotificationFlags[%d]" % index, "offset": f["offset"], "bit": f["bit"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", required=True, type=Path)
    parser.add_argument("--public-profile", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.assets.stat().st_size > 16 * 1024 * 1024 or args.public_profile.stat().st_size > 61440:
        parser.error("input exceeds preparation budget")
    assets = json.loads(args.assets.read_text())
    client = next(f["sha256"] for f in manifest()["files"] if f["path"] == "CODOL/codoMP_client_shipRetail.exe")
    if assets.get("client_sha256") != client or not assets.get("read_only"):
        parser.error("target runtime extraction required")
    d = assets["schemas"]["mp/playerdata.def"]["definitions"][0]
    if (d["version"], d["formatChecksum"]) != (233, 3811397846):
        parser.error("public layout differs from recovered version")
    table = assets["tables"]["mp/uiNotifications.csv"]
    if not table.get("found") or table["columns"] < 2 or len(table["cells_base64"]) != table["columns"] * table["rows"]:
        parser.error("loaded UI notification table required")
    indices = []
    for i in range(table["rows"]):
        row = table["cells_base64"][i * table["columns"]:(i + 1) * table["columns"]]
        if base64.b64decode(row[0]) == b"onboard_complete":
            indices.append(int(base64.b64decode(row[1])))
    if len(indices) != 1 or not 0 <= indices[0] < 128:
        parser.error("onboard_complete index is absent or ambiguous")
    original = args.public_profile.read_bytes()
    edited, changes = mark_complete(original, flatten(d), indices[0])
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as target:
        target.write(edited); target.flush(); os.fsync(target.fileno())
    print(json.dumps({"created": str(args.output), "source_sha256": hashlib.sha256(original).hexdigest(),
                      "output_sha256": hashlib.sha256(edited).hexdigest(), "changes": changes,
                      "scope": "local_lobby_test_profile_not_tutorial_completion_evidence"}, indent=2))


if __name__ == "__main__":
    main()
