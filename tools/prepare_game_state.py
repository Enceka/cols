#!/usr/bin/env python3
"""Recompute target NCS CRC from a read-only resource capture; no client writes."""
import argparse
import base64
from collections import defaultdict
import hashlib
import json
import math
from pathlib import Path
import re
import zlib

CLIENT_SHA256 = "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b"


def strings(values):
    result = [base64.b64decode(v, validate=True) for v in values]
    if any(b"\0" in v or len(v) > 8192 for v in result):
        raise ValueError("invalid NCS string")
    return result


def compute(capture, weapon_name=None, compose=False, viewhands=False):
    if (capture.get("schema_version") != 1 or capture.get("read_only") is not True
            or capture.get("client_sha256") != CLIENT_SHA256):
        raise ValueError("unsupported resource capture identity")
    names = strings(capture["network_dvars_base64"])
    if not 1 <= len(names) <= 266 or len(set(names)) != len(names):
        raise ValueError("invalid registered network dvar names")
    seed = zlib.crc32(b"".join(names))
    if seed != capture["common_checksum"]:
        raise ValueError("registered names do not reproduce common CRC")
    prefix = capture["localized_prefix"].encode("ascii").lower()
    if not prefix or not prefix.endswith(b"_"):
        raise ValueError("invalid language prefix")
    assets = capture["assets"]
    if not 1 <= len(assets) <= 512:
        raise ValueError("invalid asset count")
    parsed = []
    seen_keys = set()
    for asset in assets:
        if "error" in asset:
            raise ValueError("incomplete resource capture: " + asset["error"])
        name = asset["name"].encode("ascii")
        category, source = asset["category"], asset["source"]
        values = strings(asset["strings_base64"])
        if len(values) != asset["count"] or len(values) > 8192:
            raise ValueError("resource count mismatch")
        if (name, category, source, len(values)) == (b"void", 21, 4, 0):
            continue  # exact DB default placeholder, not a network category
        if not 0 <= category <= 20 or not 0 <= source <= 3 or b"\0" in name:
            raise ValueError("invalid resource header")
        key = (source, category, len(name) > 12 and name[8:].lower().startswith(prefix), name.lower())
        if key in seen_keys:
            raise ValueError("ambiguous resource sort order")
        seen_keys.add(key)
        parsed.append((key, values))
    # 0x5855d0 builds the seed; 0x5850b0 resets CRC to it AFTER dynamic
    # categories 18..20 have been linked. Never checksum dynamic lists twice.
    previous = defaultdict(set)
    for asset in capture["linked"]:
        if "error" in asset:
            raise ValueError("incomplete linked resource capture")
        if asset["category"] in (18, 19, 20):
            previous[asset["category"]].update(strings(asset["strings_base64"]))
    crc, count, included = seed, 0, 0
    weapons = []
    models = []
    for key, values in sorted(parsed):
        category = key[1]
        if category == 15:
            continue  # FX tags do not contribute, target 0x584ed0
        # Dedup only against EARLIER assets of the same category. Repeated
        # strings inside this asset still contribute (0x584d80).
        kept = [v for v in values if v not in previous[category]]
        if category == 13:
            weapons.extend(kept)
        if category == 0:
            models.extend(kept)
        for value in kept:
            crc = zlib.crc32(value, crc)
        previous[category].update(kept)
        count += len(kept)
        included += 1
    if capture["initialized"] and crc != capture["checksum"]:
        raise ValueError("computed CRC differs from initialized client")
    result = {"ncs_crc": crc, "common_crc": seed, "constant_count": count,
              "asset_count": included, "observed_crc_verified": bool(capture["initialized"])}
    if weapon_name is not None:
        if weapon_name != "m4_mp" or weapons.count(weapon_name.encode("ascii")) != 1:
            raise ValueError("expected one basic M4 in target weapon constants")
        index = weapons.index(weapon_name.encode("ascii")) + 1
        if not 1 <= index < 900:
            raise ValueError("weapon index outside target category")
        result["bootstrap_weapon"] = {"name": weapon_name, "index": index}
        if compose:
            if len(weapons)+1>=900:raise ValueError("no free bootstrap weapon slot")
            result["bootstrap_weapon"].update(base_index=index,index=len(weapons)+1,
                composed_name="** %d 3 0" % index)
        if viewhands:
            name=b"viewhands_player_tf141"
            if models.count(name)!=1:raise ValueError("expected one TF141 viewhands model")
            result["bootstrap_weapon"]["viewmodel_index"]=models.index(name)+1
    return result


def spawn_origin(capture):
    if (capture.get("schema_version") != 1 or capture.get("read_only") is not True
            or capture.get("client_sha256") != CLIENT_SHA256):
        raise ValueError("unsupported map capture identity")
    maps = [m for m in capture["maps"] if m["name"] == "maps/mp/mp_afghan.d3dbsp"]
    if len(maps) != 1:
        raise ValueError("expected one mp_afghan map entity resource")
    entry = maps[0]
    text = base64.b64decode(entry["entity_string_base64"], validate=True)
    if len(text) != entry["byte_count"] or len(text) > 4 * 1024 * 1024:
        raise ValueError("map entity byte count mismatch")
    for block in re.findall(rb'\{([^{}]*)\}', text):
        pairs = re.findall(rb'"([^"\r\n]*)"\s+"([^"\r\n]*)"', block)
        values = dict(pairs)
        if values.get(b"classname") != b"mp_tdm_spawn_allies_start":
            continue
        if len(pairs) != len(values):
            raise ValueError("duplicate spawn entity key")
        origin = [float(v) for v in values[b"origin"].split()]
        if len(origin) != 3 or any(not math.isfinite(v) or abs(v)>1_000_000 for v in origin):
            raise ValueError("invalid training spawn position")
        return origin
    raise ValueError("training spawn not found in map resource")


def weapon_ammo(raw, crc, weapon):
    capture=json.loads(raw)
    if (capture.get("schema_version") != 1 or capture.get("read_only") is not True
            or capture.get("client_sha256") != CLIENT_SHA256 or capture.get("ncs_crc") != crc):
        raise ValueError("weapon runtime capture identity/CRC mismatch")
    w=capture.get("composed_weapon") if "composed_name" in weapon else capture["weapon"]
    if not w or w["name"] != weapon.get("composed_name",weapon["name"]) or w["index"] != weapon["index"]:
        raise ValueError("runtime weapon differs from net constants")
    if (type(w["ammo_key"]) is not int or type(w["clip_key"]) is not int
            or not 0 <= w["ammo_key"] <= 1023 or not 0 <= w["clip_key"] <= 1022):
        raise ValueError("unsupported weapon ammo keys")
    return {"reserve_key":w["ammo_key"],"clip_key":w["clip_key"],
            "clip":30,"reserve":90,"source_sha256":hashlib.sha256(raw).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--output", type=Path, help="prepare opt-in mp_afghan/war training gamestate")
    parser.add_argument("--map-entities", type=Path, help="include a map-derived training spawn in the snapshot probe")
    parser.add_argument("--weapon", choices=["m4_mp"], help="derive the basic M4 wire index for equipment bootstrap")
    parser.add_argument("--weapon-runtime", type=Path, help="registered ammo keys for a diagnostic 30/90 ammunition seed")
    parser.add_argument("--compose-weapon", action="store_true", help="register a model-composed M4 in the next unused weapon slot")
    parser.add_argument("--viewhands", action="store_true", help="bind the inherited TF141 first-person hand model")
    args = parser.parse_args()
    raw = args.capture.read_bytes()
    if len(raw) > 32 * 1024 * 1024:
        parser.error("capture size limit")
    capture = json.loads(raw)
    if args.weapon and not args.map_entities:
        parser.error("weapon bootstrap requires map-entities")
    if args.compose_weapon and not args.weapon:parser.error("compose-weapon requires weapon")
    if args.viewhands and not args.weapon:parser.error("viewhands requires weapon")
    summary = compute(capture, args.weapon, args.compose_weapon,args.viewhands)
    if args.weapon_runtime:
        if not args.weapon:
            parser.error("weapon-runtime requires weapon")
        weapon_raw=args.weapon_runtime.read_bytes()
        if len(weapon_raw)>4096:parser.error("weapon capture size limit")
        summary["bootstrap_weapon"]["ammo"]=weapon_ammo(weapon_raw,summary["ncs_crc"],summary["bootstrap_weapon"])
    if args.map_entities:
        map_raw = args.map_entities.read_bytes()
        if len(map_raw) > 16 * 1024 * 1024:
            parser.error("map capture size limit")
        summary.update(spawn_origin=spawn_origin(json.loads(map_raw)),
                       map_entities_sha256=hashlib.sha256(map_raw).hexdigest())
    if args.output:
        if capture["connection_state"] != 7:
            parser.error("preparation requires the training map's state-7 resource capture")
        prepared = {"schema_version": 1, "client_sha256": CLIENT_SHA256,
                    "capture_sha256": hashlib.sha256(raw).hexdigest(),
                    "map": "mp_afghan", "mode": "war", "zone": "mp", "playlist": 0,
                    **summary}
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(prepared, indent=2) + "\n")
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
