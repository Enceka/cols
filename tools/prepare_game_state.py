#!/usr/bin/env python3
"""Recompute target NCS CRC from a read-only resource capture; no client writes."""
import argparse
import base64
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import zlib

CLIENT_SHA256 = "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b"


def strings(values):
    result = [base64.b64decode(v, validate=True) for v in values]
    if any(b"\0" in v or len(v) > 8192 for v in result):
        raise ValueError("invalid NCS string")
    return result


def compute(capture):
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
    for key, values in sorted(parsed):
        category = key[1]
        if category == 15:
            continue  # FX tags do not contribute, target 0x584ed0
        # Dedup only against EARLIER assets of the same category. Repeated
        # strings inside this asset still contribute (0x584d80).
        kept = [v for v in values if v not in previous[category]]
        for value in kept:
            crc = zlib.crc32(value, crc)
        previous[category].update(kept)
        count += len(kept)
        included += 1
    if capture["initialized"] and crc != capture["checksum"]:
        raise ValueError("computed CRC differs from initialized client")
    return {"ncs_crc": crc, "common_crc": seed, "constant_count": count,
            "asset_count": included, "observed_crc_verified": bool(capture["initialized"])}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--output", type=Path, help="prepare opt-in mp_afghan/war training gamestate")
    args = parser.parse_args()
    raw = args.capture.read_bytes()
    if len(raw) > 32 * 1024 * 1024:
        parser.error("capture size limit")
    capture = json.loads(raw)
    summary = compute(capture)
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
