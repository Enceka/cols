"""Bind the native compass to captured map entities (3.3.3.3).

Corner projection follows maps/mp/_compass.gsc; wire CS2037/2038 is verified
against this client's 0x43d770/0x43d500, independently of Alpha's CS indices.
"""
import argparse
import base64
import hashlib
import json
import math
import re
from pathlib import Path

CLIENT_SHA256 = "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b"
# Verified against loaded 3.3.3.3 material textures. Crash's generated
# tropical name resolves to the default image; its actual image uses _dlc.
MATERIALS = {
    "mp_afghan": "compass_map_mp_afghan",
    "mp_killhouse_sh": "compass_map_mp_killhouse_sh",
    "mp_crash_tropical": "compass_map_mp_crash_dlc",
}


def map_entities(capture, map_name):
    if capture.get("schema_version") != 1 or capture.get("read_only") is not True or capture.get("client_sha256") != CLIENT_SHA256:
        raise ValueError("map entity capture identity mismatch")
    records = [m for m in capture["maps"] if m["name"] == "maps/mp/" + map_name + ".d3dbsp"]
    if len(records) != 1:
        raise ValueError("requested map has no unique entity resource")
    record = records[0]
    data = base64.b64decode(record["entity_string_base64"], validate=True)
    if len(data) != record["byte_count"] or len(data) > 4 * 1024 * 1024:
        raise ValueError("map entity size mismatch")
    entities = []
    for block in re.findall(rb'\{([^{}]*)\}', data):
        pairs = re.findall(rb'"([^"\r\n]*)"\s+"([^"\r\n]*)"', block)
        fields = dict(pairs)
        if len(fields) != len(pairs):
            raise ValueError("duplicate map entity field")
        variants = fields.get(b"script_specialopsname", b"")
        if variants and map_name.encode("ascii") not in re.split(rb"[\s,]+", variants.strip()):
            continue
        entities.append(fields)
    return entities


def minimap(capture, map_name):
    if map_name not in MATERIALS:
        raise ValueError("unsupported minimap material")
    entities = map_entities(capture, map_name)
    worlds = [e for e in entities if e.get(b"classname") == b"worldspawn"]
    corners = [e for e in entities if e.get(b"targetname") == b"minimap_corner"]
    if len(worlds) != 1 or len(corners) != 2:
        raise ValueError("minimap needs one worldspawn and exactly two corners")
    yaw = float(worlds[0].get(b"northyaw", b"0"))
    if not math.isfinite(yaw):
        raise ValueError("invalid minimap north yaw")
    yaw %= 360
    north = [math.cos(math.radians(yaw)), math.sin(math.radians(yaw))]
    west = [-north[1], north[0]]
    projected = []
    for corner in corners:
        origin = [float(v) for v in corner[b"origin"].split()]
        if len(origin) != 3 or any(not math.isfinite(v) or abs(v) > 32768 for v in origin):
            raise ValueError("invalid minimap corner")
        projected.append([sum(origin[i] * basis[i] for i in range(2)) for basis in (north, west)])
    low = [min(p[i] for p in projected) for i in range(2)]
    high = [max(p[i] for p in projected) for i in range(2)]
    if any(high[i] - low[i] <= 1 for i in range(2)):
        raise ValueError("degenerate minimap bounds")
    point = lambda p: [round(p[0] * north[i] + p[1] * west[i], 5) for i in range(2)]
    return dict(material=MATERIALS[map_name], north_yaw=yaw,
                upper_left=point(high), lower_right=point(low))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("state", "entities", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    state = json.loads(args.state.read_text())
    if state["client_sha256"] != CLIENT_SHA256:
        raise ValueError("prepared client identity mismatch")
    raw = args.entities.read_bytes()
    state["minimap"] = minimap(json.loads(raw), state["map"])
    state["map_entities_sha256"] = hashlib.sha256(raw).hexdigest()
    with args.output.open("x") as output:
        json.dump(state, output, indent=2)
        output.write("\n")
    print(json.dumps(state["minimap"]))


if __name__ == "__main__":
    main()
