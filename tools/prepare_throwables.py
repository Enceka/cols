#!/usr/bin/env python3
"""Derive 3.3.3.3 offhand definitions and smoke optical curves from read-only captures."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import struct

from clientlib import manifest
from schema_fields import flatten

SUPPORTED = {"frag_grenade_mp", "fragholiday_grenade_mp", "fragshort_grenade_mp",
             "fragtech_grenade_mp", "semtex_mp", "stielhandgranate_mp", "smoke_grenade_mp", "flash_grenade_mp", "concussion_grenade_mp", "tabun_gas_mp", "emp_grenade_mp"}


def smoke_effect(effect):
    if effect["name"] != "props/american_smoke_grenade_mp" or effect["element_stride"] != 260:
        raise ValueError("unexpected native smoke effect")
    raw = base64.b64decode(effect["elements_base64"], validate=True)
    word = lambda b, o: struct.unpack_from("<I", b, o)[0]
    elements = [raw[i:i + 260] for i in range(0, len(raw), 260)]
    model = [b for b in elements if b[0xb0] == 9]
    if len(model) != 1 or effect["looping_count"] != 0:
        raise ValueError("smoke model lifetime is not established")
    lifetime = sum(word(model[0], o) for o in (0x28, 0x2c, 0x30, 0x34))
    emitters = []
    for index, runner in enumerate(elements):
        if runner[0xb0] != 14:
            continue
        if word(runner, 0x2c) != 0:
            raise ValueError("randomized smoke runner delay needs separate recovery")
        delay = word(runner, 0x28)
        for child in effect["elements"][index]["children"]:
            if child["name"] != "smoke/smoke_grenade_11sec_mp":
                continue
            data = base64.b64decode(child["elements_base64"], validate=True)
            for entry in child["elements"]:
                i = entry["index"]
                b = data[i * 260:(i + 1) * 260]
                if not word(b, 0) & 0x2000:
                    continue
                if i < child["looping_count"] or i >= child["looping_count"] + child["one_shot_count"]:
                    raise ValueError("looped optical smoke needs separate recovery")
                visual = base64.b64decode(entry["visual_samples_base64"], validate=True)
                if len(visual) != (b[0xb3] + 1) * 64:
                    raise ValueError("incomplete smoke visual samples")
                samples = []
                for at in range(0, len(visual), 64):
                    if visual[at + 3] != visual[at + 0x23]:
                        raise ValueError("randomized alpha needs native color-range recovery")
                    samples.append([struct.unpack_from("<f", visual, at + 12)[0],
                                    struct.unpack_from("<f", visual, at + 44)[0], visual[at + 3] / 255.0])
                emitters.append({"count": [word(b, 4), word(b, 8)],
                                 "delay_msec": [delay + word(b, 0x28), word(b, 0x2c)],
                                 "life_msec": [word(b, 0x30), word(b, 0x34)], "samples": samples})
    if len(emitters) != 2 or lifetime != 60001:
        raise ValueError("captured smoke graph differs from the recovered consumers")
    return {"effect": effect["name"], "lifetime_msec": lifetime, "emitters": emitters}


def prepare(assets_path, registry_path):
    assets = json.loads(assets_path.read_text())
    raw = registry_path.read_bytes()
    registry = json.loads(raw)
    client = next(f["sha256"] for f in manifest()["files"] if f["path"] == "CODOL/codoMP_client_shipRetail.exe")
    for capture in (assets, registry):
        if capture.get("client_sha256") != client or capture.get("read_only") is not True:
            raise ValueError("expected fixed target read-only capture")
    table = assets["tables"]["mp/coMarketplaceItems.csv"]
    cells = table["cells_base64"]
    if table["columns"] != 12 or len(cells) != table["rows"] * 12:
        raise ValueError("incomplete equipment marketplace")
    market = {}
    for i in range(0, len(cells), 12):
        item, name, category = [base64.b64decode(c, validate=True).decode("ascii") for c in cells[i:i + 3]]
        if category == "equ":
            market.setdefault(name, []).append(int(item))
    fields = flatten(assets["schemas"]["mp/playerdata.def"]["definitions"][0])
    offsets = []
    for index in range(60):
        leaves = [fields["weaponClasses[%d].%sDoubleAmmo" % (index, name)] for name in ("lethal", "tactical")]
        if any(f["type"] != 2 or f["width"] != 1 or f["bit"] is None for f in leaves):
            raise ValueError("unexpected grenade count flag layout")
        offsets.append([[f["offset"], f["bit"]] for f in leaves])
    result = []
    for weapon in registry["offhand_definitions"]:
        name = weapon["name"]
        if name not in SUPPORTED:
            continue
        definition = base64.b64decode(weapon["definition_base64"], validate=True)
        if len(definition) != 0xe28 or name not in market:
            raise ValueError("incomplete grenade definition")
        word = lambda offset: struct.unpack_from("<I", definition, offset)[0]
        number = lambda offset: struct.unpack_from("<f", definition, offset)[0]
        if word(0x47c) != 1 or word(0x480) != 6:
            raise ValueError("offhand weapon type/category differs")
        if definition[0xd7a] not in (0, 1):
            raise ValueError("invalid native cooking flag")
        result.append({"name": name, "index": weapon["index"], "items": market[name],
                       "type": word(0x47c), "class": word(0x480), "offhand_class": word(0x490),
                       "clip_key": word(0x6c0), "capacity": word(0xd08),
                       "prepare_msec": word(0x70c), "throw_msec": word(0xd10), "throw_delay_msec": word(0x6f0),
                       "fuse_msec": word(0x7ac), "cook": bool(definition[0xd7a]), "explosion_kind": word(0x914),
                       "radius": word(0x8d8), "inner_damage": word(0x8e0), "outer_damage": word(0x8e4),
                       "speed": word(0x8f4), "speed_up": word(0x8f8), "speed_forward": word(0x8fc),
                       "stickiness": word(0x928), "parallel": [number(0x934 + 4 * i) for i in range(31)],
                       "perpendicular": [number(0x9b0 + 4 * i) for i in range(31)]})
        if name in ("flash_grenade_mp", "concussion_grenade_mp"):
            result[-1]["tactical"] = "flash" if name == "flash_grenade_mp" else "concussion"
        if name == "smoke_grenade_mp":
            result[-1]["smoke"] = smoke_effect(weapon["explosion_effect"])
        if name == "tabun_gas_mp":
            if word(0x918) != 0 or weapon.get("explosion_effect") is not None:
                raise ValueError("gas grenade unexpectedly has a native explosion effect")
            result[-1]["gas"] = True
        if name == "emp_grenade_mp":
            if weapon["explosion_effect"]["name"] != "explosions/emp_grenade":
                raise ValueError("native EMP effect differs")
            result[-1]["emp"] = True
    if {r["name"] for r in result} != SUPPORTED:
        raise ValueError("supported grenade set is incomplete")
    return {"schema_version": 1, "client_sha256": client, "capture_sha256": hashlib.sha256(raw).hexdigest(),
            "double_ammo_offsets": offsets, "grenades": result}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--registry", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.assets, args.registry)
    with args.output.open("x") as output:
        json.dump(result, output, indent=2, allow_nan=False)
        output.write("\n")
    print(json.dumps({"output": str(args.output), "grenades": len(result["grenades"])}))


if __name__ == "__main__":
    main()
