#!/usr/bin/env python3
"""Prepare a TDM map using its own initialized resource captures.

Account data is never opened. Shared game settings come from a template;
map-specific collision, CRC, arsenal, spawns and animation IDs are rebound.
"""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
from clientlib import REPO

from prepare_game_state import CLIENT_SHA256, compute
from prepare_bot_locomotion import actor_animations, prepare as prepare_locomotion
from prepare_bot_roster import NativeBotCharacters, prepare as prepare_roster
from prepare_map_hud import map_entities, minimap


def team_spawns(capture, map_name, minimum=6):
    teams = [[], []]
    classes = [b"mp_tdm_spawn_axis_start", b"mp_tdm_spawn_allies_start"]
    for fields in map_entities(capture, map_name):
        if fields.get(b"classname") not in classes:
            continue
        position = [float(v) for v in fields[b"origin"].split()]
        if len(position) != 3 or any(not math.isfinite(v) or not -32768 <= v <= 32767 for v in position):
            raise ValueError("invalid map spawn coordinates")
        teams[classes.index(fields[b"classname"])].append(position)
    if any(not minimum <= len(t) <= 128 for t in teams):
        raise ValueError("map needs a captured start for every active player")
    return teams


def team_spawn_angles(capture, map_name, minimum=6):
    teams = [[], []]
    classes = [b"mp_tdm_spawn_axis_start", b"mp_tdm_spawn_allies_start"]
    for fields in map_entities(capture, map_name):
        if fields.get(b"classname") not in classes:
            continue
        angles = ([float(v) for v in fields[b"angles"].split()] if b"angles" in fields
                  else [0.0, float(fields.get(b"angle", b"0")), 0.0])
        if len(angles) != 3 or any(not math.isfinite(v) or abs(v) > 360000 for v in angles):
            raise ValueError("invalid map spawn angles")
        teams[classes.index(fields[b"classname"])].append([int(v * 65536 / 360) & 65535 for v in angles])
    if any(not minimum <= len(t) <= 128 for t in teams):
        raise ValueError("map needs a captured heading for every active player")
    return teams


def native_maps():
    registry = json.loads((REPO / 'fixtures/protocol/map-registry-3.3.3.3.json').read_text())
    if (registry['schema_version'] != 1 or registry['client_sha256'] != CLIENT_SHA256 or
            registry['resource_preparation_claimed'] is not False):
        raise ValueError('invalid captured native map registry')
    return [m['name'] for m in registry['maps']]


def prepare(template, map_name, constants, entities, collision, arsenal, characters,
            material_capture_sha256=None, native_characters=None):
    if map_name not in native_maps() or template["client_sha256"] != CLIENT_SHA256:
        raise ValueError("unsupported map/template identity")
    summary = compute(constants, "m4_mp", True, True, "viewhands_tf141")
    if not summary["observed_crc_verified"]:
        raise ValueError("TDM requires an initialized map CRC capture")
    crc = summary["ncs_crc"]
    for capture in (collision, arsenal, characters):
        if capture.get("read_only") is not True or capture.get("schema_version") != 1 or capture.get("client_sha256") != CLIENT_SHA256 or capture.get("ncs_crc") != crc:
            raise ValueError("map/collision/arsenal/character identity mismatch")
    if collision["map"] != "maps/mp/" + map_name + ".d3dbsp" or not collision["brushes"]:
        raise ValueError("collision belongs to another or empty map")
    registry = json.loads((REPO / "fixtures/protocol/map-registry-3.3.3.3.json").read_text())
    native = next(m for m in registry["maps"] if m["name"] == map_name)
    player_count = min(12, native["maximum_players"])
    if player_count not in (4, 12):
        raise ValueError("unsupported native TDM player count")
    teams = team_spawns(entities, map_name, player_count // 2)
    state = copy.deepcopy(template)
    state.update(summary)
    state.update(map=map_name, mode="war", zone="mp", playlist=0, spawn_origin=teams[1][0])
    state["minimap"] = minimap(entities, map_name, characters, material_capture_sha256)
    state["match_config"].update(spawns=teams, spawn_angles=team_spawn_angles(entities, map_name, player_count // 2),
        roster=[], models=[], player_count=player_count,
        teams=[2]*(player_count//2)+[1]*(player_count//2)+[0]*(12-player_count))
    # Rebind the human skeleton and generic nodes by names in this capture.
    models = {m["name"]: m for m in characters["models"]}
    body = models["mp_fullbody_juggernaut_explosive_so"]
    animation = state["training_enemy"]
    animation.update(model=body["model"], origin=teams[0][0], bounds_min=body["bounds_min"], bounds_max=body["bounds_max"])
    indices = {}
    for a in actor_animations(characters):
        indices.setdefault(a["name"].lower(), set()).add(a["index"])
    for field, name in (("idle_animation", "pb_stand_alert"), ("fire_animation", "pt_stand_shoot_auto"),
                        ("death_animation", "pb_stand_death_chest_blowback")):
        found = indices.get(name, set())
        if len(found) != 1:
            raise ValueError("required human animation absent or ambiguous")
        animation[field] = next(iter(found))
    prepare_locomotion(state, characters, arsenal, CLIENT_SHA256)
    result = prepare_roster(state, characters, arsenal, stream_outfits=True,
                            native_characters=native_characters)
    if state.get('appearance_file'):
        # Native inherited model slots differ by map. Player composition starts
        # after this map's inherited and explicitly registered Bot model IDs.
        state['appearance_first_model']=max([m['model'] for m in characters['models']]+
            [i for i,_ in state['match_config']['models']])+1
    result.update(map=map_name, ncs_crc=crc, spawns=[len(t) for t in teams])
    return state, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--template", type=Path, required=True)
    parser.add_argument("--map", choices=native_maps(), required=True)
    for name in ("constants", "entities", "collision", "arsenal", "characters", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--native-characters", type=Path,
                        help="retained native outfit bindings for Bot replay sleeves/gloves")
    args = parser.parse_args()
    loaded = {n: json.loads(getattr(args, n).read_text()) for n in ("constants", "entities", "collision", "arsenal", "characters")}
    state, result = prepare(json.loads(args.template.read_text()), args.map,
        material_capture_sha256=hashlib.sha256(args.characters.read_bytes()).hexdigest(),
        native_characters=NativeBotCharacters(json.loads(args.native_characters.read_text()))
                          if args.native_characters else None, **loaded)
    state["capture_sha256"] = hashlib.sha256(args.constants.read_bytes()).hexdigest()
    state["map_entities_sha256"] = hashlib.sha256(args.entities.read_bytes()).hexdigest()
    state["training_enemy"]["source_sha256"] = hashlib.sha256(args.characters.read_bytes()).hexdigest()
    state["arsenal_file"] = str(args.arsenal.resolve())
    state["collision_file"] = str(args.collision.resolve())
    with args.output.open("x") as out:
        json.dump(state, out, indent=2)
        out.write("\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
