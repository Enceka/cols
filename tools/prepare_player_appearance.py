#!/usr/bin/env python3
"""Prepare third-person character composition from measured 3.3 runtime caches."""
import argparse
import base64
import hashlib
import json
import struct
from pathlib import Path

from prepare_game_state import CLIENT_SHA256
from schema_fields import flatten

GROUPS = ["fpa", "boo", "fja", "han", "hea", "ves", "heg", "elp", "knp", "eye", "bar"]
KINDS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 11]


def native_hash(name):
    value = 0
    for byte in name.lower().encode("ascii"):
        value = ((value * 0x1000193) ^ byte) & 0xffffffff
    return value


def prepare(capture, assets):
    for value in (capture, assets):
        if value.get("client_sha256") != CLIENT_SHA256 or value.get("read_only") is not True:
            raise ValueError("appearance capture identity mismatch")
    definition = assets["schemas"]["mp/playerdata.def"]["definitions"][0]
    if (definition["version"], definition["formatChecksum"]) != (233, 3811397846):
        raise ValueError("unsupported appearance profile schema")
    fields = flatten(definition)
    selector = fields["currentCharacterIndex"]
    if selector["type"] != 1 or selector["width"] != 1:
        raise ValueError("invalid current-character selector")
    offsets = []
    for character in range(12):
        row = [fields[f"characterClasses[{character}].slots[{slot}]"] for slot in range(9)]
        if any(f["type"] != 0 or f["width"] != 4 for f in row):
            raise ValueError("invalid character slot schema")
        offsets.append([f["offset"] for f in row])
    hashes = list(struct.unpack("<9I", base64.b64decode(capture["selector_hashes_base64"], validate=True)))
    if hashes != [native_hash(n) for n in ["m", "f", "tf", "sc", "zm", "wht", "yel", "blk", "cau"]]:
        raise ValueError("native character selector order differs")
    packing_bits = None
    if "packing_bits_base64" in capture:
        packing_bits = list(struct.unpack("<11I", base64.b64decode(capture["packing_bits_base64"], validate=True)))
        fields_wire = list(struct.iter_unpack("<iii", base64.b64decode(capture["packing_fields_base64"], validate=True)))
        expected_offsets = [0x18,0x1c,0x20,0x14,0x24,0x28,0x2c,0x44,0x48,0x4c,0x54,0x40,0x50,0x30,0x34,0x38,0x3c,0x58,0x5c,0x60,0x64]
        if fields_wire != [(o,10 if o<0x40 else 6,0) for o in expected_offsets] or any(b>10 for b in packing_bits):
            raise ValueError("native character packing differs from measured client fields")
        # Historical probe labels follow address order, not semantic argument
        # names:0x17e02a8 is faction;0x17e1758 is gender in0x40c980.
        for key, expected in [("gender_selectors_base64", (2,3,4)),
                              ("faction_selectors_base64", (0,1)),
                              ("skin_selectors_base64", (5,6,7,8))]:
            values=tuple(v[0] for v in struct.iter_unpack("<I",base64.b64decode(capture[key],validate=True)))
            if values!=expected:
                raise ValueError("native character selector mapping differs")
    records = list(struct.iter_unpack("<HHI", base64.b64decode(capture["variant_items_base64"], validate=True)))
    groups = list(struct.iter_unpack("<HH", base64.b64decode(capture["variant_groups_base64"], validate=True)))
    if len(groups) != 11 or any(start + count > len(records) for start, count in groups):
        raise ValueError("invalid native variant group spans")
    items, packages = [], []
    for item in capture["items"]:
        if "error" in item:
            raise ValueError("unverified character item: " + item["name"])
        if item["type"] == "cha":
            raw = base64.b64decode(item["package_base64"], validate=True)
            if len(raw) != 40 or raw[2] > 1 or raw[3] > 3:
                raise ValueError("unsupported character package")
            packages.append(dict(id=item["item_id"], gender=raw[2], skin=raw[3],
                                 defaults=list(struct.unpack_from("<12H", raw, 8)),
                                 allowed=item["members"]))
            continue
        kind = item["kind"]
        if kind not in KINDS:
            raise ValueError("unsupported wearable kind")
        group = GROUPS[KINDS.index(kind)]
        start, count = groups[GROUPS.index(group)]
        matches = [r for r in records[start:start+count] if r[2] == native_hash(item["name"])]
        if len(matches) != 1:
            raise ValueError("ambiguous native character item hash")
        variants = []
        for v in capture["variants"]:
            if v["group"] == group and v["index"] == matches[0][0]:
                selector_bytes = base64.b64decode(v["selectors_base64"], validate=True)
                if len(selector_bytes) != 6:
                    raise ValueError("invalid variant selector")
                variants.append(dict(mask=struct.unpack_from("<I", selector_bytes, 2)[0], model=v["model"]))
        # Some catalog wearables have no model variant in this build. Keep
        # their identity; runtime must reject selecting them, not invent art.
        items.append(dict(id=item["item_id"], kind=kind, native_index=matches[0][0], variants=variants))
    return dict(schema_version=1, client_sha256=CLIENT_SHA256,
                current_character_offset=selector["offset"], slot_offsets=offsets,
                items=items, packages=packages, packing_bits=packing_bits)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("capture", "assets", "output"):
        parser.add_argument("--"+name, type=Path, required=True)
    args = parser.parse_args()
    raw, assets = args.capture.read_bytes(), args.assets.read_bytes()
    result = prepare(json.loads(raw), json.loads(assets))
    result["sources_sha256"] = [hashlib.sha256(raw).hexdigest(), hashlib.sha256(assets).hexdigest()]
    with args.output.open("x") as target:
        json.dump(result, target, indent=2)
        target.write("\n")
    print(json.dumps({"output":str(args.output),"packages":len(result["packages"]),"items":len(result["items"])}))


if __name__ == "__main__":
    main()
