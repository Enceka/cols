#!/usr/bin/env python3
"""Bind the local TDM M4 to captured3.3 native animation rules and indices.

Reads immutable exports; writes a new preparation and never touches accounts
or client memory. Raw native assets stay in ignored research directories.
"""
import argparse
import base64
import hashlib
import json
import re
import struct
import zlib
from pathlib import Path

from clientlib import manifest


def actor_animations(capture):
    # Both context words are exported by the fixed-version reader. The BG
    # context may not exist until the first player snapshot; the initialized
    # renderer already owns the matching character animation bank.
    animations=capture['animations']
    primary=[a for a in animations if a.get('source',0x07817b80)==0x07817b80]
    renderer=[a for a in animations if a.get('source')==0x07817b84]
    if primary:
        return primary
    if renderer:
        return renderer
    raise ValueError('captured character animation bank unavailable')


def rawfile(capture, name):
    record = capture["animation_rules"][name]
    if record.get("found") is not True or record.get("name") != name:
        raise ValueError("native animation rawfile is not captured: " + name)
    size = record["length"]
    if not 0 < size <= 2 * 1024 * 1024:
        raise ValueError("animation rawfile size limit")
    data = base64.b64decode(record["buffer_base64"], validate=True)
    if record["compressed_length"]:
        if len(data) != record["compressed_length"]:
            raise ValueError("animation compressed size mismatch")
        decoder = zlib.decompressobj()
        data = decoder.decompress(data, size + 1)
        if not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
            raise ValueError("invalid animation compression stream")
    if len(data) != size:
        raise ValueError("animation rawfile length mismatch")
    return data.rstrip(b"\0")


def blocks(text):
    """Only immediate brace blocks; headers retain native selection order."""
    cursor = 0
    while True:
        opening = text.find("{", cursor)
        if opening < 0:
            return
        header = text[cursor:opening].strip().splitlines()
        if not header:
            raise ValueError("animation block lacks a condition")
        level, at = 1, opening + 1
        while at < len(text) and level:
            level += (text[at] == "{") - (text[at] == "}")
            at += 1
        if level:
            raise ValueError("unclosed animation block")
        yield header[-1].strip().lower(), text[opening + 1:at - 1]
        cursor = at


def conditions_match(header, context):
    for condition in header.split(","):
        words = condition.split()
        if words == ["default"]:
            continue
        if not words:
            raise ValueError("empty animation condition")
        key, *values = words
        if key in {"akimbo", "mounted", "riotshieldnext", "perk", "underwater"}:
            return False  # The fixed TDM rifle has none of these states.
        actual = context.get(key)
        if actual is None:
            return False
        if "not" in values:
            at = values.index("not")
            allowed, excluded = values[:at], values[at + 1:]
        else:
            allowed, excluded = values, []
        allowed = set(allowed) - {"and"}
        if actual in excluded:
            return False
        if "all" not in allowed and actual not in allowed:
            if not (key == "movetype" and "moving" in allowed and actual in {"walk", "walkbk", "run", "runbk", "sprint"}):
                return False
    return True


def select_action(rules, name, context, with_duration=False):
    groups = [body for header, body in blocks(rules) if header == name]
    if len(groups) != 1:
        raise ValueError("native animation state missing or duplicated: " + name)
    for header, body in blocks(groups[0]):
        if conditions_match(header, context):
            actions = re.findall(r"(?m)^\s*(both|legs|torso)\s+([a-zA-Z0-9_]+)\b([^\n]*)", body)
            if len(actions) > 1:
                raise ValueError("random/multiple native actions require another policy")
            if not actions:
                return None
            kind, animation, suffix = actions[0]
            duration = re.search(r"\bduration\s+(\d+)\b", suffix)
            return (kind, animation, int(duration[1]) if duration else None) if with_duration else (kind, animation)
    raise ValueError("no matching native animation rule: " + name)


def prepare(state, capture, arsenal, client_sha):
    if (state["client_sha256"] != client_sha or capture["client_sha256"] != client_sha
            or capture["schema_version"] != 1 or capture["read_only"] is not True
            or capture["ncs_crc"] != state["ncs_crc"] or arsenal["ncs_crc"] != state["ncs_crc"]
            or arsenal["client_sha256"] != client_sha or arsenal["read_only"] is not True):
        raise ValueError("animation/map/weapon capture identity mismatch")
    script_bytes = rawfile(capture, "mp/playeranim.script")
    script = re.sub(r"//[^\n]*|/\*.*?\*/", "", script_bytes.decode("ascii"), flags=re.S)
    combat = [body for header, body in blocks(script) if header == "state combat"]
    if len(combat) != 1:
        raise ValueError("native combat animation state missing")
    weapons = [w for w in arsenal["weapons"] if w["name"] == "m4_mp"]
    if len(weapons) != 1:
        raise ValueError("fixed TDM rifle definition missing")
    definition = base64.b64decode(weapons[0]["definition_base64"], validate=True)
    if len(definition) != 0xe28:
        raise ValueError("native weapon layout mismatch")
    anim_type, weapon_class = struct.unpack_from("<I", definition, 0x478)[0], struct.unpack_from("<I", definition, 0x480)[0]
    types = rawfile(capture, "mp/playeranimtypes.txt").decode("ascii").split()
    if anim_type >= len(types) or types[anim_type] != "autorifle" or weapon_class != 0:
        raise ValueError("native M4 anim type/class differs from supported rifle")
    context = {"playeranimtype": "autorifle", "playeranimtypeprimary": "autorifle", "weaponclass": "rifle",
               "weapon_position": "hip", "strafing": "not", "movetype": "idle"}
    indices = {}
    for animation in actor_animations(capture):
        name, index = animation["name"].lower(), animation["index"]
        if not 0 < index < 1024:
            raise ValueError("captured animation index outside native range")
        indices.setdefault(name, set()).add(index)

    def identity(name):
        found = indices.get(name.lower(), set())
        if len(found) != 1:
            raise ValueError("selected human animation missing or ambiguous: " + name)
        return next(iter(found))

    bindings = {}
    for field, move, strafe in [
        ("walk_animation", "walk", "not"), ("walk_back_animation", "walkbk", "not"),
        ("walk_left_animation", "walk", "left"), ("walk_right_animation", "walk", "right"),
        ("run_forward", "run", "not"), ("run_back", "runbk", "not"),
        ("run_left", "run", "left"), ("run_right", "run", "right"), ("sprint", "sprint", "not"),
    ]:
        action = select_action(combat[0], move, dict(context, movetype=move, strafing=strafe))
        if action is None or action[0] != "both" or action[1].lower() not in indices:
            raise ValueError("native locomotion action is not a captured full-body animation")
        bindings[field] = (identity(action[1]), action[1])
    fire = select_action(script, "fireweapon", context)
    moving_fire = select_action(script, "fireweapon", dict(context, movetype="walk"))
    if fire is None or fire[0] != "torso" or moving_fire is not None:
        raise ValueError("native M4 firing overlay rules differ from supported branch")
    bindings["fire_standing"] = (identity(fire[1]), fire[1])
    enemy = state["training_enemy"]
    enemy.update({field: value[0] for field, value in bindings.items() if field.startswith("walk_")})
    enemy["locomotion"] = {field: value[0] for field, value in bindings.items() if not field.startswith("walk_")}
    enemy["locomotion"]["script_sha256"] = hashlib.sha256(script_bytes).hexdigest()
    return bindings


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-state", type=Path, required=True)
    parser.add_argument("--characters", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    state = json.loads(args.game_state.read_text())
    capture = json.loads(args.characters.read_text())
    arsenal_path = args.game_state.parent / state["arsenal_file"]
    arsenal = json.loads(arsenal_path.read_text())
    client_sha = next(f["sha256"] for f in manifest()["files"] if f["path"].endswith("codoMP_client_shipRetail.exe"))
    bindings = prepare(state, capture, arsenal, client_sha)
    for key in ["arsenal_file", "collision_file"]:
        state[key] = str((args.game_state.parent / state[key]).resolve(strict=True))
    with args.output.open("x") as output:
        json.dump(state, output, indent=2)
        output.write("\n")
    print(json.dumps({"output": str(args.output), "animations": bindings, "read_only": True}))


if __name__ == "__main__":
    main()
