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
# These aliases still require the current map's captured texture proof.
CAPTURED_ALIASES = {
    "mp_overgrown": "compass_map_mp_overgrown_dlc",
    "mp_ambush_sh": "compass_map_mp_convoy",
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


def minimap(capture, map_name, material_capture=None, material_capture_sha256=None):
    material = MATERIALS.get(map_name)
    proof = None
    if material is None:
        if (material_capture is None or material_capture.get('schema_version') != 1 or
                material_capture.get('read_only') is not True or
                material_capture.get('client_sha256') != CLIENT_SHA256 or
                material_capture.get('map') != 'maps/mp/' + map_name + '.d3dbsp'):
            raise ValueError('new minimap requires this map\'s read-only material capture')
        material = CAPTURED_ALIASES.get(map_name, 'compass_map_' + map_name)
        record = material_capture.get('map_hud_materials', {}).get(material, {})
        dimensions = [[t['width'], t['height']] for t in record.get('textures', [])]
        if (record.get('found') is not True or record.get('render_default') is not False or
                not dimensions or len(dimensions) > 32 or
                not any(all(type(v) is int and 64 <= v <= 8192 for v in d) for d in dimensions)):
            raise ValueError('minimap material is absent, default or has no real image')
        crc = material_capture.get('ncs_crc')
        if type(crc) is not int or not 0 <= crc <= 0xffffffff:
            raise ValueError('material capture CRC invalid')
        digest = material_capture_sha256 or hashlib.sha256(
            json.dumps(material_capture, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        if not re.fullmatch('[a-fA-F0-9]{64}', digest):
            raise ValueError('material capture hash invalid')
        proof = dict(client_sha256=CLIENT_SHA256,capture_sha256=digest,
                     map=material_capture['map'],material=material,ncs_crc=crc,
                     render_default=False,texture_dims=dimensions)
    result = dict(material=material,**minimap_geometry(capture,map_name))
    if proof is not None:
        result['material_proof'] = proof
    return result


def minimap_geometry(capture,map_name,required_aspect_ratio=1.0):
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
    if (not math.isfinite(required_aspect_ratio)
            or not 0 <= required_aspect_ratio <= 16):
        raise ValueError("invalid required minimap aspect ratio")
    # The two entities mark opposite playable corners, not the complete image
    # rectangle. Preserve their centre and expand the shorter projected axis
    # to the map image aspect. The standard MP setup uses1;0 permits the
    # explicitly unrestricted case. Shrinking would remove captured terrain.
    if required_aspect_ratio > 0:
        north_span, west_span = (high[i] - low[i] for i in range(2))
        if west_span / north_span < required_aspect_ratio:
            axis, span = 1, north_span * required_aspect_ratio
        else:
            axis, span = 0, west_span / required_aspect_ratio
        centre = (high[axis] + low[axis]) * 0.5
        low[axis], high[axis] = centre - span * 0.5, centre + span * 0.5
    point = lambda p: [round(p[0] * north[i] + p[1] * west[i], 5) for i in range(2)]
    return dict(north_yaw=yaw,upper_left=point(high),lower_right=point(low))


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
