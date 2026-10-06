#!/usr/bin/env python3
"""Create local starter ownership from the actual saved first loadout and target item table.

Does not edit the public/private profile or invent item IDs. Refuses overwrite.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import struct
import tempfile
import zlib

from clientlib import manifest
from schema_fields import flatten, value


def public_snapshot(arguments):
    if len(arguments) > 61440 or len(arguments) < 22 or arguments[0] != 8 or arguments[5] != 8 or arguments[10] != 8 or arguments[15:17] != b"\x13\x08":
        raise ValueError("invalid public profile envelope")
    compression, version, crc = (struct.unpack_from("<I", arguments, o)[0] for o in (1, 6, 11))
    count = struct.unpack_from("<I", arguments, 17)[0]
    blob = arguments[21:21 + count]
    if len(blob) != count or version != 1 or compression not in (0, 1):
        raise ValueError("unsupported public profile format")
    if compression:
        decoder = zlib.decompressobj()
        plain = decoder.decompress(blob, 32769)
        if not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
            raise ValueError("profile compressed stream exceeds bounds or contains extra bytes")
    else:
        plain = blob
    if len(plain) != 32768 or zlib.crc32(plain[4:]) != crc or struct.unpack_from("<I", plain)[0] != crc:
        raise ValueError("profile size or CRC mismatch")
    return plain, arguments[21 + count:]


def selected_items(fields, snapshot):
    selected = value(snapshot, fields["currentCharacterIndex"])
    if not 0 <= selected < 12:
        raise ValueError("character selection outside target bounds")
    names = ["characterClasses[%d].slots[%d]" % (selected, i) for i in range(9)]
    # Perk/wildcard/special arrays also contain non-item scalars in the observed
    # profile. Only grant the identified core equipment references in this step.
    names += ["weaponClasses[0]." + n for n in ("primaryBase", "secondaryBase", "meleeWpn", "lethal", "tactical")]
    for group, count in [("primaryAttach", 6), ("secondaryAttach", 6)]:
        names += ["weaponClasses[0].%s[%d]" % (group, i) for i in range(count)]
    ids = []
    for name in names:
        item = value(snapshot, fields[name])
        if item < 0:
            raise ValueError("negative item reference in " + name)
        if item and item not in ids:
            ids.append(item)
    if not ids or len(ids) > 128:
        raise ValueError("unsupported inventory size")
    return ids


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--public-profile", type=Path, required=True)
    parser.add_argument("--service-data-dir", type=Path, required=True)
    args = parser.parse_args()
    if args.assets.stat().st_size > 16 * 1024 * 1024 or args.public_profile.stat().st_size > 61440:
        parser.error("input exceeds preparation budget")
    assets = json.loads(args.assets.read_text())
    client = next(f["sha256"] for f in manifest()["files"] if f["path"] == "CODOL/codoMP_client_shipRetail.exe")
    if assets.get("client_sha256") != client or not assets.get("read_only"):
        parser.error("expected target-version runtime extraction")
    definition = assets["schemas"]["mp/playerdata.def"]["definitions"][0]
    if (definition["version"], definition["formatChecksum"]) != (233, 3811397846):
        parser.error("public schema differs from recovered target")
    fields = flatten(definition)
    snapshot, tail = public_snapshot(args.public_profile.read_bytes())
    projected = [f for f in fields.values() if f["projected"]]
    expected = b"".join(b"\x07" + snapshot[f["offset"]:f["offset"] + 4] for f in projected) + b"\0"
    if len(projected) != 125 or tail != expected:
        parser.error("public projection does not match its snapshot")
    ids = selected_items(fields, snapshot)
    table = assets["tables"]["mp/coMarketplaceItems.csv"]
    if not table.get("found") or table["columns"] != 12 or len(table["cells_base64"]) != table["rows"] * 12:
        parser.error("target item table is missing or incomplete")
    catalogue = {}
    for i in range(table["rows"]):
        row = table["cells_base64"][i * 12:(i + 1) * 12]
        item_id = int(base64.b64decode(row[0]))
        if item_id in catalogue:
            parser.error("duplicate item ID in target table")
        catalogue[item_id] = base64.b64decode(row[1]).decode("ascii")
    if any(item not in catalogue for item in ids):
        parser.error("equipped item does not exist in target table")
    uid = int(args.public_profile.parent.name)
    title = int(args.public_profile.parent.parent.name)
    if not 0 < uid <= 2**64 - 1 or not 0 <= title <= 2**32 - 1:
        parser.error("profile is not in a title/UID storage directory")
    directory = args.service_data_dir / "profiles" / str(title) / str(uid)
    directory.mkdir(parents=True, exist_ok=True)
    output = directory / "inventory.json"
    payload = {"schema_version": 1, "client_sha256": client, "title_id": title, "user_id": str(uid), "items": ids}
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=directory, prefix=".inventory-", delete=False) as temp:
            temporary = Path(temp.name)
            temp.write((json.dumps(payload, indent=2) + "\n").encode())
            temp.flush(); os.fsync(temp.fileno())
        os.link(temporary, output)  # Atomic create, never replace an existing grant file.
    finally:
        if temporary:
            temporary.unlink(missing_ok=True)
    print(json.dumps({"created": str(output), "scope": "local_first_loadout_core_ownership", "items": [{"id": i, "name": catalogue[i]} for i in ids], "profile_sha256": hashlib.sha256(args.public_profile.read_bytes()).hexdigest()}, indent=2))


if __name__ == "__main__":
    main()
